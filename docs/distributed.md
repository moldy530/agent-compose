# agent-compose — Distributed Execution

**Protocol version:** 1
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
10. [Out of v1 scope](#10-out-of-v1-scope)
11. [What is built today](#11-what-is-built-today)

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
`validate` today: a placement is a named entry whose `members:` are `agent.*` and
`tool.*` addresses that resolve; placements are disjoint; an attached tool
colocates with the agent that attaches it; `hub.join_token:` is required wherever
placements are. **A component in no placement executes on the hub.** That is the
default, and it is why a project with no `placements:` is an ordinary
single-process deployment.

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
and the hub answers it when there is work or when the hold expires.

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
| poll hold | 25 seconds (the hub answers `204` at the end of a hold with no work) |
| session liveness window | 90 seconds since the last request on a session |
| worker re-poll after `204` | immediately |
| worker re-join after a transport failure | bounded exponential backoff, starting at 1 second and capped at 30 |

A hold shorter than most intermediary idle timeouts and a liveness window
several holds wide are the two properties those numbers have; an implementation
MAY make them configurable and MUST keep that relationship.

---

## 3. The wire contract

Four routes. Every one of them is authenticated, and every one after the join
carries the session the join returned.

**Authentication.** A worker presents the deploy target's join token as
`Authorization: Bearer <token>`, read from the environment variable
`hub.join_token:` names (grammar §14.2, §4.3). The token is the same on both
sides: the hub reads it to verify, the worker reads it to offer, and neither
holds it in the artifact. There is one scheme and one kind.

### 3.1 `POST /workers/join`

```json
{
  "token": "<presented in the Authorization header, not the body>",
  "claims": ["mac", "gpu"],
  "artifact_hash": "<the hash this worker already holds, if any>",
  "env_ok": ["SIGNING_KEY", "NOTARY_PASSWORD"]
}
```

Answer:

```json
{
  "worker_session": "wrk_9f1c8a3e…",
  "artifact": { "hash": "sha256:…", "url": "/workers/artifact/sha256:…" },
  "poll_url": "/workers/poll"
}
```

`claims` are the deploy layer's placement names. `env_ok` is the worker's own
report of which variables of its placements' manifests are set in its
environment — **names only, never values** (§9). `artifact_hash` is what the
worker already has on disk, which lets an unchanged worker skip the download.

**Refusals**, and the shape of each:

| condition | status | body |
|---|---|---|
| the token does not verify | `401` | no detail. A refused credential is told nothing about why |
| a claim names no placement in the active target | `400` | names the claim, and lists the target's placement names |
| a claimed placement's env manifest is unsatisfied | `403` | names the **variables**, never their values, never whether the hub holds them |
| the worker's artifact hash is stale | — | not a refusal: the join succeeds and the answer carries the current artifact for the worker to fetch (§4) |
| the compiler version or runtime does not match | `409` | names both sides of whichever half differs (§4) |

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

- **`204`** when the hold expired with no work.

**Polling is the heartbeat.** There is no separate liveness route, and there MUST
NOT be one: the property §6 needs is "is this worker able to take work", and a
poll is the only request that answers it. A session with no request inside the
liveness window is presumed gone, and its **queued but undispatched** work
re-parks (§6).

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
`docs/durability.md` §7 refuses.

---

## 4. The artifact

**The hub ships the whole generated project.** Workers hold no checkout and no
YAML; redeployment is automatic on the next join, and version skew is a refusal
rather than a drift (PRD resolved q40).

The artifact is a **tarball of the generated project** — the same tree
`build` writes — served **hash-addressed** from the hub under worker
authentication. Its hash is over the tree's content, so two hubs built from one
composition serve one artifact.

A worker:

1. compares the hash the join returned with what it holds;
2. fetches the tarball when they differ, and verifies the hash it computed
   against the hash it asked for before unpacking anything;
3. materialises it under its own data directory, keyed by hash, so the previous
   artifact survives a rollback;
4. installs dependencies with `bun install`;
5. executes dispatches out of that tree.

### 4.1 The handshake triple

A join agrees on three values, and all three are compared:

| | what it pins |
|---|---|
| **artifact hash** | the generated project, exactly |
| **compiler version** | the `agent-compose` release that generated it |
| **runtime** | Bun, and its major version |

A hash mismatch is not a refusal — the answer carries the current artifact and
the worker fetches it. A **compiler-version or runtime mismatch is a refused
join**, and the refusal names both sides:

> refused: this hub was built by agent-compose 0.4.1 and the worker runs 0.3.9 —
> upgrade the worker, or point it at a hub of its own release

> refused: a worker executes the generated artifact under Bun 1.x and this one
> runs node v22.3.0 — install Bun, or run this placement on a machine that has it

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
sessions and nothing else: workers' next requests are answered `401`/`409` with
"re-join", they re-join, and dispatch resumes from the journal.

A worker MUST therefore treat a session as disposable: any response telling it
the session is unknown is answered by joining again, not by failing.

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

**There is no polling anywhere in this**, and that is a property rather than an
implementation detail: the wake is an event the hub already receives, so a mesh
with nothing to do costs one held request per worker and nothing else.

### 6.3 Heartbeat loss re-parks

When a session passes the liveness window (§2), everything **queued to that
worker and not yet dispatched** re-parks, and the next join claiming that
placement takes it.

### 6.4 Undispatched is a pause; mid-node is a failure

The distinction is the one PRD resolved q39 and q42 draw together, and it decides
what an author writes:

| | what it is | what governs it |
|---|---|---|
| **no worker has taken the node yet** | a **pause** — this is what parking is | nothing, until the node's `timeout:` chain fires from dispatch |
| **a worker took it and disconnected mid-node** | an **attempt failure** | the node's `retry:` / `on_error:` chain, like any execution failure (§7) |

A closed laptop is a pause, not a failure — for the undispatched case. A
placement worth sleeping on therefore wants a `retry:`, because a worker that
vanishes *while running the node* fails that attempt.

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

The partition is exactly this:

- a variable referenced by a component that is a member of placement `P` belongs
  to `P`'s manifest;
- a variable referenced by a component in **no** placement belongs to the
  **hub's** manifest;
- a variable referenced from the deploy layer itself — a storage backend, an
  event source, `hub.join_token:` — belongs to the hub's;
- a variable referenced by components in two different placements belongs to
  both.

This is §7 M3's least-privilege line in its sharpest form: blast-radius
containment falls out of the manifest, because **the hub cannot leak what it
never held**.

### 9.2 The join-time check is self-reported presence

A worker claiming `mac` without `SIGNING_KEY` set is refused at join with the
variable named (§3.1) — the same posture the serve launch check takes
(resolved q32).

**What that check is, precisely: self-reported presence, not authenticated
capability.** The worker sends the *names* it has, and the hub compares them
against the manifest it computed. Proving the worker holds the right *value*
would mean sending the value, which is the one thing this whole section exists to
forbid. The check catches the misconfiguration it is for — a machine joined
before its keychain was set up — and claims nothing more.

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

## 10. Out of v1 scope

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

## 11. What is built today

**The static surface is live. The protocol is not built yet.**

What `validate` enforces now: everything grammar §14.1 and §14.2 state — a
placement's name and members, the `flow.*` deferral, disjointness, the
attached-tool colocation rule, the conditional join token, and the `public_url:`
shape. A deploy file that breaks one of those is a compile error today.

What does not exist yet: the worker verb, the four routes of §3, the artifact
server of §4, placement waits on the board, and effect streaming. **Nothing a
placement or a `hub:` block declares reaches the project a build emits.**

That middle state is deliberate and it is bound rather than remembered:
`crates/compose-core/tests/placement_surface_inertness.rs` asserts that no
placement or hub material reaches a generated project, pins the sentences in this
document and in the grammar that say so, and enumerates what the runtime pass has
to unwind — including teaching the environment manifest about §9.1's partition,
which is the half of the surface with no sentence of its own.

This document is what that runtime will be held to.
