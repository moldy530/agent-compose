# agent-compose — Distributed Execution

**Protocol version:** 2 — carried on every join, and §10 fixes what it pins and
when it bumps; §10.4 lists the bumps
**Status:** Normative for the hub/worker protocol a compiled project speaks. Both
halves are built — the static surface it describes (`hub:` and `placements:`) is
enforced by `validate`, a build emits the hub, and `agent-compose worker` is the
spoke. §12 says what that means, file by file.
**Companion artifacts:** [`docs/durability.md`](durability.md) (the journal this
writes into, and the replay it extends over the wire),
[`docs/grammar.md`](grammar.md) §14.1, §14.2 (the deploy-layer surface), §9.4
(idempotency keys), [`prd.md`](../prd.md) §5.10, §5.12, resolved questions 37–46

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
13. [What this document does not settle](#13-what-this-document-does-not-settle)

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

That clause is about executions sharing with **each other**, and the two
processes *inside* one are a separate question that now has its own answer. A
`store.*` on a process-local backend — `memory`, `sqlite`, `sqlite_vec`,
`local_fs` — is a different physical store on a worker from the one on the hub,
whichever execution opened it, and a placement is several processes by design
(§1.1). So `validate` **refuses** the pairing: grammar §14.1 rule 5 is a compile
error wherever a component that can execute in a placement's process binds such a
store, at every scope and under `--target local` too (PRD resolved q45). A mesh
that shares one store binds a **networked** backend, whose variables §9.1's
partition already carries to every placement that reaches it — and until this
compiler release opens one (production `storage_backends` land behind the store
plugin interface in M3, PRD §7), a mesh whose composition needs a store keeps the
component that binds it on the hub.

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

**A worker polls while it is busy, and holds one dispatch at a time.** Both
halves are normative for v1; they do not have the same authority behind them —
the first is forced by the transport, the second is this document's own choice
and §13 is where it is filed — and the paragraph after them says which is which:

- A worker keeps exactly one poll in flight from the moment it joins until it
  stops. It does **not** suspend polling while a node runs: a four-minute build
  is four minutes of holds that return empty, and the session stays inside the
  liveness window the whole time. Without this clause the window below would
  presume a working worker gone, supersede the dispatch it is in the middle of
  (§6.3), and leave it with a result to post against a session the hub has
  forgotten — an attempt failed, and a build's four minutes thrown away, for a
  worker that was never in trouble.
- The hub MUST NOT answer a session's poll with a dispatch while that session
  has a dispatch it has not settled. A busy worker's polls are therefore
  heartbeats and nothing else, however much work is queued for the placement it
  claims.

**The second clause is a decision this document makes, not one a resolved PRD
entry hands it**, and it is worth marking as such because it is the only rule in
§2 that is not forced by the transport. What the resolutions do fix is where a
placement's parallelism comes from: several workers claiming one name form a
pool the hub dispatches across (PRD resolved q38, §1.1), and the project's
standing answer to "more throughput" is more processes rather than a cleverer
one (resolved q37 says it of hubs). None of them fixes how many dispatches a
single *session* may hold. v1 fixes it at one, because one is what makes
`dispatch_id` idempotency (§3.4), the mid-node disconnect rule (§7.3) and
heartbeat loss (§6.3) statements about a session rather than about a per-worker
scheduler nobody has written. **Raising it is a question for the PRD**
(§13) rather than a hub's configuration knob: it changes what heartbeat loss
costs an execution (§6.3). The wire is shaped so that whatever the answer is, it
arrives additively — a worker declaring a capacity at join is an OPTIONAL
request field whose absence means one, which §10.2 makes a compatible change.

Concurrency in v1 therefore comes from **more sessions, never from more
dispatches on one**: several workers claiming one name form a pool (§1.1), and a
machine that should run two nodes at once runs two workers.

**So `max_concurrency:` bounds admission, and a placed node's parallelism is the
size of its pool.** A `map` declares how many instances the *graph* may have in
flight (grammar §8.6); when the node it dispatches is placed, how many of them
are **running** at once is the number of live sessions claiming that placement,
and never more. A `max_concurrency: 8` over an `agent.signer` placed on `mac`,
with one worker on the Mac, runs one at a time: the map admits eight, the
placement delivers one, and the other seven are queued to the placement — §2
queues work to a placement and never to a worker (§6.3) — with no session free to
hand them to. An implementation MAY hold those seven as placement waits on the
board (§6.1) or in an admission queue of its own; what is normative is that they
are **undispatched**, which is §6.4's first row.

That is the same posture PRD 5.6's placement-synergy bullet takes, read through
the shape that replaced the one it was written against: *map instances are the
natural unit for `runtime: isolated` — one worker per sandbox with zero change to
the logical definition*. The unit of isolation there is a worker too, and "one
worker per sandbox" is "eight at once is eight workers" in the vocabulary of the
address-keyed sketch resolved q38 retired (grammar D128). What the bullet
promises is kept exactly: the *logical definition* does not change — one `map`
runs eight-wide against a pool of eight and one-wide against a pool of one, with
no edit to the graph. What the spec deliberately cannot express is how many
workers show up, because resolved q38 put machines and their addresses outside
the spec on purpose.

Undispatched is a pause rather than a failure, but it is not free of the clock:
a node's `timeout:` chain runs from dispatch (§6.5), so an item that waits out
its timeout behind a busy worker fails on it exactly as one waiting for a
machine that is switched off does. Both halves of that are the author's to size.
Eight signings at once needs eight workers claiming `mac`; a graph that should
not admit more work than the mesh can run writes the `max_concurrency:` those
workers can serve; and a `timeout:` on a placed node is a bound on **queueing
plus execution**, not on execution alone.

What it is not a bound on is *thinking*. A dispatch that settles paused (§3.4)
puts a `human` wait on the hub's board under the dispatching node's own instance
path, and grammar D102's rule — "a budget above a wait does not run while the
wait is open" — reaches it there exactly as it reaches the same node unplaced. So
a placed node given `timeout: 10m` fails if ten minutes of queueing and work go
by, and does not fail because an approver took a day: the clock stops while the
question is open and starts again when it is answered. A composition that wants
the approval itself bounded writes the `human:` block's own `timeout:`, which is
the one line that bounds a wait on either side of the wire.

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

**The worker process is told that variable's name by its invocation**, not by
the artifact — the artifact is what the hub serves *after* a join, so a worker
that had to read the name out of one could not have joined to get it. Nothing on
the wire turns on this: the name is a deployment's fact on both sides, and what
travels is the token.

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
  "protocol": 2,
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
  "protocol": 2,
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
  unchanged worker skip the download. It is the join's **one OPTIONAL field**,
  and it is optional in one direction only: a worker that holds no artifact at
  all — a cold start, the first minute of a new machine's life — **omits the key
  altogether**, and sends neither `null` nor `""` nor a placeholder hash.
  `protocol`, `compiler`, `runtime` and `claims` are REQUIRED of every join, cold
  start included; `env_ok` has the conditional rule below; and §4.1 is why the
  hash is the member of the handshake triple that may be missing.
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
- any other join — a cold start, which omits `artifact_hash` too, or one holding
  a stale hash — SHOULD omit it, because a worker does not guess a manifest it
  has not read; and a hub that is sent one anyway **MUST ignore it** rather than
  refuse the join.

**The second half of that rule is an obligation on the hub, and it is what keeps
the first half satisfiable.** A worker cannot know whether the hash it holds is
still current until it has asked — a redeployment is exactly the case where it is
not — so a worker holding an artifact sends the hash and the report together,
every time. Were the pair refused when the hash turned out to be stale, that
worker would be refused for sending what the first clause requires and refused
again for omitting it, and its only way out would be answering a refused join
with another join, which §5 and §10.1 forbid. Ignoring the report costs nothing:
it is a report about a manifest this hub is not serving, the answer carries the
current artifact, and the worker fetches, materialises and joins again — the
provisioning cycle it was going to enter anyway.

A hub compares an **absent** `artifact_hash` the way it compares a stale one: it
is not the hash being served, so it takes that same branch. Nothing here needs a
rule of its own for the cold start, and an implementation MUST NOT invent one —
"no artifact" and "the wrong artifact" want the same answer, which is the one the
next paragraph gives.

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

**The session this answer issues is issued against the artifact this answer
names**, which is what makes a redeployment reach a worker that has already
joined: §5 fixes the rule — the hub ends those sessions when it rotates, and the
`410` sends the worker back through this route.

**Refusals**, and the shape of each:

| condition | status | body |
|---|---|---|
| the token does not verify | `401` | no detail. A refused credential is told nothing about why |
| `protocol` names a version this hub does not speak | `409` | names both versions, and which end is behind (§10) |
| the compiler version or runtime does not match | `409` | names both sides of whichever half differs (§4.1) |
| a claim names no placement in the active target | `400` | names the claim, and lists the target's placement names |
| `env_ok` is present and a claimed placement's manifest is unsatisfied | `403` | names the **variables**, never their values, never whether the hub holds them |
| `env_ok` is **absent** on a join whose `artifact_hash` is the current one | `400` | names the rule above: there is a manifest this worker could have read, and dispatching to it would skip the check §9.2 exists for |
| the worker's artifact hash is stale, or absent — and an `env_ok` sent beside either | — | not a refusal: the join succeeds, the report is ignored, and the answer carries the current artifact for the worker to fetch (§3.5, §4) — the rule PRD resolved q40's amendment fixes |

The order matters, and it is the order of the rows: a worker that cannot be
authenticated is told nothing, a worker whose wire this hub does not speak is
told that before anything about the deployment, and the placement and manifest
answers — which describe the target — are given only to a worker that has got
that far.

**A refused join is terminal, without exception.** Every row above names a
condition another join would meet identically — a credential that does not
verify, a wire this hub does not speak, a release that does not match, a claim
that names no placement, a manifest that is not satisfied, a report a worker
could have made and did not. So a worker refused at join **stops**: it exits
non-zero, naming the refusal as it was given, and starting it again is an
operator's act — or its supervisor's, whose backoff is that supervisor's
business.

That the list has no exception in it is a property of the table rather than a
hope, and the ignore-rule above is what buys it: the one condition a *second*
join could have cleared — a report sent with a hash that turned out to be
stale — is not a refusal at all. The bounded backoff of §2 covers the
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
    "session_key": "…",
    "item_index": 0,
    "history": [ { "role": "assistant", "text": "…" } ],
    "policy": { "timeoutMs": 30000 },
    "effect_history": [ { "…": "…" } ]
  }
  ```

  `instance_path` is the flattened path grammar §9.4 fixes as the idempotency
  key, so the worker computes the same keys the hub would. `effect_history` is
  the journaled record of effects this node instance already issued, and is what
  a redispatched node replays to the frontier before going live (§7).

  **The four fields between them are what makes a placed node the same node.**
  Each is OPTIONAL — a dispatch that has none of them omits all four — and each
  is a fact the *hub* holds about this node execution that the node would have
  read for itself had it run there. §4.3 fixes what they are for: "a placement
  decides which *process* runs a node rather than which code exists where", and
  §2 restates it as PRD 5.6's promise that "the *logical definition* does not
  change". A payload short of one of them is a node that quietly means something
  else on a worker, which is the one thing this whole document is written against.

  | | what it carries | absent when |
  |---|---|---|
  | `session_key` | grammar §4.1's `execution.session_key`, so a `scope: session` store on the worker addresses the partition the run named | the execution has none |
  | `item_index` | the source-item index of the innermost enclosing `map` (grammar §4.1) | no `map` encloses this node |
  | `history` | the turns of the shared `messages` channel an `agent:` node would have been given (grammar §10.4) | the node takes none — every dispatch that is not an `agent:` node, and a `map`-dispatched agent, which grammar §8.6 rule 10 already runs on a fresh conversation (D105) |
  | `policy` | grammar §9.3's level 1 for this instance — the `policy:` of the `flow:` node that instantiated the enclosing flow, which crosses into a subflow an attached `flow.*` starts (D79) | nothing set one |

  They are **the hub's to derive, and the worker MUST NOT invent them**: a worker
  that defaulted `session_key` to the empty string would fail a `scope: session`
  store with a diagnostic telling the operator to pass a `--session` they already
  passed, and one that defaulted `history` to empty would answer from the input
  object alone with no diagnostic at all.

  Adding them to a `1` that had already shipped without them would be §10.2's
  first compatible change — an OPTIONAL field a peer of the previous version
  ignores — which is why the shape is written this way rather than as a second
  version of the route.

- **`204`** when the hold expired with no work **for this session** — which
  includes every hold while the session has a dispatch it has not settled (§2).
  A busy worker keeps polling and keeps being answered `204`, however deep the
  queue for the placement it claims; that is the mechanism, not a degenerate
  case of it.

- **`410`** when the session is unknown (§3): the worker joins again and resumes
  polling under the session that join returns. A hub that has just been replaced
  behind its name answers every worker this way, once each, and that is the whole
  of what a hub restart costs (§5). A hub that has just **rotated its artifact**
  answers the workers of the superseded one the same way, and for the same
  reason: the re-join is where they are told what to run (§5).

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

Carries the session and the `dispatch_id`. A batch of effect records the worker
produced while executing that dispatch, in the order it produced them:

```json
{
  "dispatch_id": "dsp_…",
  "effects": [ { "key": "…", "…": "…" } ]
}
```

Both fields are REQUIRED, and `dispatch_id` is REQUIRED for a reason that reads
at first like a contradiction of the rule three paragraphs down: the records are
keyed by effect key and scoped to their **execution**, "not by session, and not
by dispatch", so the dispatch is not what *identifies* them — it is what tells
the hub whose execution they are, and inside which instance path. **The hub
reads both off the dispatch row it already holds**, never off the batch, and a
record whose `site` falls outside that dispatch's `instance_path` is refused.
That is §8's single writer stated as a route: no session can write an effect
into an execution it was never dispatched, and no worker names an execution at
all.

**The hub is the single writer, and this route is what preserves that: workers
SEND, the hub INSERTS.** No worker ever touches the journal, which is why SQLite
stays a valid backend for a personal mesh — there is still exactly one writer
(PRD resolved q42).

Each record carries its **effect key**, derived by the execution-derived rule
grammar §9.4 fixes — `<site>#<kind>/<ordinal>`, which the hub re-derives from the
record's own three fields and refuses a record that disagrees with, since the key
is what the row is written under and one naming another node's slot would be
claimed by that node's replay as a divergence. Insertion is **idempotent by that
key**: a record the journal already holds is accepted and dropped. A batch is
therefore safe to re-send after a transport failure, and a worker SHOULD re-send
rather than guess.

| condition | status | body |
|---|---|---|
| the batch is journaled | `204` | empty. Every record in it was inserted or was already held |
| the token does not verify | `401` | no detail, as everywhere (§3.1) |
| the session is unknown | `410` | names the rule of §3: join again, and send this batch again under the new session |
| the body is not a batch — a missing `dispatch_id`, a missing `effects` array, or a record short of a field, naming a `site` outside the dispatch's, or carrying a `key` its own `site`, `kind` and `ordinal` do not derive | `400` | names what a batch and a record carry |
| the `dispatch_id` names no dispatch this hub holds | `409` | names the dispatch. The batch is discarded, as at §3.4 |

The last two rows are not about the batch's *contents* the way the first three
are about its fate, and they are written out because a `204` over a batch nothing
was written for would be the sharpest failure this route has: a worker told its
effects are journaled when they are not hands the redispatch of §7.2 an
`effect_history` short of the frontier, and the node re-issues an effect the
journal was owed.

**A batch answered `410` is re-sent, never dropped.** The records are keyed by
effect key and scoped to their execution (§7.1) — not by session, and not by
dispatch — so the journal takes them from whichever session hands them over, and
the hub is still the single writer that inserts them. A worker that discarded
the batch instead would hand the redispatch of §7.2 an `effect_history` short of
the frontier, and the node would re-issue an effect the journal was owed — so
the model call §7.3 promises is not paid for twice would be paid for twice.

A worker SHOULD send a batch as soon as an effect completes rather than
accumulating until the node ends, because an effect that never reached the hub is
an effect the replay of §7 cannot skip.

### 3.4 `POST /workers/result`

Carries the session and the `dispatch_id`. The node's outcome: its output, its
failure, or **the pause it stopped at**.

**Idempotent by `dispatch_id`.** A second result for a dispatch an earlier one
already **settled** is accepted and dropped, which is what makes at-least-once
dispatch (§7) safe on the return path as well as the outbound one.

#### The third ending: a result may settle a dispatch *paused*

A `human:` node is reachable on a worker — grammar §14.1 rule 4 runs everything
an attached `flow.*` reaches in the attaching agent's placement, and "the signing
machine pauses for approval" is a shape a mesh is *for*, since the machine
holding the capability is exactly where an approval-gated node belongs. The wait
board is not: it is the hub's, which is what keeps wait-board unity (PRD resolved
q43) and what a status report publishes, a resume route answers and a recovery
re-derives.

So a pause is neither held on the worker nor failed. **The result settles the
dispatch, carrying the pause**, and the hub plants the wait on its own board
(PRD resolved q46):

```json
{
  "dispatch_id": "dsp_…",
  "paused": {
    "wait": "escalate/0/escalation/0/ask/0",
    "flow": "flow.escalation",
    "node": "ask",
    "shown": { "…": "…" },
    "paused_at": "2026-08-31T09:14:02.113Z",
    "expires_at": "2026-09-01T09:14:02.113Z",
    "effect": {
      "key": "escalate/0/escalation/0/ask/0#human/0",
      "site": "escalate/0/escalation/0/ask/0",
      "ordinal": 0,
      "request": "{\"node\":\"ask\",\"shown\":{…},\"wait\":\"escalate/0/escalation/0/ask/0\"}"
    }
  }
}
```

`paused` is present exactly when the dispatch ended at a pause, and a result
carries at most one of `output`, `error` and `paused` — a body carrying two
endings is refused the way an unreadable pause is (below), because which one the
sender meant is not something a hub can decide, and reading the pause and
dropping the output would ask a person a question the node had already answered.
Every member of it is REQUIRED except `expires_at`, which is present exactly when
the node declares `timeout:` (grammar §8.7).

Every field is a fact **the worker derived and the hub cannot**, and nothing more
travels than that. `wait` is the node's deterministic wait identity — its
instance path plus an ordinal, grammar §9.4 — so the hub plants the wait a local
run would have opened at that site, and a resume prepared against one generation
finds it in the next (§6.1's property, for the same reason). `shown` is the
node's evaluated `input:`, which is what the person is asked.

**The two instants are the worker's record of its own clock, and neither is the
wait's.** `paused_at` is when that machine reached the node; `expires_at` is what
its own trace entry recorded for the pause (`docs/trace.md` §3.4) — `paused_at`
plus the node's `timeout:`, on the same clock. They travel because the pause
happened on another machine and the settled dispatch row is the only record of
what that machine did (`docs/durability.md` §3.8), and they are **never armed and
never republished**. Two machines' clocks disagree, so a hub that armed the wait
at `expires_at − now` would give a `timeout: 5m` node no time at all on a worker
ten minutes behind it and a quarter of an hour on one ten minutes ahead, while
the same node unplaced always gets five minutes; and a hub that published
`paused_at` beside a deadline of its own would publish a pair read off two clocks,
whose difference is the machines' offset rather than the node's `timeout:` —
negative once a worker's lead exceeds the budget — where the same node unplaced
dates both members off one reading.

**So the hub dates the wait where it plants it**, off one `Date.now()` reading,
and arms the node's own `timeout:` out of its copy of the descriptor from that
same instant — the only instant a local wait's budget is ever spent from either.
Both members of the pair it publishes are that reading's: `expires_at` is the
deadline it will fire, on the clock it published it from, and `paused_at` is when
the generation holding the question began holding it. A `timeout: 5m` node is
therefore shown as asked now and expiring in five minutes whichever machine
reached it, the five minutes a reader is shown are the five minutes the resume
surface will take an answer through, and `expires_at − paused_at` is the node's
`timeout:` on either side of the wire. That is PRD resolved q46's parity bar for
status visibility, held rather than documented around; publishing the wire's
instants instead would break it twice over — a question shown as expired for the
whole time it is answerable behind a slow worker's clock, which is a status route
contradicting the resume route, and a journaled record whose answer arrives
before its question, which is the entry `docs/durability.md` §3.4 refuses by
name. The journal keeps the pair the board published, so the answered pause's own
trace entry (`docs/trace.md` §3.4) is the entry an unplaced pause writes.

So neither instant's **value** is one a hub routes off, and the two are checked
differently for that reason. `expires_at` is the one member of a `paused` body
**no hub check backs**, and deliberately: the checks below refuse a pause over a
field the hub would otherwise have *used*, and its rule — present exactly when
the node declares `timeout:` — is a producer obligation, kept by a worker
because the settled row is where it is read back. Refusing a pause over it would
cost a node its attempt for a disagreement about a row nothing routes off, and a
member whose absence is itself meaningful is one a hub cannot tell a mistake
from. `paused_at` is REQUIRED and is checked as every other required member is —
a body short of it is unreadable, by the rule below — because it is the one fact
the settled dispatch row exists to carry about the machine that reached the
node, and a row short of it is not that record. What the check does **not** do is
spend the value: the hub dates the wait it plants off its own clock, and files
the worker's reading beside the rest of the row for a reader asking when *that*
machine got there.

**A restart re-arms it whole**, because a restarted hub re-derives a placed pause
by *planting it again*: it dates the wait its own now and gives the question the
node's whole `timeout:` in front of it, with a published deadline that says so.
That is what the same node unplaced does, for the same reason it does it: an
unanswered local pause journals nothing (resolved q28), so a resumed generation
re-parks it from scratch, re-dated, and its budget starts again
(`docs/durability.md` §5). Five minutes after a restart, a `timeout: 5m` pause
has the same time left and reads the same on either side of the wire, and "how
long do I have" is not a question a deploy file gets to answer. The hub *knows*
when the worker took the pause — the settled row is dated — and does not spend
that knowledge on the budget or on the date it shows; a wait no process was
holding is a wait nobody could have answered, and charging it to the person would
be charging them for the downtime.

**The `human:` block's `timeout:` is the only clock the question is under.** The
*dispatching* node's `timeout:` — a bound on queueing plus execution (§2, §6.5) —
is held still while the wait is open, because the wait is planted under that
node's instance path and grammar D102 says a budget above a wait does not run
while one is open. A placed node's budget therefore bounds what the instance
*does*, on either side of the wire, and never what a person takes to answer.

`effect` is the
journal record the **answer** will be written under, exactly as the worker's own
recorder claimed it (`docs/durability.md` §4) — the key the redispatched node
will look up, and the canonical `request` it will compare against.

**Both identities are held to the dispatch's own `instance_path`**, at or inside
it, exactly as §3.3 holds a record's `site`. It is §8's single writer stated for
the route that also plants a wait: no session may journal an effect into a node
it was never dispatched, and none may put a question on the board under another
node's identity — which the resume surface would then answer. The `key` is held
with them, by the derivation rather than by the prefix: it MUST be
`<site>#human/<ordinal>` for the `site` and `ordinal` beside it
(`docs/durability.md` §4), because the key is the field the record is written
under and one naming another node's slot would be claimed by *that* node's replay
as a divergence. `flow` and `node` MUST name a `human:` node the hub's own
artifact declares, since the answer is held to that node's `output:` and a
question nothing can validate an answer against is one no surface may take.

**And `ordinal` MUST be the number of `human` records this execution's journal
already holds at `effect.site`** — counted at the site itself, since an ordinal
is per site and not per subtree. That is the ordinal the *next* `human` claim at
that site takes (`docs/durability.md` §4), and the redispatched node's own claim
is what reads the answer back: a record written at any other ordinal is one no
claim ever reaches, so the person is asked a second time — the one failure this
whole ending exists to remove. It is stated here rather than left to the key
check, which derives faithfully from whatever ordinal travelled beside it and so
cannot see this. A worker of this release always sends exactly this number,
because the pause it settles on is its own first live claim after replaying the
`effect_history` the dispatch carried (§7.2), and the two derivations — the
worker's claims-in-session and the hub's records-in-journal — count the same
records.

**And all of them MUST name one node**, which is the same rule along the other
axis. `wait` and `effect.site` MUST be the **same path**: a pause's identity *is*
its effect site, derived once and sent twice, so a body carrying two paths would
plant the question under one node's identity and journal the answer into
another's slot — a correct key for the wrong node, which is precisely what the
derivation check above cannot see. That path's last frame MUST name `node`
(grammar §9.4's `<node id>/<ordinal>`), because the contract the answer is held
to is `flow`.`node`'s: a pause whose identity reached a different node would arm
that other node's `timeout:`, publish its `output:` and parse the answer with its
parser. What a hub cannot check is which *flow* the path ran in — a frame above
the node is a node id or a flow's local name, and nothing resolves one back to a
declaration — so two `human:` nodes sharing an id in two flows are
indistinguishable here, and a peer that crossed them is caught by the
redispatch's own parse (resolved q29) rather than by this route. A
`paused` that breaks any of those, or that is short of a member, is **not**
answered with a status:
this route's table is closed (§10.1) and a status outside it is a refusal a
worker stops for, so the hub settles the dispatch as a *failure* naming what was
wrong, and the node's own `retry:`/`on_error:` chain runs over it. The placement
keeps its worker; the attempt is what an unreadable pause costs.

What does **not** travel is the contract: the answer's schema and the parser that
holds an answer to it are the hub's own, read out of the artifact it is already
serving. That is §4.3 being load-bearing again — the whole artifact is
everywhere, so the hub holds the very descriptor the worker paused on — and it is
what makes the parity PRD resolved q46 requires a lookup rather than a second
contract on the wire.

**A paused result is SETTLED**, which is why it needs none of the machinery
below rewritten: the dispatch ended with a result, so idempotency by
`dispatch_id` is unchanged, a re-posted paused result is `204`, and a paused
result for a dispatch the hub superseded is `409`. The **session is free** the
moment it posts one, and may be dispatched other work while a person thinks — so
a worker gone between the pause and the answer costs the execution nothing while
the question is open, and costs it only the wait for a replacement once the
answer arrives, which is the dispatching node's own budget and is below.

What the hub does with it is the ordinary discipline and no new mechanism: it
plants the wait on the existing board under the identity above (a third kind of
wait beside a `human` pause and a placement wait — the *same* kind as the first,
in fact, reached over the wire), fires the lifecycle webhook a local pause fires,
and publishes the same status a local pause publishes. The existing resume
surface answers it. The answer is journaled as the pause's own effect record, at
`effect.key`, and the node then **re-enters dispatch**: a fresh row at the next
ordinal of that instance path, queued to its placement and parked if nothing
claims it (resolved q39's machinery), whose `effect_history` carries the answered
pause — so the replay of §7.2 consumes it at the very claim that paused and the
node goes live *past* the question. A model call made before the pause is
replayed, not re-issued, which is §7.3's promise reaching the one case that used
to be outside it.

A wait whose budget runs out is the same path with the other settlement: the
expiry is journaled as the pause's record, the node is redispatched, and the
replay raises the node's own `on_timeout:` route (grammar §8.7) — decided by the
same line of the same function an unplaced pause's expiry is decided by.

**The one interval a mesh adds is the redispatch's own queueing**, and it is
inside the dispatching node's budget rather than outside it. The wait held that
budget still (above); settling it — with an answer or with an expiry — starts it
running again, and what runs next is a fresh row waiting for a session to claim
it, which §6.5 makes part of what the node's `timeout:` bounds. So a node whose
own budget runs out before any worker takes the redispatch fails on that budget,
and neither the person's answer nor the pause's `on_timeout:` route is reached —
where the same node unplaced would have taken the route in the same process, in
the same instant the wait expired. That is the one place a placed pause is not
indistinguishable from a local one, and it is the placed shape of "fail if the
machine is not up in ten minutes" rather than a second rule: the budget the
author wrote on the *dispatching* node is the one that bounds waiting for a
machine, on this side of a wait exactly as on the other.

**A hub restart between the pause and the answer costs nothing**, because the
pause is journaled: the settled row carries it, and the replay that re-reaches
the node re-derives the wait onto the new process's board — under the identity
its predecessor published, dated by this planting, and with the node's whole
`timeout:` in front of it and a deadline that says so, exactly as a resumed
generation re-parks a local wait nobody answered (above) — unless the answer's
record is already in the journal, in which case the wait is over and the
redispatch is what replays past it.

This is a change an older peer would misread — a `1` peer sees a result with no
`output` and reads it as a node that answered nothing — so `PROTOCOL_VERSION` is
**2** (§10).

#### The two verbs

**A dispatch ends in one of two states, and this document gives them two verbs**
— they are not synonyms and the table below turns on the difference. A dispatch
is **settled** when a result posted for it is what ended it, and **superseded**
when the hub ended it without one, which §6.3's declaration is the only way of
doing. A dispatch in neither state is **unsettled**: still in flight, and the one
piece of work a session may be holding (§2). So a re-posted result never matches
two rows: `204` says the hub holds this dispatch's own result, `409` says the hub
ended this dispatch without one and the execution has moved past it.

A result the hub cannot attribute — an unknown or superseded `dispatch_id` — is
answered `409` and the worker discards it: the execution has moved on, and
re-driving it from a stale result is exactly the divergence
`docs/durability.md` §7 refuses. The only way a `dispatch_id` becomes superseded
is §6.3: the hub gave up on the session this dispatch was issued to, and the node
has been through its `retry:` chain since.

A body that names **no** `dispatch_id` is the same row and takes the same `409`,
with a `null` where the dispatch would be named. This route's table is four
statuses and §10.1 lets an implementation rely on them, so a result that cannot
be attributed at all is not answered outside it — unlike §3.3, where a batch that
is not a batch has a `400` row of its own, because that route also refuses
*contents* and this one has none to refuse.

| condition | status | body |
|---|---|---|
| this result settles the dispatch, or a result already settled it | `204` | empty |
| the token does not verify | `401` | no detail, as everywhere (§3.1) |
| the session is unknown | `410` | names the rule of §3: join again, and post this result again under the new session |
| the `dispatch_id` is unknown, missing, or the hub superseded the dispatch (§6.3) | `409` | names the dispatch, or `null` where the body named none. The result is discarded |

**`410` and `409` are different failures and a worker MUST NOT treat them
alike.** `410` says *the hub does not know you*, and the result is still owed:
join, and post it again under the new session — the hub attributes it by
`dispatch_id`, which the new session does not change. `409` says *the hub knows
you and does not want this*, and the result is dead. A worker whose re-posted
result meets `409` has its answer and stops re-posting; a worker that read the
two as one would either drop a result the hub was waiting for or re-drive an
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

A hub MUST keep serving an artifact **it holds** while any execution that was
dispatched under it is unfinished, and MAY drop it afterwards. The qualifier is
load-bearing and is what the second table row is written against: a hub that
holds two artifacts may not drop the older one out from under an execution
still running on it, because a worker mid-transfer would then have a `404` and
nothing to fetch. It does **not** oblige a hub to hold two. A hub that serves
exactly one — which is what a v1 build is, and §12 names that limit — meets this
rule for the artifact it has and answers `404` for the one it replaced, which is
not a stranded worker: the `404` sends it back to a join, the join is answered
with the current artifact, and the fetch it then makes is one this hub can serve.
That is §5's redeployment rule arriving by the other door.

A worker meeting `404` for
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
rather than a drift (PRD resolved q40). What makes the *next join* happen for a
worker already joined is §5's rule that a redeployment ends the sessions issued
under the artifact it replaced; without that half, "on the next join" would be a
promise the wire never keeps.

The artifact is a **tarball of the generated project** — the same tree
`build` writes — served **hash-addressed** from the hub under worker
authentication, on the route §3.5 fixes. Its hash is over the tree's content, so
two hubs built from one composition serve one artifact.

**The tree is what `build` wrote plus the authored files the composition
references** (PRD resolved q49). A `tool.*` may bind a hand-written TypeScript
file inside the project (`docs/grammar.md` §6.1), and such a tool executes
wherever its tool executes — on a worker, when placed or reached through
attachment — so the file has to be in the tarball or the node has nothing to run.
It is: `ARTIFACT_FILES` lists it and `ARTIFACT_HASH` covers it like every other
entry, at the same relative path it is edited at, which means **editing a tool
implementation is a new artifact** and reaches every worker through this
handshake with no new machinery. A file under `src/` the composition does not
reference is not part of the tree and ships nowhere. `manifest.json` names the
carried ones under `authored`, so a reader of an unpacked tree can tell the
compiler's files from an author's without a JavaScript runtime.

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

**Step 4 needs no configuration of its own, and that is the point of
`package_registry:`.** On a network that mandates an internal npm mirror, a
`bun install` against the public registry is an install that cannot complete —
and nothing a worker holds but the artifact could tell it otherwise. So a target
that declares a `package_registry:` (`docs/grammar.md` §14.6, PRD resolved q59)
has `build` write a `bunfig.toml` and an `.npmrc` into the tree, as ordinary
members of `ARTIFACT_FILES` covered by `ARTIFACT_HASH`: they arrive in the
tarball step 2 fetched, are unpacked by step 3 into the directory step 4 runs in,
and the installer reads them because they sit beside `package.json`. **Nothing in
this document changes** — no route, no field, no step. A credential in them is
the *name* of an environment variable, expanded by the installer, so the tarball
carries no secret and a rotated token is the same artifact under the same hash;
the variable itself is on the worker's own manifest, which is §9.1's business.

### 4.1 The handshake triple

A join agrees on three values, and all three are on the wire, in the join §3.1
fixes. Two of them are compared on every join; the third is compared whenever the
worker has one to send, and the paragraphs after the table are that asymmetry:

| | what it pins | where it is written |
|---|---|---|
| **artifact hash** | the generated project, exactly | `artifact_hash` in the request, **when the worker holds one**; `artifact.hash` in the answer |
| **compiler version** | the `agent-compose` release that generated it | `compiler` in the request, and in the answer |
| **runtime** | Bun, and its major version | `runtime` in the request, as `"<name> <version>"` |

A field the request omits is a comparison the hub cannot make, which is why
**`compiler` and `runtime` are REQUIRED on every join** rather than helpful: PRD
resolved q40 makes the refusal the point of the handshake, and a refusal that
names both sides is only writable from a request that carries one of them.

**The artifact hash is the one member a join may leave out**, and leaving it out
states something rather than omitting it: this worker holds no artifact. A cold
start has no hash to send, so it omits the key — §3.1 fixes that as an absent
key and not a `null` — and a hub reads an absent hash exactly as it reads a stale
one, because both mean "not the artifact I serve" and both are answered the same
way. That takes nothing from the handshake, because of the asymmetry the next
paragraphs turn on: the hash is the member the hub can **repair**, and the two it
can only refuse on are the two required of every join, cold start included. So a
provisioning join is still a join the version check runs on, and a worker of the
wrong release is refused before it downloads anything.

**A hash mismatch is not a refusal** — the answer carries the current artifact
and the worker fetches it — and an **absent** hash is that same case at its
limit, refused no more than the mismatch is, because a hub that answered a first
join `400` for naming no artifact would make a fresh machine unable to bootstrap
into the mesh at all, since fetching (§3.5) is downstream of the join that names
what to fetch.

This is the rule PRD resolved q40 fixes, by its amendment (2026-08-30): the
mismatch clause reads on what the worker **runs**, not what it **holds** — a
compiler-version or runtime mismatch is a refused join naming both sides, and a
hash mismatch is repaired by the join's answer. The amendment exists because the
entry's original clauses collided — a refused join on a hash mismatch cannot
coexist with "redeployment is automatic on the next join" — and this document's
earlier revisions carried the collision as a gate (§13) rather than deciding a
PRD question downstream; the amendment is that gate closing.

Why the hash is the repairable member: it is the single member of the triple the
hub can **fix in the answer it is already sending**, so fixing it is what
"automatic" means. Refusing it would also strand §3.1's provisioning join, which
is a join made with no artifact at all: no fresh machine could bootstrap into a
mesh, because fetching (§3.5) is downstream of the join that names what to
fetch. The two members the hub cannot fix — the release a worker binary was
built from, the runtime installed on its machine — are the refusals.

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

**A redeployment ends the sessions issued under the artifact it replaces.** A
session is issued against the artifact hash the join answered with, and a hub
that begins serving a new artifact under a stable name MUST forget every session
issued under the one it replaced. That ending is the ending of §6.3 and costs the
same: the next request on such a session meets `410`, the worker joins again, and
*that* join is answered with the new artifact — which it fetches, materialises,
and re-joins under, by §4's five steps. An unsettled dispatch on an ended session
is superseded exactly as §6.3 supersedes one, so its node's attempt fails under
that node's `retry:`/`on_error:` chain and the retry is dispatched under the new
artifact rather than the old.

This is the mechanism behind §4's "redeployment is automatic on the next join",
and without it there would be no next join to be automatic on: a worker that has
joined re-joins when a `410` tells it to, or when a fetch meets §3.5's `404`, and
nothing else in this document ever tells it to. It also gives the poll the
invariant it is read against — **a hub MUST NOT dispatch to a session issued
under an artifact it no longer serves** — which is why the dispatch of §3.2
carries no hash of its own: a session that can receive a dispatch is, by
construction, a session of the current artifact, and there is nothing left for a
worker to re-check. A hub that kept those sessions
alive instead would hand a worker holding the old tree a dispatch planned against
the new one — an `instance_path` and inputs from one graph, executed by another,
with the effect keys of §3.3 derived from the wrong one. That silent skew is what
resolved q40's "version skew is a refusal rather than a drift" forbids, and this
rule is what makes it a `410` instead.

A hub MAY end those sessions the moment it rotates and MUST have ended them
before it dispatches anything of the new artifact. Going on **serving** the
previous artifact at §3.5 is not in tension with that, and the two together are
deliberate: a hash stays fetchable after the sessions issued under it are gone,
which is what lets the worker of a superseded dispatch come back rather than be
stranded mid-rollback.

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
(§5). Declaring it **ends** the session — the hub forgets it, so any later
request carrying it meets `410` and the worker joins again (§3, §5).

Liveness is the only thing this protocol *detects*, and it is not the only thing
that ends a session: a redeployment ends the sessions of the artifact it replaced
(§5), which says nothing about the worker and is not a declaration about it. The
two share their whole cost model — the `410`, the re-join, and the treatment of
an unsettled dispatch below — which is why §5 states that rule by pointing here
rather than by writing a second one.

What the declaration costs depends on the one thing a session can be holding.
§2's pull model queues work to a **placement**, never to a worker, and hands it
over one dispatch at a time, so a session holds at most one piece of work: the
dispatch it has not settled. Two cases, and they are the whole rule.

- **The session had no unsettled dispatch.** Nothing re-parks, because nothing
  was ever this worker's. The placement's open placement-waits stay on the board
  untouched, and the next join claiming that placement takes them in park order
  (§6.2). This is the sleeping laptop, and it costs the execution nothing.
- **The session had an unsettled dispatch.** The hub **supersedes** it — §3.4's
  second terminal state, the one no result ended, and never the settled one — and
  the node's attempt fails with it, under that node's `retry:`/`on_error:` chain
  exactly as §7.3 says — the liveness window is what *detects* the mid-node
  disconnect §7.3 describes, and this paragraph is where that detection is
  filed. Effects the worker had already streamed home
  stay in the journal and are handed to the next attempt as its `effect_history`
  (§7.2). Where `retry:` grants that attempt, it re-enters dispatch and **parks
  on the board if no worker is claiming the placement** (§6.2) — which is the
  sense in which heartbeat loss re-parks.

A revived worker meets both halves of §3 in order, and the order is the point.
Its first request carries a session the hub has forgotten, so it is answered
`410` and the worker joins again. The result it then posts for that dispatch is
answered `409` and discarded (§3.4) — the superseded row, because this
declaration is what superseded it. It is never the `204` row: that one is a
dispatch a **result** ended, and this is one the hub ended without one, which is
the whole of why §3.4 gives a dispatch's two end states two verbs — *settled* and
*superseded*. The third ending a result may carry cuts across that axis rather
than adding to it: a paused result settles its dispatch exactly as an answer
does, so a pause taken here would still be the `204` row and never this one. The
effect batches the worker still holds are a different matter and **are**
journaled: they are keyed by effect key and scoped to their execution, not to a
session or a dispatch (§3.3), so a batch in flight when the lid closed reaches
the journal on the re-send and the retry replays it instead of re-issuing it
(§7.2).

PRD resolved q39 puts this as "heartbeat loss re-parks what was queued to the
vanished worker". The phrase is written in the vocabulary of a push model, where
a worker holds a queue of its own; this protocol pulls, and §2 queues work to a
**placement** rather than to a worker, so what the clause names is the one thing
a session can be holding. At v1's capacity of one that is the unsettled
dispatch, and it does re-park — by the second case above: the attempt fails, and
where `retry:` grants another the node re-enters dispatch and parks on the board
if nothing is claiming the placement. What the re-park costs is an attempt, and
that price is the one resolved q39 and q42 set together when they made mid-node a
failure rather than a pause (§6.4).

**This is the rule §2's open capacity question would move.** A session allowed to
hold more than one dispatch makes "what was queued" a plural, and the choice
between superseding all of it and handing back the part no effect record vouches
for is a choice about what a closed laptop costs an execution — which is why §2
sends the capacity to the PRD (§13) instead of leaving it to a hub's
configuration.

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

The first row is not only the machine that is switched off. A node whose
placement has live workers and no **free** one — the seventh item of a
`max_concurrency: 8` fan-out onto a one-worker pool (§2) — is undispatched too,
and is governed identically: it costs nothing while it waits, and its `timeout:`
is running the whole time.

### 6.5 Bounding needs no new grammar

"Fail if the machine is not up in ten minutes" is already spellable: the node's
`timeout:`/`on_error:` chain applies from dispatch (grammar §9). No key is added
for placements.

The one interval that chain does not count is a pause the dispatch settled on
(§3.4): the wait is planted under the dispatching node's instance path, so
grammar D102 holds the node's budget still while the question is open, exactly as
it does for an unplaced node with a wait inside it (§2). "Fail if nobody approves
in ten minutes" is therefore the `human:` block's own `timeout:`, not the placed
node's — the same line it is without a mesh.

**The budget starts running again the moment the wait settles**, so the
redispatch that carries the answer — or the expiry — queues inside what is left
of it, exactly as the first dispatch did. A node that has spent its `timeout:`
waiting for a machine fails on it, and the answer or the `on_timeout:` route
behind that redispatch is never reached (§3.4). Both halves are this section's
one rule read at the two ends of a wait: thinking is outside the budget, and
waiting for a machine is inside it.

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
that stopped making requests while holding a dispatch, which the hub supersedes
(§3.4), which fails the attempt. A worker that comes back afterwards learns so
from the `409` its re-posted result meets (§3.4, §6.3), and takes part in the
retry only by joining like anyone else.

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

What that third rule buys the journal is a ledger of its own, and it is the
journal's document that defines it: `docs/durability.md` §3.8 is where a dispatch
row's fields, its four-value `status`, and its relationship to the effect
frontier are normative. A reader who wants to know what a hub restart re-derives
from reads that section; this one says what the wire does with it.

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
  hub** — the default of §1.1, and unconditional: what *else* reaches that
  component narrows nothing, because dispatch is the hub's scheduler starting a
  node and every flow a composition declares is startable on the hub (grammar
  Decision D64);
- **an agent that runs in that process attaches it** — an attached `tool.*`, and
  every `agent.*` and `tool.*` an attached `flow.*` reaches, all of which run
  inside the agent's tool loop (grammar §5.4, §14.1 rule 4).

Which gives, concretely:

- a variable referenced by a member of placement `P` belongs to `P`'s manifest;
- a variable referenced by a tool that only ever runs inside placed agents'
  processes belongs to **their placements'** manifests — and to the hub's only
  if some unplaced agent or `function:` node also reaches it;
- a variable referenced by an **unplaced `agent.*`** belongs to the **hub's**
  manifest, and belongs there whether or not something placed also reaches it.
  Manual invocation is universal — every flow a composition declares is runnable
  on its own (grammar Decision D64, which §14.1 rule 4 rests on too) — and every
  construct that reaches an agent is a node of some flow, so an unplaced agent
  always has a hub dispatch to be. Being reached from a placed agent's tool loop
  as well **adds** that agent's placement to its variables' membership; it never
  moves them off the hub's. An unplaced `tool.*` a `function:` node names is the
  same case for the same reason;
- a variable referenced from the deploy layer itself — a storage backend, an
  event source, `hub.join_token:`, a **journal** — belongs to the hub's. The
  journal is the sharpest case and the one worth naming, because the plausible
  reading is the wrong one: a target that binds `journal: { provider: postgres |
  mysql, url: ${…} }` (`docs/grammar.md` §14.7, PRD resolved q62) journals the
  effects of **placed** nodes too, so a reader could expect the worker to need
  that address. It does not. The hub is the single writer (§8 rule 3, PRD
  resolved q42): a worker streams its effect records home over this wire and the
  hub writes the rows, so no worker ever opens that connection and no
  placement's manifest carries what would. Two entries of that
  layer a placement can **also** reach are the exceptions, and each adds
  placements without moving anything off the hub's list. The first is a
  **package registry** (`docs/grammar.md` §14.6, PRD resolved q59), and it is the
  widest membership anything here has: its credential belongs to the hub's
  manifest **and to every placement's**, because every one of those processes
  runs an install — the hub installs the project it serves, and a worker runs
  `bun install` over each artifact it materialises (§4 step 4). It is the one
  variable on a manifest that no *running* process spends: what reads it is the
  installer, over the emitted `bunfig.toml`/`.npmrc`, before any of this
  project's code exists. It is on the manifest all the same, and deliberately —
  neither installer treats an unset reference as an error, it goes out as text
  and the mirror answers `401`, so §9.2's join-time check is what turns that into
  a named variable at the moment a machine joins rather than at its first
  dispatch. The second is a **storage backend**, and it reads like a
  conflict with the executes-in rule above until the two are read the way the
  unplaced-agent clause is read: the credential that opens a store is spent in
  whichever process opens it, so a backend's variables belong to the hub's
  manifest **and** to the manifest of every placement whose components reach a
  store bound to it — an agent's `stores:`, and the `store:` nodes of a `flow.*`
  it attaches. That **adds** placements; it never moves a deploy-layer variable
  off the hub's list. Without the addition a worker joins clean — §3.1's `403`
  cannot fire over a name the manifest does not carry — and fails at its first
  store op on a machine with no credential, which is the failure §9.2's check
  exists to catch;
- a variable reachable in two processes belongs to both. Two placed agents
  attaching one unplaced tool is the ordinary case, and the tool's secrets go to
  both placements;
- a variable a **`coder:` node** declares — every reference inside its
  `workspace:` expression, and every value of its `env:` (grammar §8.9, PRD
  resolved q57, q61) — belongs to the manifest of every
  process that runs the node's **flow**. That is the hub's, always and
  unconditionally, because every flow a composition declares is startable on the
  hub (grammar Decision D64); and it is **also** the manifest of every placement
  whose components reach that flow — a placed agent attaching it as a tool, or a
  placed component naming it — because §14.1 rule 4 starts the attached flow's
  instance inside that agent's own loop, on the worker. A coder node is
  *declared* in a flow rather than defined, so §14.1's `members:` has nothing to
  claim and the node is never placed **on its own**; what carries it to a worker
  is the flow around it, exactly as the `store:` nodes of an attached flow are
  carried. Its references are filed under the node's own address inside that
  flow rather than under a definition's for the same reason. The addition works
  the way the storage-backend clause above works: it **adds** placements and
  never moves the variables off the hub's list. Its `model:`'s **provider**
  connection travels with it, and this is the half that reads like an exception
  and is not: PRD resolved q58 made a provider's `base_url:`, credential and
  `headers:` cross into a harness run, mapped into the harness's own connection
  surface (`docs/grammar.md` §8.9, Decision D143), so those variables are spent
  in whichever process runs the node exactly as an agent's `model:` spends them
  wherever the agent runs. A process that runs only coder nodes therefore needs
  the variables those nodes declare **and** the connection behind each one's
  model. The walk follows it the way it follows an agent's: a `coder:` node's
  `model:` is a reference site, and the provider it names joins the executes-in
  closure of the flow holding the node. Should a later release make a coder node
  placeable in its own right, this bullet is where the rule moves, and the walk
  needs no other edit — a coder node's references enter the closure exactly as a
  `builtin:` tool's do.

**One of a tool's `${ENV}` references is declared rather than walked, and the
closure does not care.** A `module:` binding names hand-written TypeScript
(`docs/grammar.md` §6.1), and a variable read inside it is not something the
walk above can see — so the binding declares its environment in the YAML, in the
same shape an `exec:` block's `env:` takes, and those names enter this closure at
the tool's own address like every other tool surface's (PRD resolved q49). The
declaration is also the implementation's only way to the values, which reach it
as a typed argument rather than through `process.env` — so a variable this
closure did not carry to a process is one that tool's code cannot read there,
rather than one it silently reads from whatever else that machine was started
with. What follows is the property that matters and it is unchanged: a module
tool's variables reach exactly the processes that can execute it. Everything else in
this section — the executes-in rule, the two worked cases, §9.2's `env_ok` — is
written over the closure and not over how a name got into it.

**This closure has a second reader, and it is one walk rather than two.** Grammar
§14.1 rule 5 refuses a store on a process-local backend wherever a component that
can execute in a placement's process binds it (PRD resolved q45), and "can
execute in a placement's process" is the question this section answers — so
`validate` asks *this* partition rather than deriving the set again. Two
derivations of one closure agree on the day they are written; that is the whole
reason the partition is computed once and shipped in the artifact, and a static
rule reading a second copy would reintroduce exactly the drift §9.1 exists to
prevent. The two answers it needs are the set above and the **route** into it:
which `members:` entry the process came from, and which attachment carried it to
the binding — which is what lets the refusal point at lines rather than at a
verdict.

Worked, because this is the case the rule exists for: `tool.sign` carries
`KEYCHAIN_PASSWORD` in its `exec.env`, `agent.signer` attaches it, and the deploy
file places `agent.signer` in `mac` and nothing else. `validate` accepts that —
§14.1 rule 4's first row, the tool claims nothing and runs where the agent runs.
`KEYCHAIN_PASSWORD` belongs to **`mac`'s** manifest and **not** to the hub's: the
hub never runs `tool.sign`, so requiring the secret there would be a false
requirement, and omitting it from `mac`'s would let a machine without a keychain
join clean (§9.2) and fail on its first dispatch.

Worked from the other side, because the symmetric mistake is the expensive one:
`agent.outer` is placed in `mac` and attaches `flow.review`, whose one `agent:`
node names `agent.inner`, which is unplaced and reads `${INNER_KEY}`. Every call
*through `agent.outer`* runs `agent.inner` on the `mac` worker, so `INNER_KEY`
belongs to `mac`'s manifest — and it belongs to the **hub's** as well, because
`agent-compose run main.yml flow.review` starts that flow on the hub and the hub
dispatches `agent.inner` itself. A partition that read "reached by something
placed" as "therefore not the hub's" would emit a hub manifest without
`INNER_KEY`: `readEnvironment()` passes at start, and the first direct run of
`flow.review` fails at the first model call — the failure this section exists to
make impossible, arrived at from the opposite direction to the one above. The
rule that prevents it is the second clause of the executes-in list — dispatched
with no placement of its own, on the hub — and that clause is unconditional for
exactly this reason. `validate` accepts this composition, which is the other half
of why the manifest has to: nothing refuses it, so nothing else stands between
`INNER_KEY` and the hub's list.

This is §7 M3's least-privilege line in its sharpest form: blast-radius
containment falls out of the manifest, because **the hub cannot leak what it
never held** — which is a claim about what the hub *runs*, and is only true if
the partition is computed that way. The two worked cases are the two directions
of that one sentence: the hub's list is narrowed by what the hub **cannot run**,
never by what some placement also happens to reach.

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
* that a session is bound to the artifact it was issued under (§5): a hub that
  rotates its artifact ends those sessions, so every dispatch a worker is
  answered is a dispatch of the artifact it is holding, and a worker need not
  re-check one;
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
  death. §2's other clause is the opposite case and belongs to §10.2: a hub that
  learned to hold more than one dispatch on a session still answers one at a time
  to a worker that declares no capacity, so nothing an older peer does becomes
  wrong;
* changing which side writes the journal (§3.3), or where an idempotency key
  comes from (§3.3, §3.4);
* changing what a status code means at a route — a `410` that stopped meaning
  "re-join" is the sharpest case, because §5 tells workers to act on it, and a
  join refusal that started meaning it is the same case from the other side;
* shortening the **liveness window** of §2. Lengthening it is compatible;
  shortening it is not, because the first thing an older worker learns about the
  new one is a `409` on the result of a dispatch superseded underneath it (§6.3).

A bump is a statement that workers of the older release cannot join this hub,
and §3.1's `409` is what enforces it. Because a worker and a hub are built from
one compiler release, the ordinary upgrade path never meets this: the triple
refuses the mixed pair first, and `PROTOCOL_VERSION` is what remains for the
case the triple stops covering — a worker binary somebody upgrades separately,
or a second implementation of this document.

### 10.4 The bumps

| version | what changed, and why it is a bump |
|---|---|
| **1** | this document, as the runtime that landed with it speaks it |
| **2** | §3.4's **paused** result (PRD resolved q46, 2026-08-31). A result may settle a dispatch with the wait a `human:` node opened, and a peer of `1` would read one as a node that answered with no output — a node that "quietly means something else", which is §10.3's first row exactly, and its third: `paused` changes what a result *is*. The same release also **enforces** §3.3's effect-key derivation at `/workers/effects`: a record carrying a `key` its own `site`, `kind` and `ordinal` do not derive is `400` where it was journaled before. That is not what the bump is for and needs none of its own — the derivation is the rule §3.3 and grammar §9.4 already stated, so a conforming `1` peer sends what it always sent, and one that spelled a key its own fields do not derive is now *refused* rather than left to write into another node's slot, which is what §10 says a protocol check is for. It is recorded here because this table is where a second implementation reads what moved |

**The bump to 2 costs nothing in practice, and saying so is the point of
recording it.** A worker and the hub it talks to are built from one compiler
release, so the handshake triple (§4.1) already refuses a mixed pair at the join
— by compiler version, before `protocol` could ever be the deciding field. What
the bump buys is the case the triple does not cover: a worker binary somebody
upgraded separately, or a second implementation of this document, either of which
would otherwise settle a placed pause into silence. It is a version number doing
what §10 says it is for — refusing a peer rather than letting one be wrong — and
not an upgrade any operator of this release has to plan for.

---

## 11. Out of v1 scope

Named, so that each is a decision rather than a gap (PRD resolved q44):

| | why, and what would change |
|---|---|
| **multi-hub** | §8's invariants keep it possible; shipping it needs the Postgres lease and an arbitration story |
| **worker-to-worker edges** | every edge goes through the hub, which is what keeps one scheduler and one journal |
| **per-placement artifact slicing** | §4.3: an optimisation with a per-placement build, hash and reachability bill |
| **Windows workers** | owner call, 2026-08-29. Linux and macOS in v1 |

Two of those are worth reading as a pair: **no worker-to-worker edges** and
**one execution on one hub** are the same decision seen from two sides, and
together they are why there is exactly one scheduler and exactly one journal to
reason about.

**Containment is not on this list, and the omission is deliberate.** A worker
runs the artifact with its own privileges, exactly as a hand-rolled `exec:` tool
does today — v1 ships no sandbox and no per-placement restriction — but that is
where the campaign arrived rather than something a resolution named out. PRD
resolved q31 fixed v1 containment at root plus timeout and deferred the rest —
containers, seccomp, and "any deploy-target-level restriction (refusing bash on a
distributed placement is a placement fact)" — *to this work*; resolved q37–q44
then settled the topology, the binding surface and the wire without taking the
question up, and q44's out-list does not contain it. So it is an open question,
filed as one in §13 — not a decision this document, or the grammar beside it, may
cite as settled.

---

## 12. What is built today

**Both halves are built.** The static surface is live, and so is the protocol it
describes.

What `validate` enforces: everything grammar §14.1 and §14.2 state — a
placement's name and members, the `flow.*` deferral, disjointness, repeated
members, the colocation rule for an attached tool and for what an attached flow
reaches, the refusal of a store on a process-local backend that a placement's
process could open (§14.1 rule 5, PRD resolved q45), the conditional join token,
and the `public_url:` shape. A deploy file that breaks one of those is a
compile error.

What a **build** emits for a target that declares `placements:`: the hub. The
five routes of §3 on the served app, the artifact server of §3.5 over a content
hash the tree carries, the dispatch board with placement waits on it (§6), the
liveness sweep of §6.3, idempotent effect ingestion (§3.3), the *paused* ending
of §3.4 — a `human:` node a worker reaches lands on the hub's own wait board, is
answered through the ordinary resume surface, and sends the node back through
dispatch with the answered pause in its history — and the environment partition of
§9.1, emitted into the artifact, so the hub checking a join's `env_ok` and a
worker computing one read one answer under one hash. A placed component's node is
dispatch-and-await rather than a call (§7).

What **`agent-compose worker`** is: the spoke. `--hub <url> --claim <name>…
--token-env <VAR> [--data-dir <path>]`, a complete protocol client — the
provisioning cycle of §4, one held poll at a time with a node running beside it
(§2), effect batches as they happen, results, §2's backoff for transport
failures, and the status discipline of §3 and §5. It executes each dispatch by
spawning the node runner the artifact carries, one process per dispatch. Its
store is the artifact in hand and the one it replaced, keyed by hash as §4 step 3
says: a hub rolled back to the older of the two is answered off the disk, and no
third tree accumulates.

What is **not** built, and is named rather than missing: per-placement artifact
slicing (§4.3), multi-hub (§8), worker-to-worker edges and Windows workers
(§11) — and the two questions of §13, each held to what stands for it there:
two knobs nobody has weighed.

**A hub of this release serves exactly one artifact: its own tree.** That is the
fourth named absence, and it is named here because §3.5's table has a row for a
*previous* artifact a hub still holds and this hub holds none — an older hash is
`404`, naming the one being served. Nothing is stranded by it: §3.5's `404`
sends a worker back to a join, and the join answers with the artifact this hub
does have. What it costs is one round trip on a rollback where a hub holding two
would have cost none. Holding a second is additive — a hash is already the whole
of what the route is addressed by — so it needs no wire change when a release
wants it.

Three suites are what make that claim checkable rather than asserted:
`crates/agent-compose/tests/distributed_hub_wire.rs` speaks §3 to a served hub
clause by clause; `crates/agent-compose/tests/distributed_worker_protocol.rs`
holds the worker to every status this document gives it, against a hub that is a
fixture; and `crates/agent-compose/tests/distributed_mesh_acceptance.rs` runs a
real hub and real workers and asks whether a distributed execution works — the
steady state, a cold start, parking and wake, a mid-node disconnect and the
replay that follows it, three hub restarts (one idle, one over a dispatch a
worker is in the middle of running, and one between a placed pause and its
answer), a hub opening a journal written before the dispatch board existed, a
placed `human:` node whose question comes home and whose answer sends the node
back to a worker — including the variant where the worker that asked is gone by
the time the person decides, the wait that is never answered at all, whose budget
runs out on the hub and whose redispatch raises the node's own `on_timeout:`
route on a worker, and the one interval a mesh adds to a wait, where the
dispatching node spends its own budget on a redispatch no worker claims and that
route is never reached (§6.5) — the refusals a worker stops on, and a fan-out
queued onto a pool of one.
`crates/compose-core/tests/placement_surface_landing.rs` holds the surface to
where it lands: the deploy layer's facts in `src/deployment.ts`, the wire in
`src/mesh.ts`, and neither in the composition's own lowering.

**Six clauses of this document were amended while that runtime landed**, and
they are listed here rather than left to a diff: a document the implementation
edited is a document the implementation is measured against, so which sentences
moved has to be as readable as the sentences are.

- **§3.1's stale-hash row**, and the "without exception" clause that rests on it.
  A worker holding an artifact cannot know whether the hub still serves it, so it
  sends the hash and the report together; refusing that pair made the two halves
  of the `env_ok` rule unsatisfiable together for exactly the worker a
  redeployment produces, and its only way out would have been answering a refused
  join with another join. The row now says such a report is **ignored** rather
  than refused.
- **§3.2's four OPTIONAL payload fields** — `session_key`, `item_index`,
  `history`, `policy`. Each is a fact about the node execution the hub holds and
  a worker cannot derive, and a payload short of one is a placed node that
  quietly means something else (§4.3).
- **§3.3's request shape**, which this document had left unwritten: a batch is
  `{ dispatch_id, effects }`, and the two refusals that follow from naming it.
  The dispatch is what tells the hub whose execution the records are and inside
  which instance path — §8's single writer, stated as a route.
- **§3.4's `409` row**, which now covers a result body that names no
  `dispatch_id` at all. That route's table gives four statuses and §10.1 lets an
  implementation rely on them, so answering a fifth outside it would be a status
  a second implementation could meet and act on wrongly — a `4xx` this document
  does not give the route is a refusal no re-send improves, and the *only*
  reading left for it. What such a status costs is a **dispatch** rather than a
  worker, and that is the one place a refusal is not read the way §3.1 reads a
  refused join: §3.1's terminality rests on "a second join would be refused
  identically", which is a statement about this worker's right to be in this mesh
  at all, and a body one hub would not take says nothing of the kind. A worker
  that ended on one would leave the placement with none, and its replacement
  would reach the same record and end the same way. So the attempt fails under
  the node's own `retry:`/`on_error:` chain and the process goes on polling —
  which is also why the two routes a worker POSTs to are mounted with a body
  limit of their own rather than the framework's, since a `413` from underneath
  a handler is precisely such a status and one an effect record can reach without
  anything having gone wrong.
- **§3.5's "an artifact it holds"**, which scopes a MUST that a hub of this
  release could not otherwise meet: it serves exactly one artifact, and the
  paragraph above says what that costs.
- **§9.1's storage-backend clause**: a backend's variables belong to the hub's
  manifest *and* to every placement that reaches a store bound to it, because the
  credential that opens a store is spent in whichever process opens it.

None of the six changes what a peer may rely on at this version (§10.1), and
none is a change §10.3 would bump for: the first is a relaxation, the second and
third name shapes rather than replace them, the fourth moves a body nothing was
promised a status for into a row it already had, and no implementation of
protocol 1 older than this release exists to be made wrong by any of them.

This document is what both halves are held to — **except the rows of §13**,
which are the clauses it does not settle. Those are not wire this document fixes,
and the code they govern is held to the PRD's answer to each rather than to the
placeholder §13 records; until there is one, the defaults §13 records stand and
nothing implements past them.

---

## 13. What this document does not settle

Two questions are **open**, and each one is here because a normative document
may fix a wire and may not fix a design decision the PRD has not made. `prd.md`
is the single source of truth for design decisions; this document is downstream
of it, and §12's "held to" stops at this table.

**So this section is a gate, not a note.** The project's discipline is that a new
design question lands in the PRD's Open Questions and is resolved there before
the affected area is implemented. These two are that list *staged*, which is as
far as this document can take them: entering a question in the PRD's Open
Questions, and resolving it there, is a change to `prd.md` and a reviewed
decision of its own — never something a downstream document performs by
describing it. **Before any code goes past a row's default, that row's question
must be in the PRD's Open Questions and resolved there, and the code written
against the resolution rather than against the cell below.** An implementation
that reads a "what stands in the meantime" cell as wire has decided a PRD
question in a downstream document, which is the thing this section exists to
prevent.

Both remaining rows are the same shape: a **gap** — a knob nobody has weighed,
filled here conservatively, where the wire admits any answer additively. So what
the PRD owes each is a decision rather than a correction, and the runtime that
landed was written to those defaults and no further. Their absences are held
rather than remembered, and `crates/compose-core/src/codegen/mesh.rs` says
exactly how far that holding reaches: it **pins** the emitted session to the four
fields §5 gives it, so a capacity arriving under a spelling nobody has used yet is
still a failing test, and it **greps** the hub's code for the spellings the two
rows have arrived under before — `capacity`, `maxDispatches`, `max_dispatches`
for the first; `sandbox`, `seccomp`, `restrictions` for the second. A grep is a
floor rather than a proof, which is why the first row has the pin as well. Raising
a default is what needs the PRD; keeping one needs a test.

**Three rows have left this table by being answered, which is what the table is
for.** It held a row for a contradiction between resolved q40's mismatch clause
and §4.1's repair, until q40's amendment of 2026-08-30 resolved it; §4.1 now
states that rule as settled wire. It held a row for **a placed component's
`store.*` on a process-local backend** — a *sharp edge*, a composition this
compiler accepted and ran worse than its author would expect — until PRD resolved
q45 (2026-08-31) answered it with a refusal: grammar §14.1 rule 5 is now a
compile error over that shape, at every scope and under `--target local` too, and
§1 and §9.1 state it as a rule rather than as a hazard. And it held the other
sharp edge, **a `human:` node a placed component reaches**, until PRD resolved
q46 (2026-08-31) answered it with a third ending on the wire: §3.4's *paused*
result carries the wait home, the hub plants it on the one board, and the answer
sends the node back through dispatch. That row's own note said the repair would
be a breaking change and a reason for the PRD to weigh it rather than to pre-empt
one here — which is what happened: the resolution took the decision, and
`PROTOCOL_VERSION` moved to 2 (§10.4).

| | what is unsettled | what stands in the meantime |
|---|---|---|
| **a session's dispatch capacity** (§2, §6.3) | how many dispatches one worker session may hold. Resolved q38 fixes that a placement's pool is several workers, and resolved q37 that scale comes from more processes; neither says anything about one session. Raising the number changes what heartbeat loss costs an execution — the difference between failing an attempt and handing work back to the board — which is why it is not a hub's knob | one, as §2 states it — a v1 default this document proposes, which no resolved entry contradicts and none has weighed. The wire admits any other answer additively (an OPTIONAL capacity at join, §10.2), so the runtime may build against one; **raising** it is what the PRD has to answer first |
| **containment beyond the process boundary** (§11) | resolved q31 fixed v1 containment at root plus timeout and deferred containers, seccomp and "any deploy-target-level restriction (refusing bash on a distributed placement is a placement fact)" **to the distribution work**. The distribution resolutions did not take it up, and q44's out-list does not name it, so nothing has decided whether a placement may carry a sandbox or a capability restriction | no such key exists, in the grammar or on the wire; a worker runs the artifact with its own privileges. Grammar D128 retires the reserved `network:` key on the ground that no *resolved* containment story backs it, which is a statement about today rather than about what a later resolution may add |

Neither row blocks the runtime: each names what stands until the PRD answers, and
both arrive additively rather than as a re-cut, which is what lets the runtime
build against their defaults. What they block is the same thing the three
answered rows blocked: an implementation deciding one of them quietly.
