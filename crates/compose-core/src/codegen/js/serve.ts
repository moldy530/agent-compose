//
// The generated app: the graph as an HTTP endpoint (PRD 5.11, grammar 13.3).
//
// `./triggers.ts` is the composition's own trigger table — one entry per
// declared `http` trigger, with its route, its method, its response mode and the
// CEL that turns a request payload into the flow's inputs. This module is what
// that table becomes: a Fastify app exposing PRD 5.11's three verbs.
//
// | verb | route | what it does |
// |---|---|---|
// | start | the trigger's own `path:`, at its own `method:` | validates the payload, starts an execution, and answers per `respond:` |
// | status | `GET /executions/:id` | the execution's state, its outputs once it has them, and any pause it is holding |
// | resume | `POST /executions/:id/resume` | delivers a human's answer to an interrupted execution |
// | cancel | `POST /executions/:id/cancel` | cancels an open execution, and every open child execution in its lineage |
// | source status | `GET /sources/:name` | a `kind: command` event source's program, checkpoint, restarts and refused lines |
// | intake status | `GET /intakes/:id` | an arrival a trigger's `concurrency:` queued, debounced, started or dropped |
//
// It is byte-identical in every project this compiler release builds, like
// `./runtime.ts`: what differs between two projects is `./triggers.ts`.
//
// # Admission: one function every trigger kind's start passes through
//
// An execution starts in one place, [`admit`], whatever started it: an `http`
// request after its auth, its body and its `input:`; a `schedule` tick the
// scheduler found due; an `event` a `kind: command` source printed (PRD
// resolved q78, `./sources.ts`). Each arrives with its inputs bound and its
// flow's `inputs:` checked, and with the **gate** its kind keeps — a tick's
// `fired` row, claimed before the execution starts so a tick fires once across
// restarts and hubs; an event's dedupe key, claimed when the execution is
// registered so a repeat starts nothing (`docs/durability.md` §3.11, §3.12).
//
// # The intake ledger (PRD resolved q77, Decision D163)
//
// PRD resolved q77's intake lands on the same seam, extended rather than
// forked. A trigger may declare a `dedupe_key:` (`http`, beside the one an
// `event` trigger always has) and a `dedupe_window:` — the TTL of the key's
// row — and a `concurrency:` block: its arrivals are admitted into **groups**,
// each a lease held by its running executions and read off the journal's rows
// at every decision, never counted in this process ([`IntakeLedger`]). An
// arrival that finds no free slot is queued, dropped or supersedes what is
// running; one under a `debounce:` is coalesced into the group's one pending
// row. A queued or pending arrival has no execution yet, so it is answered with
// an intake id of its own, which `GET /intakes/:id` reports on. The order every
// admission keeps: the trigger's `auth:`, the payload decoded and the keys
// evaluated, the gate claimed (a tick's `fired` row, the dedupe key), the
// group asked — and nothing registered before the gate is claimed.
//
// # Why Fastify
//
// PRD 5.11 names it ("`http` → a generated Fastify app wrapping the compiled
// graph"), and the emitted `package.json` pins it exactly for the reason every
// other pin is exact (PRD 5.12): a framework that changed how a body is decoded,
// or what an unparseable one answers, would change what a compiled graph does
// with no commit saying so.
//
// # `respond:` (grammar 13.3, PRD §9.8)
//
// * **`async`** — the default. The execution starts, the route answers `202`
//   with an execution id and a status URL immediately, and the optional
//   `callback:` webhook fires when the run finishes.
// * **`sync`** — the route waits for the run, up to the trigger's mandatory
//   `timeout:`. On expiry the response **upgrades to async**: `202` with the same
//   execution id and status URL, while the execution carries on. Nothing is
//   cancelled and no work is lost, which is the whole of what the upgrade is for.
//
// # What an execution is here, and what it is not
//
// Executions are tracked **in this process**: a `Map` from id to the run's state
// and the promise it settles. The *record* is this process's; the **run** is
// not — every execution is journaled as it goes (`./journal.ts`, PRD resolved
// q26-q29), and [`Recovery`] puts every one the journal holds open back on this
// map before the app accepts a connection. So a restart loses the reports of
// executions that had already finished and keeps the ones that had not, which is
// the half that matters: a status route answers `404` for an id neither this
// process nor the journal knows, and the sync-timeout upgrade continues the same
// in-process execution it started.
//
// **Nothing is evicted**, and that is a decision rather than an omission. The
// map holds every execution this process started or recovered, with its outputs
// and its trace, so a long-running `serve` grows with the number of requests it
// has answered. The alternative is an eviction policy, and every policy this
// milestone could write is a `404` for an execution that really ran — a caller
// polling a status URL it was handed, told the run never existed. A retention
// story needs somewhere for an evicted execution's *report* to be, which the
// journal is not (it records effects, not reports), so the boundary is stated
// here and in the emitted `README.md` rather than approximated with a bound.
//
// # Resume (grammar 8.7, PRD 5.11)
//
// A `human` node parks its execution: the graph stops advancing, the status
// route reports `interrupted`, and the report carries what the human is shown
// and what their answer has to fit, so a UI polling the status can present the
// question without reading the composition.
//
// The answer comes back as the **body** of `POST /executions/:id/resume`,
// validated against that node's `output:` — the schema the status route
// published. A payload that does not fit is a `400` that does **not** consume
// the wait: the execution stays interrupted and the same answer can be sent
// again once it is corrected. Every other refusal here is about *which* pause,
// not about what was in the body, and each is a `4xx` naming what happened:
// there is nothing waiting, the wait already expired, or the execution is
// holding more than one pause and the request named none of them — or named
// more than one.
//
// **Addressing a pause.** One execution can hold more than one at a time — a
// `human` node inside a `map`-dispatched flow is the reachable case — so a
// resume may carry `?wait=<id>`, where the id is grammar 9.4's instance path
// flattened (`approve/0`, `review/0/2/approve/0`). It may be omitted where the
// execution is holding exactly one, which is what the status route's
// `resume_url` does for a caller that never has to think about it. Exactly one
// `wait` is what it addresses: a repeated `?wait=` names two pauses and is
// refused as such, rather than joined into an id nothing is holding.
//
// **What durability there is.** The wait is a promise parked in this process,
// and a restarted `serve` does not *hold* it — it **replays** the execution out
// of the journal and parks again, under the same `wait_id`, because that id is
// the node's instance path (grammar 9.4) and no process generation is part of
// it. So a `resume_url` handed out by the process that died answers in the one
// that replaced it. What the journal holds no record of is a wait nobody
// answered — there is nothing to record about one — which is exactly what makes
// re-parking the right thing to do with it (PRD resolved q28,
// `docs/durability.md` §3.4, §6.1).
//
// **Replaying back to it takes as long as it takes**, and recovery does not wait
// for that (§6.1). So a resume can arrive while the wait is still ahead of the
// replay, and that request is refused with a refusal of its own — `recovering:
// true`, and a sentence that says to send it again — rather than with the
// sentence that means the pause is over. It is the one refusal on this route
// about *when* a request arrived rather than about what it addressed, and it is
// decided per **pause** rather than per execution: two branches of one execution
// reach their pauses independently, so one being back says nothing about the
// other (see [`stillReplayingTo`]).
//
// # Who may call: `auth:` (grammar 13.3, PRD resolved q32)
//
// A trigger declaring `auth:` verifies its caller before anything else happens:
// no payload is read, no execution exists, and a refusal is a `401` naming the
// trigger and the scheme and **nothing else** — echoing any part of a
// credential, even the one that arrived, would put it in a log somebody ships.
//
// A credential that resolved to the **empty string** is refused at launch rather
// than served: `src/env.ts` counts an empty variable as present, which is the
// right rule for a region or a base URL and the wrong one for a token that would
// then compare equal to the one every anonymous caller sends (see
// [`blankCredentials`]).
//
// The subtle clause is that `auth:` covers **four** routes rather than one.
// `resume` injects data into a parked run, `status` publishes what a run is
// holding and `cancel` stops one, and all three are per execution rather than
// per trigger — so each enforces the auth of **the trigger that started that
// execution**, which the journal's lifecycle row records (`docs/durability.md`
// §3.5). An execution a no-auth
// trigger began keeps open routes; one an authenticated trigger began never
// answers an unauthenticated poll, in this process or in the one that recovers
// it after a restart.
//
// # Cancel (PRD resolved q75, Decision D160)
//
// `POST /executions/:id/cancel` stops an open execution: the request is recorded
// on the journal first, then the execution's one abort controller fires — every
// running node's signal with it, model calls, spawned commands, harness runs and
// placed dispatches alike — and every open child execution in its lineage is
// cancelled the same way (`runtime.cancelExecution`). Its waits are released,
// each ending instance runs its `finally:` node at outcome `cancelled`, and the
// row settles `cancelled`: the status route says so, the settle webhook carries
// it, and the trace ships. `202` for an execution still open — a second cancel
// of one already cancelling included, since the request stands either way — and
// `409` for one that has completed, failed or been cancelled.
//
// # Lifecycle webhooks (grammar 13.3, PRD resolved q34, q35)
//
// A trigger's `callback:` is a subscription to the execution's **lifecycle**
// rather than only to its end. Two events reach it: a **parking**, one webhook
// per quiescence listing every pause then open — a quiescence being the moment
// every branch of the execution has parked or finished, which
// `runtime.quiescent` decides and [`parking`] waits for — and the **settle**,
// journaled by the run itself immediately before it closes the lifecycle row.
// Each carries the report the status route serves and the `X-AgentCompose-*`
// headers grammar 13.3 tabulates, signed and identified by `callback_auth:`
// where the trigger declares one, and refused before it is sent where
// `callback_allow:` admits its URL nowhere.
//
// Deliveries are **journaled effects of their own** (`docs/durability.md` §3.7):
// the intent is recorded before the first attempt, each attempt's outcome after
// it, and a restarted `serve` picks up what is still pending beside the
// executions it recovers. A delivery that exhausts its schedule is recorded and
// visible on the status route, and is never the execution's failure — a webhook
// is a courtesy the status route backstops.

import { createHmac, timingSafeEqual } from "node:crypto";
import process from "node:process";

import Fastify from "fastify";
import type { FastifyInstance, FastifyReply, FastifyRequest } from "fastify";

import {
  JOURNAL_RETRY_MS,
  beingWorked,
  blankSinkCredentials,
  insisting,
  message,
  pause,
  retrySchedule,
  secret,
  shipTrace,
  sinkAuth,
  sinkConfigured,
  workDelivery,
} from "./delivery.ts";
import { type CompiledFlow, type FlowRun, flows, runChildFlow, runFlow, sessionRefusal } from "./graph.ts";
import {
  type DedupeRow,
  type FiredRow,
  type IntakeAdmission,
  type IntakeChange,
  type IntakeRow,
  type JoinedArrival,
  type Journal,
  type Lease,
  openJournal,
} from "./journal.ts";
import { mountWorkerRoutes, placementWaits, watchPlacementWaits } from "./mesh.ts";
import {
  type Admitted,
  type Intake,
  type TickAdmitted,
  declaresSource,
  preflightSources,
  sourceStatus,
  startSources,
  stopSources,
  triggersOf,
} from "./sources.ts";
import { drainStores, sweepBoundStores } from "./stores.ts";
import {
  cancelExecution,
  cancelFired,
  deliverHumanAnswer,
  deliveriesOf,
  endedByCancel,
  executionReport,
  exhaustRecordedDelivery,
  hostChildren,
  humanWaits,
  intendDelivery,
  journaledExecution,
  openExecutions,
  quiescent,
  recordDeliveryAttempt,
  refuseDelivery,
  refuseRecordedDelivery,
  resumeChild,
  undeliveredDeliveries,
  watchHumanPauses,
  watchQuiescence,
} from "./runtime.ts";
import type * as runtime from "./runtime.ts";
import {
  type Concurrency,
  type EventPayload,
  type EventTrigger,
  type HttpTrigger,
  type InboundAuth,
  type SchedulePayload,
  type ScheduleTrigger,
  eventTriggers,
  httpTriggers,
  scheduleTriggers,
} from "./triggers.ts";

export { ScheduleError, UnimplementedSourceError } from "./sources.ts";

/** What an execution this process started is doing (PRD 5.11). */
export type ExecutionStatus = "running" | "completed" | "failed" | "interrupted" | "cancelled";

/** One execution, tracked in this process for as long as the process lives. */
interface Execution {
  readonly id: string;
  readonly flow: string;
  readonly trigger: string;
  /**
   * What the run itself has done: `running` until it stops, then `completed` or
   * `failed`.
   *
   * `interrupted` is never written here — it is derived at report time from the
   * pauses the runtime is holding ([`statusOf`]), because a pause is not a state
   * the run transitions into and out of: the graph is still mid-superstep, and a
   * resume puts it straight back to work. Deriving it means the two can never
   * disagree, which a stored flag updated from two sides eventually would.
   */
  status: Exclude<ExecutionStatus, "interrupted">;
  /**
   * Whether this is an execution [`Recovery`] picked up whose replay is **still
   * running**.
   *
   * `false` for every execution a request started, and for a recovered one from
   * the moment its run ends.
   *
   * What it is for is one sentence on the resume route. Recovery does not wait
   * for the replays it starts (`docs/durability.md` §6.1), so an execution is on
   * this map as `running` while it is still consuming its recorded prefix — and
   * a `POST /executions/:id/resume` prepared against the process that died can
   * land in that window.
   *
   * It is deliberately **not** cleared when the execution re-parks. One
   * execution can hold more than one pause (grammar 8.6), the branches reach
   * them independently, and a branch whose prefix the crash left an effect of
   * has to run that effect live before it re-parks at all — so the first pause
   * back says nothing about the second. What decides whether a *particular*
   * answer arrived early is [`stillReplayingTo`], off the pause it named.
   */
  recovering: boolean;
  /**
   * Whether a cancel of this execution has been accepted here and the run has
   * not settled yet (PRD resolved q75) — what lets the resume route say why
   * nothing is waiting rather than which pause is not.
   */
  cancelling: boolean;
  /**
   * Whether this execution has held a pause since this process picked it up.
   *
   * The other half of [`stillReplayingTo`], and the half a wait id cannot
   * supply: a resume that names **no** pause is not about any particular one, so
   * the only thing that makes "there is nothing waiting" premature is a board
   * that has held nothing at all. Read off the pauses the runtime publishes
   * rather than asserted from the replay, for [`statusOf`]'s reason: the board
   * is where a wait *is*.
   */
  parked: boolean;
  /**
   * Where this execution's lifecycle webhooks go, where its trigger asked for
   * one (grammar 13.3's `callback:`).
   *
   * Read when the request arrived rather than when an event happens, and
   * recorded on the lifecycle row, because the process that finishes an
   * execution need not be the one that started it (`docs/durability.md` §6.1).
   */
  readonly callback?: string;
  /**
   * The pauses a `parked` delivery has already reported, by wait id.
   *
   * What keeps a recovered execution from re-announcing a question nobody
   * asked again: re-parking under the same wait ids is what recovery *is*
   * (`docs/durability.md` §6.1), so a parking webhook fires only where a
   * quiescence opened a pause this set does not hold. Seeded from the journal's
   * own delivery rows before the replay starts, so the knowledge survives the
   * process that made the deliveries (PRD resolved q35).
   */
  readonly reported: Set<string>;
  /**
   * The chain that allocates this execution's event ordinals in order.
   *
   * Ordinals are what a receiver orders by, so two lifecycle events racing to
   * claim one would publish an order the execution did not have. The *attempts*
   * are deliberately not on this chain — a delivery that is retrying for ten
   * minutes must not hold up the next event's ordinal, and grammar 13.3 already
   * tells receivers that deliveries can arrive out of order.
   */
  deliveries: Promise<void>;
  /**
   * Whether this execution's `settled` delivery has already been queued.
   *
   * Written by [`closed`], which runs **inside** the run, before the lifecycle
   * row closes. What reads it is [`settling`]'s tail, whose subject is the one
   * settle the hook is never reached for: a run that failed before it was
   * journaled at all. Without it that tail would push a second `settled` webhook
   * for every ordinary run.
   */
  announced: boolean;
  outputs?: Record<string, unknown>;
  trace?: readonly runtime.TraceEntry[];
  error?: string;
  /**
   * What a `finally:` node of the run raised after its own retries, where one
   * did — beside a `status` it did not change (grammar 7.9, PRD resolved q76).
   */
  finallyError?: string;
  /** Resolves when the run has stopped, however it stopped. */
  settled: Promise<void>;
}

/**
 * What an execution is doing, pauses included (PRD 5.11).
 *
 * A run that has stopped reports how it stopped. One that has not is
 * `interrupted` exactly while the runtime is holding a pause for it, and
 * `running` otherwise.
 */
function statusOf(execution: Execution): ExecutionStatus {
  if (execution.status !== "running") return execution.status;
  return humanWaits(execution.id).length > 0 ? "interrupted" : "running";
}

/**
 * Whether a resume this recovered execution could not deliver arrived **before
 * the replay got to the pause it named** — the window `docs/durability.md` §6.1
 * describes, decided per request rather than per execution.
 *
 * Per request because one execution can hold more than one pause and its
 * branches reach them independently (grammar 8.6): a branch whose recorded
 * prefix the crash left an effect of has to run that effect live before it
 * re-parks, while a branch whose prefix is whole is back at once. A window that
 * closed on the first pause the board saw would hand the second branch's client
 * the final refusal this one exists to prevent.
 *
 * The two refusals it decides are decided differently, because only one of them
 * names a pause.
 *
 *  * `no-such-wait` **is** the proof: a pause stays on the board once it opens,
 *    answered or expired or still waiting (see `runtime.deliverHumanAnswer`,
 *    which finds a settled one and says so). So an id the board does not know is
 *    an id this generation has not reached, and while the replay runs it may
 *    still reach it.
 *  * `not-waiting` names nothing, so there is no pause to ask about. What makes
 *    it premature is a board that has held **none at all**: once this generation
 *    has published a pause, an unaddressed answer arriving to an empty board is
 *    being told the truth about the board it was sent to.
 */
function stillReplayingTo(
  execution: Execution,
  reason: Extract<runtime.ResumeOutcome, { ok: false }>["reason"],
): boolean {
  if (reason === "no-such-wait") return true;
  return reason === "not-waiting" && !execution.parked;
}

/** The payload shape grammar 13.3 fixes, as one request presents it. */
export interface Payload {
  readonly body: unknown;
  readonly query: Readonly<Record<string, string>>;
  readonly headers: Readonly<Record<string, string>>;
  readonly path: string;
  readonly method: string;
  /**
   * The arrivals a debounce coalesced into the execution this payload starts
   * (PRD resolved q77 ruling c): present only on a trigger whose `concurrency:`
   * declares `debounce:`, and `1` for a lone arrival.
   */
  readonly coalesced?: number;
}

/** How the app is built and what it is told about itself. */
export interface ServeOptions {
  readonly host?: string;
  readonly port?: number;
}

/**
 * Build the app over this composition's declared `http` triggers.
 *
 * Exported so an ejected project can mount the same routes inside a server of
 * its own — the app is the composition's invocation surface, not this file's.
 */
export function createApp(): FastifyInstance {
  const app = Fastify({ logger: false });
  const executions = new Map<string, Execution>();
  // The intake every trigger declaring `concurrency:` admits through (PRD
  // resolved q77): one per app, found from the map [`admit`] is handed.
  const ledger = new IntakeLedger(executions);
  ledgers.set(executions, ledger);
  liveLedgers.add(ledger);
  // Read here, before a route exists, so an unreadable override is a command
  // that could not be run rather than a schedule nobody notices until the first
  // delivery is already late (Decision D50, `docs/durability.md` §3.7).
  retrySchedule();
  // And, for the same reason one line up, a credential that resolved to nothing:
  // a route mounted over one is open to everybody and says nothing about it
  // (see [`blankCredentials`]).
  const blank = blankCredentials();
  if (blank.length > 0) throw new BlankCredentialError(blank);
  // And what the scheduler and the consumers would otherwise fail on after the
  // app is listening: an event source of a kind this release runs no consumer
  // for — refused by name, PRD resolved q78's q63 posture — a timezone the
  // runtime's database does not know, a clock variable that is not one.
  preflightSources();
  decodeBodies(app);

  // The worker protocol's five routes, on a target that declares `placements:`
  // and on no other (`docs/distributed.md` §2, §3). Mounted **before** the
  // recovery hook below, so the dispatch rows a dead process left are back on
  // the board before the replays that reach them start (§5, `./mesh.ts`).
  mountWorkerRoutes(app);

  for (const trigger of httpTriggers) {
    app.route({
      method: trigger.method,
      url: trigger.path,
      handler: (request, reply) => start(executions, trigger, request, reply),
    });
  }

  // **Child executions are this app's executions** (PRD resolved q65): every
  // one a detached `flow.*` dispatch starts in this process — under a request's
  // execution or a recovered one — is tracked on the same map, answers on the
  // same status route by its own id, and ships its own trace from the hook that
  // closes its own row ([`trackChild`]). Hosted **before** the recovery hook
  // below, because recovery is one of the ways a child starts here: an open
  // child is resumed — natively where its parent has settled, and through its
  // recovered parent's re-issued dispatch where the parent is open too. And let
  // go of when the app closes, so an ejected caller that builds a second app in
  // one process tracks a child on the map of the app that is still serving.
  const unhost = hostChildren((child) => trackChild(executions, child));
  app.addHook("onClose", async () => {
    unhost();
  });

  // Recovery, on the hook Fastify runs **before** the server accepts a
  // connection: `onReady` is awaited by `listen`, so every execution the
  // journal holds open has been put back on the board before the first request
  // arrives (PRD resolved q28). A resume request that lands the instant after
  // `listen` resolves therefore finds its wait, which is the whole promise —
  // the wait id is deterministic (node path + ordinal), so it is the same id the
  // caller was given by the process that died.
  const recovery = new Recovery(executions);
  app.addHook("onReady", async () => {
    await recovery.atStart();
    // **After** recovery and before any arrival (PRD resolved q77 ruling f):
    // the intake rows a dead process left — a start it claimed and never
    // journaled, its queues, its open debounces — are taken up once the open
    // executions holding their groups' leases are back on the board.
    await ledger.atStart();
    // **After** recovery (PRD resolved q78 ruling c): the schedules start
    // taking ticks and each `kind: command` source's program starts with the
    // newest checkpoint the process that died had accepted — and no execution
    // a tick or an event starts is one a replay is racing. A recovery this
    // start could not finish — a standby's, on a journal another `serve` is
    // writing — is finished before any tick or event is admitted
    // ([`Recovery.before`]), so that ordering holds for a takeover too.
    await startSources(intake(executions, recovery));
  });
  // The programs go with the app: `SIGTERM` to each group, `SIGKILL` after a
  // grace (Decision D160's exec discipline), and no tick armed after it — nor
  // a recovery retry.
  app.addHook("onClose", async () => {
    recovery.stop();
    ledger.stop();
    liveLedgers.delete(ledger);
    await stopSources();
  });

  app.get("/executions/:id", async (request, reply) => {
    const id = (request.params as { id: string }).id;
    const execution = executions.get(id);
    if (execution === undefined) return unknownExecution(reply, id);
    const refusal = guarded(execution, request);
    if (refusal !== undefined) return refuse(reply, execution.trigger, refusal);
    return reply.code(200).send(await report(execution));
  });

  app.post("/executions/:id/resume", async (request, reply) => {
    const id = (request.params as { id: string }).id;
    const execution = executions.get(id);
    // The existence check first: a resume against an id this process never
    // started is a different mistake from a resume that arrived at the wrong
    // moment, and answering both the same way would hide the first.
    if (execution === undefined) return unknownExecution(reply, id);
    // …and the guard second, ahead of everything that reads the body or the
    // board: an execution an authenticated trigger started never answers an
    // unauthenticated resume, and a refusal that had already told the caller
    // what the execution is holding would have answered the question it
    // refused (grammar 13.3, PRD resolved q32).
    const refusal = guarded(execution, request);
    if (refusal !== undefined) return refuse(reply, execution.trigger, refusal);
    if (execution.status === "cancelled") {
      return reply.code(409).send({
        execution_id: id,
        status: execution.status,
        error: CANCELLED_RESUME,
      });
    }
    if (execution.status !== "running") {
      return reply.code(409).send({
        execution_id: id,
        status: execution.status,
        error: `this execution has already ${execution.status}, so nothing is waiting for an answer`,
      });
    }
    // A cancel accepted and not yet settled has released every wait this
    // execution held, so an answer arriving now has nothing to settle.
    if (execution.cancelling) {
      return reply.code(409).send({
        execution_id: id,
        status: statusOf(execution),
        cancelling: true,
        error: CANCELLING_RESUME,
      });
    }
    // Fastify's default query parser answers a **repeated** key with an array,
    // so `?wait=a&wait=b` arrives here as `["a", "b"]`. Refused rather than
    // joined or first-wins: every refusal on this route says what actually
    // happened, and a request that named two pauses cannot be told `no pause
    // \`a,b\`` — an id no client ever sent — nor answered by picking one of the
    // two, which would settle a pause the request did not unambiguously name.
    // Like every other refusal about *which* pause, it consumes nothing.
    const asked = (request.query as { wait?: string | string[] }).wait;
    if (Array.isArray(asked)) {
      const pending = humanWaits(id).map((wait) => wait.id);
      return reply.code(400).send({
        execution_id: id,
        status: statusOf(execution),
        wait: asked,
        ...(pending.length === 0 ? {} : { pending }),
        error: `a resume answers exactly one pause, and this request carried \`wait\` ${asked.length} times`,
      });
    }
    const named = asked;
    // **Awaited**, which is what makes the `202` below a statement about the
    // journal rather than only about this process's memory: `deliverHumanAnswer`
    // settles the wait and then waits for the record of it to be appended, so a
    // hub that dies between this line and the reply has already written the
    // answer it is about to acknowledge (`runtime.deliverHumanAnswer`,
    // `docs/durability.md` §3.4).
    const outcome = await deliverHumanAnswer(id, named, request.body);
    if (outcome.ok) {
      // `202` rather than `200`: the answer has been delivered and the graph has
      // gone back to work, which the status route is where to watch. The run is
      // not finished, and a `200` carrying no outputs would read as if it were.
      //
      // Derived rather than asserted, for [`statusOf`]'s own reason: answering
      // one of two pauses leaves the execution `interrupted`, and a body that
      // said `running` would tell a client that trusts it to stop polling for
      // the second question.
      return reply.code(202).send({
        execution_id: id,
        wait: outcome.wait.id,
        status: statusOf(execution),
        status_url: `/executions/${id}`,
      });
    }
    // **A recovered execution the replay has not brought back to this pause has
    // refused nothing.** The two refusals below that mean "no such pause here" —
    // there is none at all, or none under the id you named — are true of the
    // board and false of the execution while a replay is still on its way to
    // that wait ([`Execution.recovering`], `docs/durability.md` §6.1). Both
    // sentences read as final, and one of them is the very sentence a *settled*
    // pause is refused with, so a client holding a `resume_url` the dead process
    // handed out would drop an answer nothing was wrong with. It is told to send
    // it again instead, and given a key to decide that on rather than a sentence
    // to match: this is the one refusal on this route that is about *when* the
    // request arrived.
    if (execution.recovering && stillReplayingTo(execution, outcome.reason)) {
      return reply.code(409).send({
        execution_id: id,
        status: statusOf(execution),
        ...(named === undefined ? {} : { wait: named }),
        recovering: true,
        error: `this execution is being recovered from the journal and has not come back to its pause yet, so this answer has not been refused: send it again`,
      });
    }
    return reply.code(outcome.reason === "mismatch" ? 400 : 409).send({
      execution_id: id,
      status: statusOf(execution),
      ...(named === undefined ? {} : { wait: named }),
      ...(outcome.pending === undefined ? {} : { pending: outcome.pending }),
      error:
        outcome.reason === "mismatch"
          ? `the resume payload does not fit the \`human\` node's \`output:\`, so the execution is still waiting for one that does: ${outcome.detail}`
          : outcome.detail,
    });
  });

  app.post("/executions/:id/cancel", async (request, reply) => {
    const id = (request.params as { id: string }).id;
    const execution = executions.get(id);
    // The order the resume route keeps, for its reasons: an id this process
    // does not know is a different mistake from a cancel the caller may not
    // make, and the guard comes before anything about the execution is said
    // (grammar 13.3, PRD resolved q32).
    if (execution === undefined) return unknownExecution(reply, id);
    const refusal = guarded(execution, request);
    if (refusal !== undefined) return refuse(reply, execution.trigger, refusal);
    if (execution.status !== "running") {
      return reply.code(409).send({
        execution_id: id,
        status: execution.status,
        error: `this execution has already ${execution.status === "cancelled" ? "been cancelled" : execution.status}, so there is nothing to cancel`,
      });
    }
    // **Recorded before it is acted on**, and awaited, which is what makes the
    // `202` a statement about the journal: a process that dies between this
    // line and the reply has already written the request, and the start that
    // recovers the execution completes the cancel rather than resuming it
    // (`runtime.cancelExecution`, `docs/durability.md` §6.1). A second cancel
    // of an execution already cancelling is the same request again, and
    // answered the same way.
    //
    // Marked before the await, so an answer arriving while the request is
    // being written is refused — and **unmarked if it fails**: a journal that
    // would not take the request leaves nothing recorded and nothing fired,
    // the execution running at its waits, and a mark left standing would
    // refuse every answer to them for the life of the process. Kept where the
    // cancel did fire before the failure — a cascade that could not finish —
    // since its waits are released by then.
    const was = execution.cancelling;
    execution.cancelling = true;
    try {
      await cancelExecution(id);
    } catch (error) {
      if (!cancelFired(id)) execution.cancelling = was;
      throw error;
    }
    // A recorded cancel releases the execution's slot of its trigger's lease
    // (PRD resolved q77, Decision D163), so the queue it was holding back is
    // looked at now rather than when the run settles.
    released(execution.trigger);
    return reply.code(202).send({
      execution_id: id,
      status: statusOf(execution),
      cancelling: true,
      status_url: `/executions/${id}`,
    });
  });

  app.get("/sources/:name", async (request, reply) => {
    const name = (request.params as { name: string }).name;
    // The existence check first, as on the execution routes: a name this
    // target does not declare is a different mistake from a caller the route
    // refuses.
    if (!declaresSource(name)) {
      return reply.code(404).send({
        error: `no event source \`${name}\` is declared under this target (grammar 14.4)`,
      });
    }
    // …and the guard second, on the reading PRD resolved q32 takes one level
    // up: the route enforces the `auth:` of the `event` triggers bound to the
    // source. Where any of them declares one, a caller must satisfy one of
    // them; where none does, the route is open, exactly as an execution a
    // no-auth trigger started keeps open routes (Decision D162, whose Status
    // names the reading).
    const refusal = sourceGuard(name, request);
    if (refusal !== undefined) {
      return reply.code(401).send({
        source: name,
        ...(refusal.scheme === undefined ? {} : { scheme: refusal.scheme }),
        error: `the event source \`${name}\` refused this request: ${refusal.detail}`,
      });
    }
    const status = await sourceStatus(name);
    if (status === undefined) {
      return reply.code(404).send({
        error: `no event source \`${name}\` is declared under this target (grammar 14.4)`,
      });
    }
    return reply.code(200).send(status);
  });

  app.get("/intakes/:id", async (request, reply) => {
    const id = (request.params as { id: string }).id;
    // The order the execution routes keep: the existence check first, then
    // the guard of the trigger whose arrival this is (PRD resolved q77 ruling
    // d, q32), and only then anything about the row.
    const row = await (await openJournal()).intake(id);
    if (row === undefined) {
      return reply.code(404).send({ error: `no intake \`${id}\` is held in this project's journal` });
    }
    const refusal = intakeGuard(row, request);
    if (refusal !== undefined) return refuse(reply, row.trigger, refusal);
    return reply.code(200).send(intakeReport(row));
  });

  return app;
}

/** What the resume route answers for an execution somebody cancelled (PRD resolved q75). */
const CANCELLED_RESUME =
  "this execution was cancelled, so nothing is waiting for an answer: a cancelled execution is ended for good and is never resumed";

/** …and for one whose cancel is accepted and still settling. */
const CANCELLING_RESUME =
  "this execution is being cancelled, so every wait it held has been released and nothing is waiting for an answer";

// ---------------------------------------------------------------------------
// Verifying a caller (grammar 13.3, PRD resolved q32)
// ---------------------------------------------------------------------------

/** The name a lifecycle row carries for an invocation no `http` trigger made. */
const MANUAL = "manual";

/**
 * Why a request was refused, in the words the caller is given.
 *
 * `detail` says what was wrong with the *credential* — that none arrived, or
 * that the one that did does not verify — and never carries any part of one.
 * Echoing what arrived would put an attacker's guess, or a genuine token sent
 * to the wrong route, into whatever reads a `401` body.
 */
interface Refusal {
  /** Which scheme refused, where a scheme did. */
  readonly scheme?: "bearer" | "hmac";
  readonly detail: string;
}

/** Answer a request the trigger's `auth:` refused: `401`, and nothing else. */
function refuse(reply: FastifyReply, trigger: string, refusal: Refusal): unknown {
  return reply.code(401).send({
    trigger,
    ...(refusal.scheme === undefined ? {} : { scheme: refusal.scheme }),
    error: `the trigger \`${trigger}\` refused this request: ${refusal.detail}`,
  });
}

/**
 * The refusal this trigger's `auth:` has for this request, or `undefined` where
 * it has none.
 *
 * A trigger declaring no `auth:` refuses nothing, which is grammar 13.3's own
 * default: absent leaves the route open.
 */
function verified(
  trigger: { readonly auth?: InboundAuth },
  request: FastifyRequest,
): Refusal | undefined {
  const auth = trigger.auth;
  if (auth === undefined) return undefined;
  const sent = headerValues(request, auth.header);
  if (sent.length === 0) {
    return {
      scheme: auth.scheme,
      detail: `no \`${auth.header}\` header carried a credential`,
    };
  }
  // **A credential offered twice is not one this route can say it verified.**
  // Which of two values a reader takes is a thing runtimes and the proxies in
  // front of them disagree about ([`headerValues`]), and a request whose
  // credential depends on who resolved that disagreement has not presented one.
  // Refused before the comparison, so nothing about either value is measurable.
  if (sent.length > 1) {
    return {
      scheme: auth.scheme,
      detail: `the \`${auth.header}\` header arrived ${sent.length} times, and a request offering more than one credential is not one this trigger can say it verified`,
    };
  }
  const only = sent[0] ?? "";
  if (!only.startsWith(auth.prefix)) {
    return {
      scheme: auth.scheme,
      detail: `the \`${auth.header}\` header does not begin with the expected prefix`,
    };
  }
  const offered = only.slice(auth.prefix.length);
  // **A credential that resolved to nothing verifies nothing.** `createApp`
  // refuses to start a deployment holding one ([`blankCredentials`]), so this
  // is the guard for the window that check cannot cover — an environment edited
  // under a running process — and it is stated here as well as there because
  // the two failures it prevents are the whole of what `auth:` is for: an empty
  // expected token is equal to the empty token every anonymous caller can send,
  // and an empty HMAC key signs a body anybody can sign.
  const resolved = secret(auth.scheme === "bearer" ? auth.tokenEnv : auth.secretEnv);
  if (resolved === "") {
    return {
      scheme: auth.scheme,
      detail: "the credential this trigger verifies against is not set in this process",
    };
  }
  if (auth.scheme === "bearer") {
    return equal(offered, resolved)
      ? undefined
      : { scheme: "bearer", detail: "the credential does not match" };
  }
  // The signature is over the **raw request body bytes**, before any JSON
  // decoding and after none of it (grammar 13.3): a re-serialized body is a
  // different byte string and would fail every signature a vendor computed. See
  // [`decodeBodies`], which is what keeps those bytes.
  const signed = createHmac(auth.algorithm, resolved).update(raw(request)).digest(auth.encoding);
  return equal(offered, signed)
    ? undefined
    : { scheme: "hmac", detail: "the signature does not verify over this request's body" };
}

/**
 * Every credential this deployment declares that resolved to **nothing**, by
 * variable name.
 *
 * `src/env.ts` decides *presence*, and presence there is `!== undefined`: a
 * variable set to the empty string is set (grammar 4.3). That is the right rule
 * for the class of value it is written about — a `base_url:`, a region — and the
 * wrong one for a credential, because an empty one is not a missing setting but
 * an **open door**: an empty bearer token compares equal to the empty token any
 * caller can send, and an HMAC key of no bytes signs a body anybody can sign. So
 * the four surfaces of grammar 13.3 are held to more than presence, and they are
 * held to it at launch — one refusal a deployment meets on its first start,
 * rather than a route that answers `401` to the caller holding the right token
 * or a delivery signed with nothing.
 *
 * **Five surfaces rather than four**, because the deploy layer spends a
 * credential of its own: `trace_sink.auth:` is §13.3's outbound block unchanged
 * (grammar §14.5 rule 3), and a sink signed with nothing is the same open door
 * one delivery further out — every settled execution's trace pushed to a
 * collector under an `Authorization: Bearer ` with nothing after it. It is asked
 * about in [`blankSinkCredentials`], beside the sink's own shape, and named here
 * so that one launch refusal covers everything this app can present.
 */
function blankCredentials(): readonly string[] {
  const blank: string[] = [];
  const named = (variable: string): void => {
    if (secret(variable) === "" && !blank.includes(variable)) blank.push(variable);
  };
  for (const trigger of httpTriggers) {
    const auth = trigger.auth;
    if (auth?.scheme === "bearer") named(auth.tokenEnv);
    if (auth?.scheme === "hmac") named(auth.secretEnv);
    const delivery = trigger.callbackAuth;
    if (delivery?.bearer !== undefined) named(delivery.bearer.tokenEnv);
    if (delivery?.hmac !== undefined) named(delivery.hmac.secretEnv);
  }
  // An `event` trigger's `auth:` guards the routes about its work (grammar
  // 13.5, Decision D162), and an empty credential would open them as surely.
  for (const trigger of eventTriggers) {
    const auth = trigger.auth;
    if (auth?.scheme === "bearer") named(auth.tokenEnv);
    if (auth?.scheme === "hmac") named(auth.secretEnv);
  }
  for (const variable of blankSinkCredentials()) named(variable);
  return blank;
}

/**
 * What a trigger's `auth:` or `callback_auth:`, or the deploy layer's
 * `trace_sink.auth:`, named and this process cannot use.
 *
 * The sentence names **the deployment** rather than a trigger, because after
 * grammar §14.5 the credential may belong to the target instead: an operator
 * reading it is pointed at the variable, which is the half that repairs it
 * either way.
 */
export class BlankCredentialError extends Error {
  constructor(blank: readonly string[]) {
    super(
      `${blank.map((name) => `\`${name}\``).join(", ")} ${blank.length === 1 ? "is" : "are"} set to the empty string, and this deployment ${blank.length === 1 ? "verifies or signs with it" : "verifies or signs with them"}: an empty credential admits every caller and signs every delivery, so this app refuses to serve until ${blank.length === 1 ? "it is given a value" : "they are given values"} (grammar 13.3, 14.5)`,
    );
    this.name = "BlankCredentialError";
  }
}

/**
 * The refusal the trigger that **started** `execution` has for this request
 * (grammar 13.3: "`auth:` covers three routes, not one").
 *
 * Which trigger that was is the journal's lifecycle row, so the guard survives
 * the process that answered the `202`: a `serve` restarted while somebody was
 * thinking re-parks the execution and goes on refusing the same callers
 * (`docs/durability.md` §3.5, §6.1).
 *
 * Three answers, and the order of the first two is what settles a name that
 * could be either. A **declared trigger** enforces its own `auth:`, whatever it
 * is called. Otherwise `manual` is the row a `run` and a `manual` trigger both
 * write, and there is no trigger to enforce — the routes are open, exactly as
 * they are for an execution an unauthenticated `http` trigger began. What is
 * left is a row naming a trigger **this build no longer declares**: the
 * composition moved under an open execution, and this process cannot know what
 * the trigger that started it demanded. Answering the poll would be answering
 * on a guarantee nobody can read, so it is refused until the composition is put
 * back.
 */
function guarded(execution: Execution, request: FastifyRequest): Refusal | undefined {
  return guardedBy(execution.trigger, request);
}

/**
 * [`guarded`]'s rule, for whatever the trigger called `name` began — an
 * execution, or an arrival its intake holds (`GET /intakes/:id`).
 */
function guardedBy(name: string, request: FastifyRequest): Refusal | undefined {
  const execution = { trigger: name };
  const trigger = httpTriggers.find((one) => one.name === execution.trigger);
  if (trigger !== undefined) return verified(trigger, request);
  // An `event` trigger enforces its own `auth:` on the routes of the
  // executions it started, on PRD resolved q32's coverage clause — nobody
  // called it, but somebody may poll, resume or cancel what it began
  // (grammar 13.5, Decision D162).
  const event = eventTriggers.find((one) => one.name === execution.trigger);
  if (event !== undefined) return verified(event, request);
  if (execution.trigger === MANUAL) return undefined;
  // A `schedule` trigger is a declared trigger with no `auth:` — grammar 13.4
  // gives it none, since nobody calls a clock — so the routes of an execution
  // one started are open, PRD resolved q32's "an execution a no-auth trigger
  // started keeps open routes" (Decision D162).
  if (scheduleTriggers.some((one) => one.name === execution.trigger)) return undefined;
  return {
    detail: `it was started by the trigger \`${execution.trigger}\`, which this build does not declare, so the authentication that trigger required cannot be read`,
  };
}

/**
 * The refusal `GET /sources/:name` has for this request (grammar 14.4, PRD
 * resolved q32 one level up, Decision D162).
 *
 * The route reports on a source, and a source belongs to the `event` triggers
 * bound to it, so it enforces theirs: where none declares an `auth:` it is
 * open; where some do, a request any one of them verifies is answered — each
 * of those triggers may already read what the source started through its own
 * executions' routes, so none of them learns more here than it could there.
 */
function sourceGuard(source: string, request: FastifyRequest): Refusal | undefined {
  const guarding = triggersOf(source).filter((trigger) => trigger.auth !== undefined);
  if (guarding.length === 0) return undefined;
  const refusals: { readonly trigger: string; readonly refusal: Refusal }[] = [];
  for (const trigger of guarding) {
    const refusal = verified(trigger, request);
    if (refusal === undefined) return undefined;
    refusals.push({ trigger: trigger.name, refusal });
  }
  const only = refusals.length === 1 ? refusals[0] : undefined;
  if (only !== undefined) {
    return {
      ...(only.refusal.scheme === undefined ? {} : { scheme: only.refusal.scheme }),
      detail: `it enforces the \`auth:\` of the trigger \`${only.trigger}\` bound to it, and ${only.refusal.detail}`,
    };
  }
  return {
    detail: `it enforces the \`auth:\` of the triggers bound to it, and none of them verified it (${refusals.map(({ trigger, refusal }) => `\`${trigger}\`: ${refusal.detail}`).join("; ")})`,
  };
}

/**
 * Every value this request carried under `name`, matched **case-insensitively**
 * (grammar 13.3), in the order it carried them.
 *
 * Header names are case-insensitive by definition and HTTP/2 lowercases every
 * one on the wire, so a check that compared the author's capitalisation would
 * refuse every genuine delivery over HTTP/2 while passing a `curl` that
 * happened to preserve case.
 *
 * Read off `raw.rawHeaders` — the list as it arrived — rather than off the
 * parsed `request.headers`, which is what makes "sent more than once" a thing
 * this surface can see at all. The parsed map holds **one** value per name, and
 * the two runtimes this project runs under disagree about which: Node keeps the
 * first `authorization` and drops the rest, Bun keeps the last, and both join
 * repeats of any other name with `", "`. So a route reading that map verifies a
 * different credential depending on what the app was launched with, and a proxy
 * in front of it normalising on the other value is the whole of a
 * header-smuggling differential. The arrival list has no opinion to disagree
 * with, and [`verified`] refuses a credential that appears in it twice.
 */
function headerValues(request: FastifyRequest, name: string): readonly string[] {
  const wanted = name.toLowerCase();
  // A flat `[name, value, name, value, …]` list, which is why the step is two.
  const arrived = request.raw.rawHeaders;
  const values: string[] = [];
  for (let index = 0; index + 1 < arrived.length; index += 2) {
    if ((arrived[index] ?? "").toLowerCase() === wanted) values.push(arrived[index + 1] ?? "");
  }
  return values;
}

/**
 * Whether two credentials are equal, compared in **constant time**
 * (grammar 13.3).
 *
 * A byte-by-byte early return leaks the expected value to a caller who can time
 * it — for `bearer` the token itself, and for `hmac` the expected digest for a
 * body of the caller's choosing, which forges a validly signed request without
 * the caller ever holding the secret.
 *
 * A **length** mismatch is refused before the comparison, because
 * `timingSafeEqual` requires equal-length views and there is nothing to leak:
 * what the length of an arriving credential is, its sender already knows. What
 * is not leaked is any statement about its *content*, which is the whole of
 * what the requirement is about.
 */
function equal(offered: string, expected: string): boolean {
  // An expected credential of **no bytes** is equal to the credential an
  // anonymous caller sends, so it is refused before the comparison rather than
  // handed to one that would answer `true`. [`verified`] has already refused it
  // by then; this is the statement made where the comparison is, so that no
  // later caller of this can reintroduce it.
  if (expected.length === 0) return false;
  const left = new TextEncoder().encode(offered);
  const right = new TextEncoder().encode(expected);
  if (left.length !== right.length) return false;
  return timingSafeEqual(left, right);
}

/**
 * The bytes a request arrived with, exactly as they arrived.
 *
 * Kept by [`decodeBodies`] under the request itself, in a map that holds no
 * request alive past its own handling. A request the framework parsed no body
 * for — a `GET`, or a trigger that reads none — signs over nothing, which is
 * the empty string a vendor would have signed too.
 */
function raw(request: FastifyRequest): Uint8Array {
  return bodies.get(request) ?? new Uint8Array();
}

/** Every in-flight request's raw body. See [`raw`], [`decodeBodies`]. */
const bodies = new WeakMap<FastifyRequest, Uint8Array>();

/**
 * One pause, as the status route publishes it (PRD 5.11, grammar 8.7).
 *
 * `snake_case` because these are document keys, alongside `execution_id` and
 * `status` on the same report — the seam `docs/trace.md` §2 names, where an
 * entry's keys are the runtime record's `camelCase` and a document's are these.
 */
function question(execution: Execution, wait: runtime.HumanWait): Record<string, unknown> {
  return {
    wait_id: wait.id,
    flow: wait.flow,
    node: wait.node,
    paused_at: wait.pausedAt,
    ...(wait.expiresAt === undefined ? {} : { expires_at: wait.expiresAt }),
    input: wait.shown,
    output_schema: wait.schema,
    resume_url: `/executions/${execution.id}/resume?wait=${encodeURIComponent(wait.id)}`,
  };
}

/**
 * One decoding rule for every request body, which is grammar 13.3's own
 * (Decision D117).
 *
 * The framework's parsers are removed and replaced by a single one, because
 * D117 states the rule over *bodies* rather than over media types and Fastify's
 * default set does not decide it that way:
 *
 *  * a body-bearing method sent with an **empty** body presents `{}` and starts
 *    an execution — where the stock `application/json` parser refuses the same
 *    request `400 FST_ERR_CTP_EMPTY_JSON_BODY` before a handler runs, making
 *    whether an execution starts turn on a `content-type:` header the grammar
 *    gives no meaning to: the identical request sent without the header
 *    presents `{}` and runs;
 *  * a body that is present and is **not** a decodable JSON object is refused
 *    at request time, which [`start`] does with one message for every way of
 *    not being one — a media type nothing decodes (`415` from the stock set), a
 *    body that is not JSON, and JSON that is not an object.
 *
 * What a framework does with a body it cannot decode, and what status it answers
 * with, is part of what a compiled graph does — which is why `package.json` pins
 * Fastify exactly — so the two rows above are settled here rather than inherited
 * and described in a README.
 *
 * **The bytes are kept as well as decoded**, which is what an inbound `hmac:`
 * verifies over (grammar 13.3): a signature is computed over the raw request
 * body, and a body re-serialized from the value this parser produced is a
 * different byte string that would fail every signature a vendor computed. So
 * the parser reads a buffer rather than a string and files it under the request
 * ([`raw`]) before decoding a copy of it.
 */
function decodeBodies(app: FastifyInstance): void {
  app.removeAllContentTypeParsers();
  app.addContentTypeParser("*", { parseAs: "buffer" }, (request, body, done) => {
    const bytes = body as Uint8Array;
    bodies.set(request, bytes);
    const text = new TextDecoder().decode(bytes);
    // Empty, or nothing but whitespace — which is the same request with a
    // newline in it, and no caller means one as a payload.
    if (text.trim() === "") {
      done(null, {});
      return;
    }
    try {
      done(null, JSON.parse(text) as unknown);
    } catch {
      // Handed on as the text it is: what a body that does not decode answers
      // is [`start`]'s to say, in the same sentence a decodable non-object
      // gets, rather than two shapes of refusal for one rule.
      done(null, text);
    }
  });
}

/** Start one execution for a trigger (grammar 13.3's `start`). */
async function start(
  executions: Map<string, Execution>,
  trigger: HttpTrigger,
  request: FastifyRequest,
  reply: FastifyReply,
): Promise<unknown> {
  // **First**, before the payload is read and before anything exists to report
  // (grammar 13.3, PRD resolved q32). A route that verified after building the
  // payload would have run this composition's CEL over an unverified request,
  // and one that verified after registering would leave an execution behind for
  // every refused call.
  const refusal = verified(trigger, request);
  if (refusal !== undefined) return refuse(reply, trigger.name, refusal);

  const flow = flows[trigger.flow];
  if (flow === undefined) {
    return reply
      .code(500)
      .send({ error: `\`${trigger.flow}\` is not a flow of this composition` });
  }

  // A `GET` decodes no body and presents `{}`; a body-bearing method that
  // arrived without one presents `{}` too, and one that is present and is not a
  // JSON **object** — whatever media type it announced, see [`decodeBodies`] —
  // starts no execution (grammar 13.3, Decision D117).
  //
  // The two cases are told apart by `undefined`, and only by it: a request the
  // framework parsed **no** body for is the one way `request.body` is
  // `undefined`, because [`decodeBodies`] answers every body it is handed with
  // an object, the decoded value, or the undecodable text. So a body that is
  // present and decodes to JSON `null` arrives here as `null` and is refused
  // below with every other non-object — `?? {}` would make it the empty payload
  // of a request that carried nothing, and start an execution D117 says starts
  // none.
  const decoded = trigger.readsBody ? request.body : undefined;
  const body = decoded === undefined ? {} : decoded;
  if (typeof body !== "object" || body === null || Array.isArray(body)) {
    return reply.code(400).send({
      error: "the request body is not a JSON object, so there is nothing for the trigger's `input:` to read (grammar 13.3, Decision D117)",
    });
  }

  const payload: Payload = {
    body,
    query: strings(request.query),
    headers: strings(request.headers),
    path: request.url.split("?")[0] ?? request.url,
    method: request.method,
  };

  // Under a debounce an arrival is bound as the lone arrival it would be —
  // `payload.coalesced` is `1` — which is what holds a request the trigger
  // could never start to a `400` now rather than at its deadline; the one
  // execution is bound again when the debounce closes, with the count it closed
  // over (PRD resolved q77 ruling c, Decision D163).
  const concurrency = trigger.concurrency;
  const bound: Payload =
    concurrency?.debounceMs === undefined ? payload : { ...payload, coalesced: 1 };
  let inputs: Record<string, unknown>;
  let sessionKey = "";
  let dedupe: string | undefined;
  let group: string | undefined;
  try {
    // The two keys an intake decides on, over the payload as it arrived —
    // after the trigger's `auth:` above, before anything exists (PRD resolved
    // q77 ruling a, b).
    if (trigger.dedupeKey !== undefined) dedupe = trigger.dedupeKey(payload);
    if (concurrency?.key !== undefined) group = concurrency.key(payload);
    inputs = trigger.input(bound);
    if (trigger.sessionKey !== undefined) sessionKey = trigger.sessionKey(bound);
  } catch (error) {
    return reply.code(400).send({
      error: `the trigger \`${trigger.name}\` could not read this request: ${message(error)}`,
    });
  }
  // A key that answered the empty string names nothing a repeat could be told
  // by: every request whose field is empty, or whose fallback answered `''`,
  // would be one delivery for the whole window, and all but the first would
  // start nothing. It is refused, as an empty `session_key:` a flow needs is
  // (Decision D163's Status).
  if (dedupe === "") {
    return reply.code(400).send({
      error: `the trigger \`${trigger.name}\` could not read this request: ${EMPTY_DEDUPE_KEY}`,
    });
  }

  // The payload is held to the flow's own `inputs:` before an execution exists,
  // which is what makes a bad request a `400` rather than a failed run: the same
  // schema `runFlow` parses with, asked one step earlier (grammar 13.1). The
  // message already names the flow's `inputs:` and the field inside it that did
  // not fit — `CompiledFlow.parse` reports through `runtime.parseResult` — so
  // this says what the *request* did and leaves the naming to it.
  try {
    flow.parse(inputs);
  } catch (error) {
    return reply.code(400).send({
      error: `the request does not fit ${message(error)}`,
    });
  }
  // An arrival an intake may hold is one it must be able to start later
  // without asking anybody, and one a gate keeps is answered only once its
  // lifecycle row is down — so a refusal `runFlow` would raise before that row
  // ([`refusesSession`]) would read as the journal's failure: a `503` that says
  // the request may be sent again, and a key every resend takes over and fails
  // the same way. Under either, a session key the flow would refuse is the
  // trigger's binding failing, and is refused now, as a tick's or an event's is.
  if (concurrency !== undefined || dedupe !== undefined) {
    try {
      refusesSession(flow, sessionKey);
    } catch (error) {
      return reply.code(400).send({
        error: `the trigger \`${trigger.name}\` could not admit this request: ${message(error)}`,
      });
    }
  }

  // The caller's own trace, where the caller named one (PRD resolved q51), and
  // the completion webhook this request asked for — both read here, when the
  // request arrived, and journaled on the lifecycle row: the export that adopts
  // the trace and the delivery that calls back happen at settle, which may be
  // in another process. Header names are case-insensitive and this framework
  // lowercases them, so the lookup is the lowercase spelling W3C writes anyway.
  const callback = callbackOf(trigger, bound);
  const traceparent = (payload.headers["traceparent"] ?? "").trim();
  const intaking = concurrency === undefined ? undefined : httpIntake(trigger, concurrency);
  // A gate or an intake decides against the journal, and one that cannot be
  // read decides nothing: the request was not admitted, nothing started, and
  // it may be sent again — said as such rather than as the framework's `500`.
  const gated = dedupe !== undefined || intaking !== undefined;
  let admitted: AdmitAnswer;
  try {
    admitted = await admit(executions, {
    trigger: trigger.name,
    flow,
    inputs,
    sessionKey,
    ...(callback === undefined ? {} : { callback }),
    ...(traceparent === "" ? {} : { traceparent }),
    ...(dedupe === undefined
      ? {}
      : {
          gate: {
            kind: "dedupe",
            key: dedupe,
            windowMs: trigger.dedupeWindowMs ?? DEFAULT_DEDUPE_WINDOW_MS,
          },
        }),
    ...(intaking === undefined
      ? {}
      : {
          intake: {
            trigger: intaking,
            ...(group === undefined ? {} : { group }),
            payload: withoutCredential(trigger, payload),
          },
        }),
    });
  } catch (error) {
    if (!gated) throw error;
    return reply.code(503).send({
      error: `the trigger \`${trigger.name}\` could not admit this request (${message(error)}): nothing started, so the request may be sent again`,
    });
  }
  return answered(executions, trigger, reply, admitted);
}

/**
 * Answer one `http` admission (grammar 13.3, PRD resolved q77 ruling a, b, d).
 *
 * The same shape a fresh start has wherever there is an execution to name —
 * `202` with its id and status URL, or under `sync` the outputs within the
 * budget — and `202` with an `intake_id` and the intake's status URL where
 * there is none yet. A repeat inside its dedupe window carries
 * `X-AgentCompose-Deduped: true` and names what the first delivery got; an
 * arrival its full group dropped carries `X-AgentCompose-Dropped: true` and
 * names the execution running it.
 */
async function answered(
  executions: Map<string, Execution>,
  trigger: HttpTrigger,
  reply: FastifyReply,
  admitted: AdmitAnswer,
): Promise<unknown> {
  switch (admitted.outcome) {
    case "started":
      return responded(trigger, reply, admitted.record);
    case "held": {
      reply.header(DEDUPED_HEADER, "true");
      const holder = admitted.intake;
      if (holder !== undefined && (holder.state === "queued" || holder.state === "pending")) {
        return intakeAccepted(reply, holder);
      }
      // **A repeat of a dropped arrival is the drop** (grammar 13.3's table,
      // PRD resolved q77 ruling b): the arrival never ran, so it is answered
      // as the drop was — the running execution's id beside the arrival's own
      // `intake_id`, under `X-AgentCompose-Dropped: true` — and never as that
      // execution's start, which under `sync` would wait on it and hand the
      // caller another payload's outputs as its own. A row dropped with no
      // execution to name answers as the intake it is ([`journaledAnswer`]).
      if (holder?.state === "dropped") {
        if (holder.heldBy !== undefined && holder.heldBy !== "") {
          return droppedAnswer(reply, holder.heldBy, holder);
        }
        return journaledAnswer(reply, holder.execution, holder);
      }
      const record = executions.get(admitted.execution);
      // Under `sync`, a repeat waits on the execution the first delivery
      // started, up to the budget, as the first did (Decision D163's Status).
      if (record !== undefined) return responded(trigger, reply, record);
      return journaledAnswer(reply, admitted.execution, holder);
    }
    case "queued":
    case "pending":
      return intakeAccepted(reply, admitted.intake);
    case "dropped":
      return droppedAnswer(reply, admitted.running, admitted.intake);
    case "unbegun":
      // Nothing started and nothing was answered: the gate this request claimed
      // names an execution that exists nowhere, so the same request sent again
      // takes it over rather than being told it is a repeat.
      return reply.code(503).send({
        error: `the trigger \`${trigger.name}\` admitted this request, but its execution could not be journaled (${admitted.reason}): nothing started, so the request may be sent again`,
      });
  }
}

/**
 * What a dropped arrival is answered, and a repeat of one with it (grammar
 * 13.3's table): `202` naming the execution its full group was running and
 * the arrival's own `intake_id`, under `X-AgentCompose-Dropped: true` —
 * whatever `respond:` says, because the arrival started nothing to wait on.
 */
function droppedAnswer(reply: FastifyReply, running: string, row: IntakeRow): unknown {
  reply.header(DROPPED_HEADER, "true");
  return reply.code(202).send({
    execution_id: running,
    status_url: `/executions/${running}`,
    intake_id: row.id,
  });
}

/**
 * A repeat's answer where the execution it names is not this process's: one
 * that settled before this process started, which recovery does not put back
 * on its board (`docs/durability.md` §6.1). Its lifecycle row answers — the
 * id, the status, the status URL, the shape an `async` start has — and under
 * `respond: sync` the same `202`, because the row holds no outputs to answer
 * with (Decision D163's Status). An intake row owed a start whose execution
 * has no row yet answers as the intake it is, and so does one dropped with no
 * execution to name — an admission that no longer bound when its turn came
 * (`#stillBinds`), or a debounce whose payload did not bind at its close —
 * whose execution never began and never will.
 */
async function journaledAnswer(
  reply: FastifyReply,
  id: string,
  holder: IntakeRow | undefined,
): Promise<unknown> {
  let row: runtime.ExecutionRow | undefined;
  try {
    row = await (await openJournal()).execution(id);
  } catch {
    row = undefined;
  }
  if (
    row === undefined &&
    holder !== undefined &&
    (holder.state === "started" || holder.state === "dropped")
  ) {
    return reply.code(202).send({
      intake_id: holder.id,
      status: holder.state,
      status_url: `/intakes/${holder.id}`,
    });
  }
  const status: ExecutionStatus =
    row === undefined || row.status === "open" ? "running" : row.status;
  return reply.code(202).send({ execution_id: id, status, status_url: `/executions/${id}` });
}

/** What a started execution answers, per `respond:` (grammar 13.3). */
async function responded(
  trigger: HttpTrigger,
  reply: FastifyReply,
  execution: Execution,
): Promise<unknown> {
  if (trigger.respond === "sync") {
    const budget = trigger.timeoutMs ?? DEFAULT_SYNC_TIMEOUT_MS;
    const finished = await within(execution.settled, budget);
    if (finished) {
      if (execution.status === "completed") {
        return reply
          .code(200)
          .send({ execution_id: execution.id, status: execution.status, outputs: execution.outputs });
      }
      return reply
        .code(500)
        .send({ execution_id: execution.id, status: execution.status, error: execution.error });
    }
    // The upgrade of grammar 13.3: the budget is the *response's*, not the
    // run's, so the execution carries on and the caller is handed the id and the
    // status URL it now needs.
    return accepted(reply, execution);
  }
  return accepted(reply, execution);
}

/** The `60s` grammar 13.3 defaults a sync trigger's response budget to. */
const DEFAULT_SYNC_TIMEOUT_MS = 60_000;

// ---------------------------------------------------------------------------
// Admission (PRD resolved q78 ruling e)
// ---------------------------------------------------------------------------

/**
 * One execution about to start, from whichever trigger started it — its inputs
 * bound and already held to its flow's `inputs:`.
 */
interface Admission {
  /** The trigger's name, as the lifecycle row records it. */
  readonly trigger: string;
  readonly flow: CompiledFlow;
  readonly inputs: Record<string, unknown>;
  readonly sessionKey: string;
  /** An `http` trigger's resolved `callback:`, where the request yielded one. */
  readonly callback?: string;
  /** The `traceparent` an `http` request carried. */
  readonly traceparent?: string;
  /**
   * The gate this trigger kind keeps, where it keeps one: a `schedule` tick's
   * `fired` row, or an `event` trigger's dedupe key.
   */
  readonly gate?:
    | { readonly kind: "tick"; readonly scheduledAt: string }
    | {
        readonly kind: "dedupe";
        readonly key: string;
        /** `dedupe_window:` — the TTL of the key's row (PRD resolved q77 ruling a). */
        readonly windowMs: number;
      };
  /**
   * The id a gate's row already names for this execution, where a process
   * died holding the row: a tick's `fired` row whose execution never began
   * ([`admitUnbegun`]). Absent, a fresh id is allocated.
   */
  readonly execution?: string;
  /**
   * The intake the arrival is admitted under, where its trigger declares
   * `concurrency:` (PRD resolved q77, [`IntakeLedger`]).
   */
  readonly intake?: AdmissionIntake;
}

/**
 * What [`admit`] did: the execution it started, the one its gate already held,
 * or — for a gated admission only — a gate it claimed for an execution the
 * journal would not begin.
 */
type AdmitAnswer =
  | { readonly outcome: "started"; readonly record: Execution }
  /**
   * The gate already held: the tick fired, or the key admitted, as
   * `execution` — and `intake`, where an intake row holds that execution and
   * it has not started (or never will: a dropped arrival).
   */
  | { readonly outcome: "held"; readonly execution: string; readonly intake?: IntakeRow }
  /** Its group had no free slot, and `on_conflict: queue` waits for one. */
  | { readonly outcome: "queued"; readonly intake: IntakeRow }
  /** A debounce opened or extended over it (PRD resolved q77 ruling c). */
  | { readonly outcome: "pending"; readonly intake: IntakeRow }
  /** Its group had no free slot, and `on_conflict: drop` answered `running`. */
  | { readonly outcome: "dropped"; readonly intake: IntakeRow; readonly running: string }
  | {
      readonly outcome: "unbegun";
      readonly execution: string;
      /** The tick's `fired` row, where the gate was a tick's. */
      readonly fired?: FiredRow;
      readonly reason: string;
    };

/**
 * Start one execution — **the** start, for every trigger kind (PRD resolved
 * q78 ruling e).
 *
 * The execution's id is allocated first, so the gate's row can name it:
 *
 *  * **a tick's `fired` row**, claimed before the execution exists. The insert
 *    that wins fires; a tick some earlier generation of this process or another
 *    hub sharing the journal already fired is answered with that execution and
 *    starts nothing (`docs/durability.md` §3.11). The one row it is handed
 *    back is a claim no execution began under — `admission.execution` names
 *    the id it recorded — and that starts under the id only where no execution
 *    of it began, which is the tick's repair: honoured, the tick would never
 *    fire.
 *  * **an event's dedupe key**, claimed as the execution is registered. A key
 *    already held by an execution that exists — in this process, or anywhere in
 *    the journal — is a repeat and starts nothing. A key held by one that
 *    **never began** is a claim no execution began under, and is taken over
 *    rather than honoured: honoured, the event would be lost (§3.12).
 *
 * **A gated admission answers `started` only once the lifecycle row is down**
 * — `runFlow`'s `opened` hook, [`register`]'s `begun` — and never as soon as
 * the run is set going. What its caller does next is only safe for an
 * execution the journal holds: a consumer journals the event's checkpoint, a
 * scheduler claims its next tick. A run the journal would not begin is
 * answered `unbegun` instead, and taken off this process's map, so the claim
 * it leaves names an execution that exists nowhere: the scheduler holds its
 * schedule and starts it under the claim ([`admitUnbegun`]), and an event's
 * retry takes the key over.
 *
 * An `http` request with no `dedupe_key:` keeps no gate, and is answered as
 * soon as its run is set going, as it always was: a run that fails before its
 * row is reported by the status route and the completion webhook
 * ([`settling`]). One with a key keeps the dedupe gate on this same seam (PRD
 * resolved q77 ruling a), and is answered only once its lifecycle row is down,
 * as a tick or an event is.
 */
async function admit(
  executions: Map<string, Execution>,
  admission: Admission,
): Promise<AdmitAnswer> {
  // An arrival under a `concurrency:` block is the intake's to decide (PRD
  // resolved q77), on this same seam: the gate below, then the group.
  if (admission.intake !== undefined) {
    const ledger = ledgers.get(executions);
    if (ledger === undefined) throw new Error("no intake ledger is attached to this app");
    return ledger.admit({ ...admission, intake: admission.intake });
  }
  const id = admission.execution ?? newExecutionId();
  let claim: FiredRow | undefined;
  if (admission.gate !== undefined) {
    const gate = await claimGate(executions, await openJournal(), admission, id);
    if (gate.outcome === "held") return gate;
    claim = gate.fired;
  }
  const { record, begun: lifecycle } = register(executions, admission, id);
  if (admission.gate === undefined) return { outcome: "started", record };
  try {
    await lifecycle;
  } catch (error) {
    // Never begun, so never this process's either: off the map, the claim
    // names an execution [`begun`] finds nowhere, which is what the repair and
    // the takeover look for — and a repeat of this event in this process is
    // not answered with an execution that does not exist.
    if (executions.get(id) === record) executions.delete(id);
    return {
      outcome: "unbegun",
      execution: id,
      ...(claim === undefined ? {} : { fired: claim }),
      reason: message(error),
    };
  }
  return { outcome: "started", record };
}

/**
 * Claim the gate an admission's kind keeps, for the execution `id` — before
 * anything is registered (PRD resolved q78 ruling e, q77 ruling a).
 *
 *  * **A tick's `fired` row**: the insert that wins fires; the row it is
 *    handed back for a claim no execution began under is the tick's repair.
 *  * **A dedupe key**, held for its window: a key already held by an
 *    admission that began — an execution this process or the journal holds,
 *    or one an intake row holds and will start — is a repeat and is answered
 *    with it. A key held by one that **never began** is taken over: honoured,
 *    the arrival would be lost (`docs/durability.md` §3.12).
 */
async function claimGate(
  executions: Map<string, Execution>,
  journal: Journal,
  admission: Admission,
  id: string,
): Promise<
  | { readonly outcome: "claimed"; readonly fired?: FiredRow }
  | { readonly outcome: "held"; readonly execution: string; readonly intake?: IntakeRow }
> {
  const gate = admission.gate;
  // A repeat is answered with the execution its holder is or will be: under a
  // debounce, the one the row that counted it starts — which a gate a crash
  // cut short of its fold still names by the arrival's own id (D163).
  const repeat = async (holder: string) => {
    const intake = await journal.intakeOf(holder);
    return {
      outcome: "held" as const,
      execution: intake?.execution ?? holder,
      ...(intake === undefined ? {} : { intake }),
    };
  };
  if (gate?.kind === "tick") {
    const held = await journal.fire({
      trigger: admission.trigger,
      scheduledAt: gate.scheduledAt,
      execution: id,
      firedAt: new Date().toISOString(),
    });
    if (held.execution !== id) return repeat(held.execution);
    if (admission.execution !== undefined && (await begun(executions, journal, id))) {
      return repeat(id);
    }
    return { outcome: "claimed", fired: held };
  }
  if (gate?.kind === "dedupe") {
    const claim = (): DedupeRow => {
      const at = Date.now();
      return {
        trigger: admission.trigger,
        key: gate.key,
        execution: id,
        admittedAt: new Date(at).toISOString(),
        expiresAt: new Date(at + gate.windowMs).toISOString(),
      };
    };
    const held = await journal.claimDedupe(claim());
    if (held.execution !== id) {
      if (await begun(executions, journal, held.execution)) return repeat(held.execution);
      const retaken = claim();
      if (
        !(await journal.retakeDedupe(
          admission.trigger,
          gate.key,
          held.execution,
          id,
          retaken.expiresAt,
        ))
      ) {
        const now = await journal.claimDedupe(claim());
        if (now.execution !== id) return repeat(now.execution);
      }
    }
  }
  return { outcome: "claimed" };
}

/**
 * Whether `id` names an execution that began: one this process holds — its
 * lifecycle row may still be in flight, and a gated admission whose row the
 * journal refused is taken off the map ([`admit`]) — or one the journal has a
 * lifecycle row for. What a gate's row names and this answers `false` for is
 * a claim no execution began under: a process died holding it, or the journal
 * would not begin its execution.
 */
async function begun(
  executions: Map<string, Execution>,
  journal: Journal,
  id: string,
): Promise<boolean> {
  if (executions.has(id) || (await journal.execution(id)) !== undefined) return true;
  // …or one an intake accepted (PRD resolved q77): queued, pending, dropped,
  // or started after it waited — which the intake starts, or repairs at its
  // next start ([`IntakeLedger`]) — or an arrival a debounce counted, whose
  // gate was claimed under `id` and not yet folded into the row's execution
  // ([`Journal.intakeOf`] reads both). A `started` row that never waited and
  // whose execution never began was answered to nobody, and is no admission.
  const intake = await journal.intakeOf(id);
  return intake !== undefined && (intake.state !== "started" || intake.waited);
}

/**
 * The [`Intake`] `./sources.ts` hands its ticks and events to: each bound to
 * its flow's inputs and held to its flow's `inputs:` here, then [`admit`]ted —
 * and none of them before `recovery` is done ([`Recovery.before`]), which
 * rejects while it cannot be, as a journal that could not be written does.
 */
function intake(executions: Map<string, Execution>, recovery: Recovery): Intake {
  return {
    tick: async (trigger, at, payload) => {
      await recovery.before();
      return admitTick(executions, trigger, at, payload);
    },
    unbegun: async (trigger, fired, payload) => {
      await recovery.before();
      return admitUnbegun(executions, trigger, fired, payload);
    },
    event: async (trigger, payload) => {
      await recovery.before();
      return admitEvent(executions, trigger, payload);
    },
  };
}

/** Bind one tick and admit it under its `fired` gate. */
async function admitTick(
  executions: Map<string, Execution>,
  trigger: ScheduleTrigger,
  at: number,
  payload: SchedulePayload,
): Promise<TickAdmitted> {
  return admitTickAs(executions, trigger, new Date(at).toISOString(), payload, undefined);
}

/**
 * Start the execution a tick's `fired` row names, where it never began — the
 * repair `./sources.ts`'s scheduler asks for at start, and for a claim of its
 * own whose execution the journal would not begin (`docs/durability.md`
 * §3.11). `undefined`, and nothing bound, where the execution did begin: that
 * is every row a `serve` that stopped cleanly leaves.
 */
async function admitUnbegun(
  executions: Map<string, Execution>,
  trigger: ScheduleTrigger,
  fired: FiredRow,
  payload: SchedulePayload,
): Promise<TickAdmitted | undefined> {
  if (await begun(executions, await openJournal(), fired.execution)) return undefined;
  return admitTickAs(executions, trigger, fired.scheduledAt, payload, fired.execution);
}

/**
 * The refusal `runFlow` would raise before journaling anything for a run with
 * no session identity whose flow reaches a `scope: session` store (grammar
 * 11.3), asked here first.
 *
 * A gated admission waits for the lifecycle row and treats a run that never
 * reached it as the journal's failure — retried, holding the schedule or the
 * stream — so nothing the run refuses on its own account may be left to
 * refuse it there: a tick or an event whose `session_key:` answered the empty
 * string would be retried for ever. It is the trigger's binding that failed,
 * and it is refused as one.
 */
function refusesSession(flow: CompiledFlow, sessionKey: string): void {
  if (sessionKey === "" && flow.sessionStores.length > 0) {
    throw new Error(sessionRefusal(flow.address, flow.sessionStores));
  }
}

/** Bind one tick and admit it under the `fired` row for `scheduledAt`. */
async function admitTickAs(
  executions: Map<string, Execution>,
  trigger: ScheduleTrigger,
  scheduledAt: string,
  payload: SchedulePayload,
  execution: string | undefined,
): Promise<TickAdmitted> {
  const flow = flows[trigger.flow];
  if (flow === undefined) {
    return { outcome: "refused", reason: `\`${trigger.flow}\` is not a flow of this composition` };
  }
  const concurrency = trigger.concurrency;
  // Bound as a lone arrival under a debounce, for [`start`]'s reason.
  const bound: SchedulePayload =
    concurrency?.debounceMs === undefined ? payload : { ...payload, coalesced: 1 };
  let inputs: Record<string, unknown>;
  let sessionKey = "";
  let group: string | undefined;
  try {
    if (concurrency?.key !== undefined) group = concurrency.key(payload);
    inputs = trigger.input(bound);
    if (trigger.sessionKey !== undefined) sessionKey = trigger.sessionKey(bound);
    flow.parse(inputs);
    refusesSession(flow, sessionKey);
  } catch (error) {
    return { outcome: "refused", reason: `the trigger \`${trigger.name}\` could not bind this tick: ${message(error)}` };
  }
  const intaking = concurrency === undefined ? undefined : scheduleIntake(trigger, concurrency);
  const admitted = await admit(executions, {
    trigger: trigger.name,
    flow,
    inputs,
    sessionKey,
    gate: { kind: "tick", scheduledAt },
    ...(execution === undefined ? {} : { execution }),
    ...(intaking === undefined
      ? {}
      : { intake: { trigger: intaking, ...(group === undefined ? {} : { group }), payload } }),
  });
  if (admitted.outcome === "started") return { outcome: "started", execution: admitted.record.id };
  if (admitted.outcome === "held") return { outcome: "held", execution: admitted.execution };
  if (admitted.outcome !== "unbegun") return intaken(admitted);
  // A tick's gate is its `fired` row, and [`admit`] carries it back with the
  // failure. Were it ever missing, a rejection is still the safe answer: the
  // scheduler looks at the newest row again before it claims another tick.
  if (admitted.fired === undefined) {
    throw new Error(`its execution ${admitted.execution} could not be journaled: ${admitted.reason}`);
  }
  return { outcome: "unbegun", fired: admitted.fired, reason: admitted.reason };
}

/**
 * Bind one event and admit it under its trigger's dedupe key.
 *
 * An event whose execution the journal would not begin **rejects**: the
 * consumer retries it with backoff and reads no further line until it is
 * admitted, so the source's checkpoint never moves past it, and the retry
 * finds the key naming an execution that exists nowhere and takes it over
 * (`docs/durability.md` §3.12, §3.13).
 */
async function admitEvent(
  executions: Map<string, Execution>,
  trigger: EventTrigger,
  payload: EventPayload,
): Promise<Admitted> {
  const flow = flows[trigger.flow];
  if (flow === undefined) {
    return { outcome: "refused", reason: `\`${trigger.flow}\` is not a flow of this composition` };
  }
  const concurrency = trigger.concurrency;
  // Bound as a lone arrival under a debounce, for [`start`]'s reason.
  const bound: EventPayload =
    concurrency?.debounceMs === undefined ? payload : { ...payload, coalesced: 1 };
  let inputs: Record<string, unknown>;
  let sessionKey = "";
  let key: string;
  let group: string | undefined;
  try {
    key = trigger.dedupeKey(payload);
    if (key === "") throw new Error(EMPTY_DEDUPE_KEY);
    if (concurrency?.key !== undefined) group = concurrency.key(payload);
    inputs = trigger.input(bound);
    if (trigger.sessionKey !== undefined) sessionKey = trigger.sessionKey(bound);
    flow.parse(inputs);
    refusesSession(flow, sessionKey);
  } catch (error) {
    return { outcome: "refused", reason: `the trigger \`${trigger.name}\` could not bind this event: ${message(error)}` };
  }
  const intaking = concurrency === undefined ? undefined : eventIntake(trigger, concurrency);
  const admitted = await admit(executions, {
    trigger: trigger.name,
    flow,
    inputs,
    sessionKey,
    gate: { kind: "dedupe", key, windowMs: trigger.dedupeWindowMs },
    ...(intaking === undefined
      ? {}
      : { intake: { trigger: intaking, ...(group === undefined ? {} : { group }), payload } }),
  });
  if (admitted.outcome === "unbegun") {
    throw new Error(`its execution ${admitted.execution} could not be journaled: ${admitted.reason}`);
  }
  if (admitted.outcome === "started") return { outcome: "started", execution: admitted.record.id };
  if (admitted.outcome === "held") return { outcome: "held", execution: admitted.execution };
  return intaken(admitted);
}

/**
 * A tick's or an event's answer where its intake holds it rather than having
 * started it (PRD resolved q77): accepted all the same — a consumer journals
 * its checkpoint past it, a scheduler claims its next tick — because the row
 * is the journal's, and the intake starts it.
 */
function intaken(
  admitted: Extract<AdmitAnswer, { readonly outcome: "queued" | "pending" | "dropped" }>,
): Admitted {
  if (admitted.outcome === "dropped") {
    return { outcome: "dropped", intake: admitted.intake.id, running: admitted.running };
  }
  if (admitted.outcome === "pending") {
    return {
      outcome: "pending",
      intake: admitted.intake.id,
      coalesced: admitted.intake.coalesced,
      deadline: admitted.intake.deadline ?? "",
    };
  }
  return { outcome: "queued", intake: admitted.intake.id, execution: admitted.intake.execution };
}

// ---------------------------------------------------------------------------
// The intake ledger (PRD resolved q77, Decision D163)
// ---------------------------------------------------------------------------

/** What a repeat inside its dedupe window is answered with (grammar 13.3). */
const DEDUPED_HEADER = "X-AgentCompose-Deduped";

/** …and an arrival its full group dropped (grammar 13.3). */
const DROPPED_HEADER = "X-AgentCompose-Dropped";

/** The `24h` a dedupe key is held for where a trigger declares no window. */
const DEFAULT_DEDUPE_WINDOW_MS = 24 * 60 * 60 * 1000;

/** Why an arrival whose `dedupe_key:` answered `''` is refused (Decision D163). */
const EMPTY_DEDUPE_KEY =
  "its `dedupe_key:` answered the empty string, so there is no key to tell a repeat of it by (grammar 13.3)";

/**
 * How soon a trigger whose queue still holds rows is looked at again.
 *
 * A slot frees when an execution of the group settles or its cancel is
 * recorded, and both are seen here — but a settle another process wrote (a
 * `resume` finishing one), or a start the journal refused, is seen by nothing
 * in this process, so a queue that did not drain is read again on this period
 * until it does.
 */
const DRAIN_RETRY_MS = 2_000;

/**
 * The longest a debounce timer is armed for at once.
 *
 * Nothing bounds `debounce:`, and a runtime timer holds 2³¹−1 ms; a longer
 * wait is stepped, re-reading the row's deadline at each step — which is also
 * what keeps a deadline another arrival moved from firing on the old one.
 */
const LONGEST_TIMER_MS = 60_000;

/** What the ledger needs of a trigger whose `concurrency:` it admits under. */
interface IntakeTrigger {
  readonly name: string;
  readonly flow: CompiledFlow;
  readonly concurrency: Pick<
    Concurrency<unknown>,
    "limit" | "onConflict" | "debounceMs" | "maxRunning"
  >;
  /**
   * Bind one payload into the invocation it starts, throwing where the
   * trigger refuses it — the same `input:`, `session_key:` and `callback:`
   * every admission of the kind binds, held to the flow's `inputs:`.
   */
  bind(payload: unknown): IntakeAdmission;
}

/** The trigger called `name`, where it declares `concurrency:`. */
function intakeTrigger(name: string): IntakeTrigger | undefined {
  const http = httpTriggers.find((one) => one.name === name);
  if (http !== undefined) return httpIntake(http, http.concurrency);
  const schedule = scheduleTriggers.find((one) => one.name === name);
  if (schedule !== undefined) return scheduleIntake(schedule, schedule.concurrency);
  const event = eventTriggers.find((one) => one.name === name);
  if (event !== undefined) return eventIntake(event, event.concurrency);
  return undefined;
}

/** Every trigger this build declares `concurrency:` on. */
function intakeTriggers(): readonly IntakeTrigger[] {
  return [
    ...httpTriggers.map((one) => httpIntake(one, one.concurrency)),
    ...scheduleTriggers.map((one) => scheduleIntake(one, one.concurrency)),
    ...eventTriggers.map((one) => eventIntake(one, one.concurrency)),
  ].filter((trigger): trigger is IntakeTrigger => trigger !== undefined);
}

/**
 * What a trigger this build declares **without** `concurrency:` is read
 * under, for the rows an earlier build's intake accepted for it: no group is
 * ever full, so each queued row starts as soon as it is looked at and a
 * pending one at the deadline it records (Decision D163's Status).
 */
const UNBOUNDED: IntakeTrigger["concurrency"] = {
  limit: Number.POSITIVE_INFINITY,
  onConflict: "queue",
};

/**
 * Every trigger this build declares with no `concurrency:` block, read with
 * no bound ([`UNBOUNDED`]) — the ones a deploy stripped of the block its
 * intake rows were accepted under ([`IntakeLedger.atStart`]).
 */
function unboundedTriggers(): readonly IntakeTrigger[] {
  return [
    ...httpTriggers
      .filter((one) => one.concurrency === undefined)
      .map((one) => httpIntake(one, UNBOUNDED)),
    ...scheduleTriggers
      .filter((one) => one.concurrency === undefined)
      .map((one) => scheduleIntake(one, UNBOUNDED)),
    ...eventTriggers
      .filter((one) => one.concurrency === undefined)
      .map((one) => eventIntake(one, UNBOUNDED)),
  ].filter((trigger): trigger is IntakeTrigger => trigger !== undefined);
}

/** The bound invocation, held to the flow's `inputs:` and its session stores. */
function bindAdmission(
  flow: CompiledFlow,
  inputs: Record<string, unknown>,
  sessionKey: string,
  extra: { readonly callback?: string; readonly traceparent?: string } = {},
): IntakeAdmission {
  flow.parse(inputs);
  refusesSession(flow, sessionKey);
  return { inputs, sessionKey, ...extra };
}

function httpIntake(
  trigger: HttpTrigger,
  concurrency: IntakeTrigger["concurrency"] | undefined,
): IntakeTrigger | undefined {
  const flow = flows[trigger.flow];
  if (concurrency === undefined || flow === undefined) return undefined;
  return {
    name: trigger.name,
    flow,
    concurrency,
    bind: (payload) => {
      const arrived = payload as Payload;
      const inputs = trigger.input(arrived);
      const sessionKey = trigger.sessionKey === undefined ? "" : trigger.sessionKey(arrived);
      const callback = callbackOf(trigger, arrived);
      const traceparent = (arrived.headers?.["traceparent"] ?? "").trim();
      return bindAdmission(flow, inputs, sessionKey, {
        ...(callback === undefined ? {} : { callback }),
        ...(traceparent === "" ? {} : { traceparent }),
      });
    },
  };
}

function scheduleIntake(
  trigger: ScheduleTrigger,
  concurrency: IntakeTrigger["concurrency"] | undefined,
): IntakeTrigger | undefined {
  const flow = flows[trigger.flow];
  if (concurrency === undefined || flow === undefined) return undefined;
  return {
    name: trigger.name,
    flow,
    concurrency,
    bind: (payload) => {
      const tick = payload as SchedulePayload;
      const inputs = trigger.input(tick);
      const sessionKey = trigger.sessionKey === undefined ? "" : trigger.sessionKey(tick);
      return bindAdmission(flow, inputs, sessionKey);
    },
  };
}

function eventIntake(
  trigger: EventTrigger,
  concurrency: IntakeTrigger["concurrency"] | undefined,
): IntakeTrigger | undefined {
  const flow = flows[trigger.flow];
  if (concurrency === undefined || flow === undefined) return undefined;
  return {
    name: trigger.name,
    flow,
    concurrency,
    bind: (payload) => {
      const event = payload as EventPayload;
      const inputs = trigger.input(event);
      const sessionKey = trigger.sessionKey === undefined ? "" : trigger.sessionKey(event);
      return bindAdmission(flow, inputs, sessionKey);
    },
  };
}

/** A fresh execution id — the one spelling every start uses. */
function newExecutionId(): string {
  return `exec_${globalThis.crypto.randomUUID()}`;
}

/** A fresh intake id: an identity of its own, for an arrival with no execution yet. */
function newIntakeId(): string {
  return `int_${globalThis.crypto.randomUUID()}`;
}

/** One line of the serve log about a trigger's intake. */
function sayIntake(trigger: string, line: string): void {
  process.stderr.write(`intake \`${trigger}\`: ${line}\n`);
}

/** How many of a lease's slots one group holds. */
function heldBy(leases: readonly Lease[], group: string | undefined): readonly Lease[] {
  return leases.filter((lease) => lease.group === group);
}

/**
 * Whether a group has a free slot: fewer of its executions hold the lease than
 * its `limit:`, and fewer of the trigger's than its `max_running:` (PRD
 * resolved q77 ruling b). Both counts are of rows ([`Journal.leasesOf`]).
 */
function hasSlot(
  concurrency: IntakeTrigger["concurrency"],
  leases: readonly Lease[],
  group: string | undefined,
): boolean {
  if (heldBy(leases, group).length >= concurrency.limit) return false;
  return concurrency.maxRunning === undefined || leases.length < concurrency.maxRunning;
}

/**
 * Whether a trigger's intake holds anything still owed a start: a queued row,
 * a pending one, or a waited start no lifecycle row followed.
 */
async function holdsOpenRows(journal: Journal, trigger: string): Promise<boolean> {
  return (
    (await journal.intakesOf(trigger, "queued")).length > 0 ||
    (await journal.intakesOf(trigger, "pending")).length > 0 ||
    (await journal.unbegunIntakes(trigger)).length > 0
  );
}

/**
 * The slot a start this process owes holds, read as a lease: the execution
 * its row names, in the row's group ([`IntakeLedger.#leases`]).
 */
function leaseOf(row: IntakeRow): Lease {
  return {
    execution: row.execution,
    ...(row.group === undefined ? {} : { group: row.group }),
    startedAt: row.startedAt ?? row.admittedAt,
  };
}

/** What a group at its limit is answered with: its oldest running execution. */
function oldestOf(leases: readonly Lease[], group: string | undefined): string | undefined {
  return (heldBy(leases, group)[0] ?? leases[0])?.execution;
}

/** Every app's ledger, so a run's settle anywhere frees its trigger's slot. */
const liveLedgers = new Set<IntakeLedger>();

/** The ledger of the app whose executions map this is. */
const ledgers = new WeakMap<Map<string, Execution>, IntakeLedger>();

/**
 * A slot of `trigger`'s lease may have freed: an execution it started
 * settled, or its cancel was recorded. Every live app's ledger looks at its
 * queue — after, never inside, the work that freed it.
 */
function released(trigger: string): void {
  for (const ledger of liveLedgers) ledger.kick(trigger);
}

/**
 * The intake: what admits an arrival under its trigger's `concurrency:` block
 * (PRD resolved q77, Decision D163, `docs/durability.md` §3.15).
 *
 * **A group is a lease held by its running executions**, read off the journal's
 * rows at every decision ([`Journal.leasesOf`]) and never kept as a count: a
 * slot is an execution the trigger started under the block whose lifecycle row
 * is open and whose cancel nobody has requested. So a restart finds the lease
 * the rows hold, and the queue a dead process left starts as those rows free.
 *
 * **One trigger's decisions run one at a time** in this process ([`#locked`]),
 * and each start waits for its lifecycle row before the next decision reads
 * the lease — so two arrivals can never both read a slot free. That is a
 * serialization, not a counter: what is counted is still the rows.
 *
 * Every decision lands on [`admit`]'s seam: the gate a kind keeps — a tick's
 * `fired` row, a dedupe key — is claimed first, under the arrival's own id —
 * the execution it will start, or under a debounce the id its group's pending
 * row records it by once it is counted ([`#debounce`]) — and only then is the
 * group asked.
 */
class IntakeLedger {
  readonly #executions: Map<string, Execution>;
  /** The tail of each trigger's chain of decisions. */
  readonly #locks = new Map<string, Promise<void>>();
  /** The triggers whose rows a dead process left have been looked at. */
  readonly #prepared = new Set<string>();
  /** The armed debounce timers, by intake id. */
  readonly #timers = new Map<string, ReturnType<typeof setTimeout>>();
  /** The armed re-drains, by trigger. */
  readonly #drains = new Map<string, ReturnType<typeof setTimeout>>();
  /**
   * The starts this process owes and could not put back on the queue, by
   * trigger, each with why its execution was not journaled ([`#begin`]).
   */
  readonly #owed = new Map<string, Map<string, string>>();
  /**
   * The triggers this build declares with no `concurrency:` whose rows an
   * earlier build's intake accepted, each read with no bound ([`UNBOUNDED`],
   * [`atStart`]).
   */
  readonly #unbounded = new Map<string, IntakeTrigger>();
  #stopped = false;

  constructor(executions: Map<string, Execution>) {
    this.#executions = executions;
  }

  /** The trigger a row of this ledger's names, as its decisions read it. */
  #triggerOf(name: string): IntakeTrigger | undefined {
    return intakeTrigger(name) ?? this.#unbounded.get(name);
  }

  /**
   * Run one trigger's decision with no other decision of that trigger's
   * between its reads and its writes. A decision that threw is one decision
   * failing, and the chain goes on past it.
   */
  #locked<T>(trigger: string, work: () => Promise<T>): Promise<T> {
    const prior = this.#locks.get(trigger) ?? Promise.resolve();
    const next = prior.then(work, work);
    this.#locks.set(
      trigger,
      next.then(
        () => undefined,
        () => undefined,
      ),
    );
    return next;
  }

  /**
   * After recovery and before anything is admitted (PRD resolved q77 ruling
   * f): every trigger's rows a dead process left are looked at — a start it
   * claimed and never journaled, its queue, its open debounces.
   *
   * **A trigger this build declares without `concurrency:`** may hold rows an
   * earlier build's intake accepted under a block a deploy has since removed:
   * arrivals answered `202` with an intake id, owed a start (ruling f). They
   * are read under no bound ([`UNBOUNDED`]) — the bound the serving build
   * declares — so every queued row starts as it is looked at, and each pending
   * one at the deadline it records, as the rows say and the build now allows
   * (Decision D163's Status). A trigger with none of them is looked at no
   * further.
   */
  async atStart(): Promise<void> {
    for (const trigger of intakeTriggers()) {
      await this.#locked(trigger.name, () => this.#prepare(trigger)).catch((error: unknown) => {
        sayIntake(
          trigger.name,
          `its intake rows could not be read (${message(error)}); they are looked at again before its next admission`,
        );
        this.#redrainLater(trigger.name);
      });
    }
    for (const trigger of unboundedTriggers()) {
      await this.#locked(trigger.name, async () => {
        const journal = await openJournal();
        if (!(await holdsOpenRows(journal, trigger.name))) return;
        this.#unbounded.set(trigger.name, trigger);
        sayIntake(
          trigger.name,
          "this build declares no `concurrency:` on the trigger, so the arrivals an earlier build's intake accepted for it start with no bound",
        );
        await this.#prepare(trigger);
      }).catch((error: unknown) => {
        // Unknown whether it holds any: looked at again, as one that might.
        this.#unbounded.set(trigger.name, trigger);
        sayIntake(
          trigger.name,
          `its intake rows could not be read (${message(error)}); they are looked at again shortly`,
        );
        this.#redrainLater(trigger.name);
      });
    }
  }

  /** No timer is armed and no decision begun after this: the app is closing. */
  stop(): void {
    this.#stopped = true;
    for (const timer of this.#timers.values()) clearTimeout(timer);
    for (const timer of this.#drains.values()) clearTimeout(timer);
    this.#timers.clear();
    this.#drains.clear();
  }

  /**
   * Look at a trigger's queue again: one of its slots may have freed. Never
   * awaited by what freed it, which may be running under the lock itself.
   */
  kick(name: string): void {
    if (this.#stopped) return;
    const trigger = this.#triggerOf(name);
    if (trigger === undefined) return;
    void this.#locked(name, async () => {
      if (this.#stopped) return;
      const journal = await openJournal();
      if (!this.#prepared.has(name)) {
        await this.#prepare(trigger);
        return;
      }
      await this.#drain(journal, trigger);
    }).catch((error: unknown) => {
      sayIntake(name, `its queue could not be read (${message(error)}); it is read again shortly`);
      this.#redrainLater(name);
    });
  }

  /**
   * The rows a dead process left, for one trigger (PRD resolved q77 ruling f,
   * `docs/durability.md` §3.15, §6.3), under its lock:
   *
   *  * a `started` row that **waited** — it was answered with its intake id,
   *    so the intake owes its start — whose execution has no lifecycle row is
   *    a start the process died between claiming and journaling: it starts
   *    under the execution id the row recorded, once, **as the lease allows**.
   *    Its group may have no slot for it now — the execution the dead process
   *    was running is recovered and holds it, or another arrival took the
   *    slot while the start was owed — so a row with no free slot is owed
   *    rather than started, and goes back on the queue at the place it was
   *    admitted in ([`#requeue`]), to start when a slot frees. A row that did
   *    not wait was answered only once its lifecycle row was down, so a row
   *    whose execution never began is a claim nobody was told of, and is taken
   *    back — its arrival's own source re-delivers it;
   *  * each open debounce has its newest arrival's gate folded, and is armed
   *    at its deadline, re-derived from the row — one already past fires at
   *    once. A fold the journal will not take now is the close's to make
   *    first ([`#close`]), so the row is armed all the same;
   *  * the queue drains as the lease the rows hold allows.
   *
   * The trigger counts as looked at only once all three are done: a look the
   * journal cut short is made again, whole, before the trigger's next decision
   * ([`kick`], [`admit`]) — never left with a pending row nothing armed.
   */
  async #prepare(trigger: IntakeTrigger): Promise<void> {
    const journal = await openJournal();
    for (const row of await journal.unbegunIntakes(trigger.name)) {
      if (this.#executions.has(row.execution)) continue;
      if (!row.waited) {
        await journal.abandonIntake(row.id);
        continue;
      }
      // Already owed by this process: the drain puts it back ([`#requeue`]).
      if (this.#owed.get(trigger.name)?.has(row.id) === true) continue;
      if (row.admission === undefined) continue;
      if (!(await this.#stillBinds(journal, trigger, row, row.admission))) continue;
      // Read with the starts this pass already owes, each of which holds its
      // group's slot as a lease would.
      const leases = await this.#leases(journal, trigger);
      if (!hasSlot(trigger.concurrency, leases, row.group)) {
        const detail = `its start was claimed by a stopped serve and not journaled, and its group had no free slot: ${oldestOf(leases, row.group) ?? "its trigger"} held it`;
        this.#owe(trigger.name, row.id, detail);
        continue;
      }
      const answer = await this.#begin(journal, trigger, row.admission, row.execution, row, undefined, true);
      if (answer.outcome === "started") {
        sayIntake(
          trigger.name,
          `${row.id} started ${row.execution}, whose start a stopped serve had claimed and not journaled`,
        );
      }
    }
    for (const row of await journal.intakesOf(trigger.name, "pending")) {
      // A counted arrival whose gate a stopped serve had not yet moved onto
      // the row's execution ([`#debounce`]).
      await this.#fold(journal, row).catch((error: unknown) => {
        sayIntake(
          trigger.name,
          `${row.id}'s newest arrival's gate could not yet be moved onto ${row.execution} (${message(error)}); it is moved when the debounce closes`,
        );
      });
      this.#arm(row);
    }
    await this.#drain(journal, trigger);
    this.#prepared.add(trigger.name);
  }

  /**
   * A trigger's lease as a decision reads it: the rows' ([`Journal.leasesOf`]),
   * and each start this process owes and could not yet put back on the queue
   * ([`#requeue`]) — the slot its arrival was answered for, which no later
   * arrival may take while it waits to be started.
   */
  async #leases(journal: Journal, trigger: IntakeTrigger): Promise<readonly Lease[]> {
    const leases = await journal.leasesOf(trigger.name);
    const owed = this.#owed.get(trigger.name);
    if (owed === undefined || owed.size === 0) return leases;
    const held: Lease[] = [];
    for (const id of owed.keys()) {
      const row = await journal.intake(id);
      if (row !== undefined && row.state === "started" && !leases.some((one) => one.execution === row.execution)) {
        held.push(leaseOf(row));
      }
    }
    return [...leases, ...held];
  }

  /**
   * Admit one arrival under its trigger's `concurrency:` block — [`admit`]'s
   * branch for such a trigger, run under the trigger's lock.
   */
  admit(admission: Admission & { readonly intake: AdmissionIntake }): Promise<AdmitAnswer> {
    const trigger = admission.intake.trigger;
    return this.#locked(trigger.name, async () => {
      const journal = await openJournal();
      if (!this.#prepared.has(trigger.name)) await this.#prepare(trigger);
      const { group, payload } = admission.intake;
      const concurrency = trigger.concurrency;
      // **Every arrival claims its gate under an id of its own** (Decision
      // D163's Status) — the execution it starts, or under a debounce, the id
      // its group's open row records it by once the row has counted it. So a
      // tick's `fired` row and a dedupe row name an id the intake holds
      // exactly where the arrival was admitted: a repeat finds what the first
      // got, and a claim a crash left uncounted names an id nothing holds —
      // which the gate's repair and takeover read as a claim no admission
      // began under, and admit again.
      const id = admission.execution ?? newExecutionId();
      const gate = await claimGate(this.#executions, journal, admission, id);
      if (gate.outcome === "held") return gate;
      const bound: IntakeAdmission = {
        inputs: admission.inputs,
        sessionKey: admission.sessionKey,
        ...(admission.callback === undefined ? {} : { callback: admission.callback }),
        ...(admission.traceparent === undefined ? {} : { traceparent: admission.traceparent }),
      };
      if (concurrency.debounceMs !== undefined) {
        const pending = await journal.pendingIntake(trigger.name, group);
        return this.#debounce(journal, trigger, group, payload, id, pending, admission.gate);
      }
      // The queue first: a slot that freed is the oldest waiting arrival's,
      // never a newcomer's (FIFO per group, PRD resolved q77 ruling b) — and
      // a queue that stopped short of its end keeps every slot it left free
      // for the rows still on it, so the newcomer waits behind them.
      const stalled = await this.#drain(journal, trigger);
      return this.#place(journal, trigger, group, bound, id, gate.fired, undefined, stalled);
    });
  }

  /**
   * Open the group's debounce, or extend the one it has: the newest payload,
   * a deadline `debounce:` from now, one more arrival coalesced (PRD resolved
   * q77 ruling c).
   *
   * **An arrival is counted in the move that records it** (Decision D163's
   * Status, `docs/durability.md` §3.15). Its gate was claimed under `id`, its
   * own; the move that adds it to `coalesced` names `id` and the gate as the
   * row's newest arrival, in the one statement, and only then is the gate's
   * row re-pointed at the execution the row will start ([`#fold`]). So:
   *
   *  * a repeat of a counted arrival finds the row — through the folded gate,
   *    or the record of the newest — and is answered with it, never counted
   *    again;
   *  * a claim a crash left before the move names an id nothing records,
   *    which a repeat's takeover and a tick's repair read as a claim no
   *    admission began under — so the arrival is counted when it comes again,
   *    never lost;
   *  * a crash between the move and the fold leaves the row naming the gate,
   *    and the next move, the close or the next start folds it first.
   */
  async #debounce(
    journal: Journal,
    trigger: IntakeTrigger,
    group: string | undefined,
    payload: unknown,
    id: string,
    pending: IntakeRow | undefined,
    gate: Admission["gate"],
  ): Promise<AdmitAnswer> {
    const debounce = trigger.concurrency.debounceMs ?? 0;
    const now = Date.now();
    const deadline = new Date(now + debounce).toISOString();
    const joined: JoinedArrival | undefined =
      gate === undefined
        ? undefined
        : {
            execution: id,
            gate:
              gate.kind === "tick"
                ? { kind: "tick", scheduledAt: gate.scheduledAt }
                : { kind: "dedupe", key: gate.key },
          };
    if (pending !== undefined) {
      // The arrival the row last counted is folded first: this move overwrites
      // the record of it, and a gate naming an id nothing records any more
      // would read as a claim nobody counted.
      await this.#fold(journal, pending);
      const moved = await journal.moveIntake(pending.id, "pending", {
        state: "pending",
        payload,
        coalesced: pending.coalesced + 1,
        deadline,
        ...(joined === undefined ? {} : { joined }),
      });
      const row = moved ? await journal.intake(pending.id) : undefined;
      if (row !== undefined) {
        // …and this one, now that it is counted. One that cannot be folded now
        // is folded by the next move, the close or the next start: the row
        // records it, which is what a repeat of it reads meanwhile.
        await this.#fold(journal, row).catch((error: unknown) => {
          sayIntake(
            trigger.name,
            `${row.id} counted an arrival whose gate could not yet be moved onto ${row.execution} (${message(error)}); it is moved when the debounce next moves`,
          );
        });
        this.#arm(row);
        return { outcome: "pending", intake: row };
      }
    }
    // None open, or it closed under this decision: the arrival opens one,
    // which starts under the id its gate was claimed under — nothing to fold.
    const row = await journal.admitIntake({
      id: newIntakeId(),
      trigger: trigger.name,
      ...(group === undefined ? {} : { group }),
      state: "pending",
      execution: id,
      waited: true,
      payload,
      coalesced: 1,
      admittedAt: new Date(now).toISOString(),
      deadline,
    });
    this.#arm(row);
    return { outcome: "pending", intake: row };
  }

  /**
   * Decide one arrival against the lease, or one debounce at its deadline
   * (`waiting`): start it where its group has a free slot, else do what
   * `on_conflict:` says (PRD resolved q77 ruling b).
   */
  async #place(
    journal: Journal,
    trigger: IntakeTrigger,
    group: string | undefined,
    bound: IntakeAdmission,
    id: string,
    fired: FiredRow | undefined,
    waiting: IntakeRow | undefined,
    behind: boolean,
  ): Promise<AdmitAnswer> {
    const concurrency = trigger.concurrency;
    let leases = await journal.leasesOf(trigger.name);
    // **Behind a stalled queue, an arrival queues** — whatever `on_conflict:`
    // says, and whether or not its group has a slot: the drain before it
    // stopped at a row the journal would not begin, or still owes a start it
    // could not put back on the queue ([`#drain`]), so a slot it left free
    // belongs to those older arrivals, which start first once the journal
    // takes them.
    if (behind) return this.#enqueue(journal, trigger, group, bound, id, waiting, leases, true);
    if (hasSlot(concurrency, leases, group)) {
      return this.#begin(journal, trigger, bound, id, waiting, fired, false, group);
    }
    if (concurrency.onConflict === "supersede") {
      // **The group's running executions are cancelled, through the cancel
      // every execution has** (PRD resolved q75): the request is recorded, the
      // controller fires, and the cascade reaches their children. Each one's
      // slot is released the moment its cancel is recorded — the lease is of
      // executions nobody has asked to cancel — so the newest arrival starts
      // without waiting for them to settle (Decision D163's Status).
      for (const lease of heldBy(leases, group)) {
        await this.#cancel(lease.execution);
        sayIntake(trigger.name, `superseded ${lease.execution} for ${id}`);
      }
      leases = await journal.leasesOf(trigger.name);
      if (hasSlot(concurrency, leases, group)) {
        return this.#begin(journal, trigger, bound, id, waiting, fired, false, group);
      }
      // Still no slot: the group had none of its own to give, and the
      // trigger's `max_running:` is held by its other groups' executions,
      // which a supersede never cancels. The arrival waits for one.
    }
    if (concurrency.onConflict === "drop") {
      const running = oldestOf(leases, group) ?? "";
      const detail = `its group had no free slot: ${running} is running it`;
      const row =
        waiting === undefined
          ? await journal.admitIntake({
              id: newIntakeId(),
              trigger: trigger.name,
              ...(group === undefined ? {} : { group }),
              state: "dropped",
              execution: id,
              waited: true,
              heldBy: running,
              admission: bound,
              coalesced: 1,
              admittedAt: new Date().toISOString(),
              detail,
            })
          : await this.#moved(journal, waiting, {
              state: "dropped",
              heldBy: running,
              admission: bound,
              detail,
            });
      sayIntake(trigger.name, `${row.id} dropped: ${detail}`);
      return { outcome: "dropped", intake: row, running };
    }
    return this.#enqueue(journal, trigger, group, bound, id, waiting, leases, false);
  }

  /**
   * Put one arrival on its trigger's queue — written for an arrival that
   * never waited, moved for a debounce that closed ([`#place`]) — saying what
   * it waits for: the execution holding its group, or, `behind` a queue that
   * stalled, the older arrivals still on it.
   */
  async #enqueue(
    journal: Journal,
    trigger: IntakeTrigger,
    group: string | undefined,
    bound: IntakeAdmission,
    id: string,
    waiting: IntakeRow | undefined,
    leases: readonly Lease[],
    behind: boolean,
  ): Promise<AdmitAnswer> {
    const row =
      waiting === undefined
        ? await journal.admitIntake({
            id: newIntakeId(),
            trigger: trigger.name,
            ...(group === undefined ? {} : { group }),
            state: "queued",
            execution: id,
            waited: true,
            admission: bound,
            coalesced: 1,
            admittedAt: new Date().toISOString(),
          })
        : await this.#moved(journal, waiting, { state: "queued", admission: bound });
    sayIntake(
      trigger.name,
      behind
        ? `${row.id} queued for ${row.execution} behind the arrivals its trigger's queue has not yet started`
        : `${row.id} queued for ${row.execution}: ${oldestOf(leases, group) ?? "its group"} holds ${group === undefined ? "the trigger's group" : `the group ${JSON.stringify(group)}`}`,
    );
    this.#redrainLater(trigger.name);
    return { outcome: "queued", intake: row };
  }

  /**
   * Whether a waiting row's recorded admission still binds under the flow
   * this build serves — held to its `inputs:` and its session stores, as
   * every arrival is before it is admitted ([`bindAdmission`]) — and where it
   * does not, the row dropped, naming why (Decision D163's Status).
   *
   * A row is bound when it arrives, by the build that admitted it, and may be
   * started by another: a deploy that narrows the flow's `inputs:`, or gives
   * it a `scope: session` store a row's empty session key cannot reach,
   * leaves rows `runFlow` would refuse before their lifecycle row. [`#begin`]
   * reads every start that never reached that row as the journal's failure
   * and puts a waited row back on the queue — which, for a refusal no retry
   * can answer, is a row tried first at every look for ever, holding every
   * row behind it. So it is asked here, first, as a debounce's close binds its
   * payload again ([`#close`]); a row that no longer binds is dropped as one
   * is there, and the queue moves on past it.
   */
  async #stillBinds(
    journal: Journal,
    trigger: IntakeTrigger,
    row: IntakeRow,
    admission: IntakeAdmission,
  ): Promise<boolean> {
    try {
      bindAdmission(trigger.flow, admission.inputs, admission.sessionKey);
      return true;
    } catch (error) {
      const detail = `its admission no longer binds under this build: ${message(error)}`;
      if (await journal.moveIntake(row.id, row.state, { state: "dropped", detail })) {
        sayIntake(trigger.name, `${row.id} dropped: ${detail}`);
      }
      return false;
    }
  }

  /**
   * Re-point the gate of the arrival `row` counted last at the execution the
   * row starts ([`Journal.foldGate`]). Nothing where the row records none, or
   * where that arrival opened the row and so claimed its gate under the row's
   * own id.
   */
  async #fold(journal: Journal, row: IntakeRow): Promise<void> {
    const joined = row.joined;
    if (joined === undefined || joined.execution === row.execution) return;
    await journal.foldGate(row.trigger, joined.gate, joined.execution, row.execution);
  }

  /** Move a waiting row on, answering what it became. */
  async #moved(journal: Journal, row: IntakeRow, change: IntakeChange): Promise<IntakeRow> {
    if (!(await journal.moveIntake(row.id, row.state, change))) {
      throw new Error(`the intake row ${row.id} moved under this decision`);
    }
    return (await journal.intake(row.id)) ?? row;
  }

  /**
   * Start one execution under the lease: claim it on its intake row —
   * inserted for an arrival that never waited, moved for one that did — then
   * register it and wait for its lifecycle row, which is what makes the slot
   * one the next decision reads as held.
   *
   * **The order, and what a crash leaves** (Decision D163): the row says
   * `started` before the execution is registered, exactly as a tick's `fired`
   * row is claimed first (D162). A process that dies between the two leaves a
   * row naming an execution with no lifecycle row — which holds no slot, since
   * a slot is an open lifecycle row — and the next start repairs it once
   * ([`#prepare`]): a row that waited starts under the id it recorded, a row
   * that did not is taken back. A start the journal would not begin is
   * undone the same two ways here and now.
   */
  async #begin(
    journal: Journal,
    trigger: IntakeTrigger,
    bound: IntakeAdmission,
    id: string,
    waiting: IntakeRow | undefined,
    fired: FiredRow | undefined,
    repairing: boolean,
    group?: string,
  ): Promise<AdmitAnswer> {
    const startedAt = new Date().toISOString();
    let row: IntakeRow;
    if (repairing && waiting !== undefined) {
      row = waiting;
    } else if (waiting !== undefined) {
      row = await this.#moved(journal, waiting, { state: "started", admission: bound, startedAt });
    } else {
      row = await journal.admitIntake({
        id: newIntakeId(),
        trigger: trigger.name,
        ...(group === undefined ? {} : { group }),
        state: "started",
        execution: id,
        waited: false,
        admission: bound,
        coalesced: 1,
        admittedAt: startedAt,
        startedAt,
      });
    }
    const { record, begun } = register(
      this.#executions,
      {
        trigger: trigger.name,
        flow: trigger.flow,
        inputs: bound.inputs,
        sessionKey: bound.sessionKey,
        ...(bound.callback === undefined ? {} : { callback: bound.callback }),
        ...(bound.traceparent === undefined ? {} : { traceparent: bound.traceparent }),
      },
      id,
    );
    try {
      await begun;
    } catch (error) {
      if (this.#executions.get(id) === record) this.#executions.delete(id);
      const reason = message(error);
      if (row.waited) {
        // Owed: the arrival was answered with its intake id, so the row goes
        // back on the queue and is started again as a slot allows. A journal
        // that will not take that move either leaves the row `started`, naming
        // an execution that never began — which no drain reads, since a drain
        // reads the queue — so the start is remembered as owed, and every drain
        // of the trigger tries the move again until it lands ([`#requeue`]);
        // the next start of `serve` finds the row either way ([`#prepare`]).
        const detail = `its execution could not be journaled: ${reason}`;
        const requeued = await journal
          .moveIntake(row.id, "started", { state: "queued", detail })
          .catch(() => false);
        if (!requeued) this.#owe(trigger.name, row.id, detail);
        sayIntake(
          trigger.name,
          `${row.id}'s execution ${id} could not be journaled (${reason}); it waits on the queue again`,
        );
        this.#redrainLater(trigger.name);
      } else {
        await journal.abandonIntake(row.id).catch(() => undefined);
      }
      return { outcome: "unbegun", execution: id, ...(fired === undefined ? {} : { fired }), reason };
    }
    if (row.waited && !repairing) {
      sayIntake(
        trigger.name,
        waiting?.state === "pending"
          ? `${row.id} closed over ${row.coalesced} arrival${row.coalesced === 1 ? "" : "s"} and started ${id}`
          : `${row.id} started ${id}`,
      );
    }
    return { outcome: "started", record };
  }

  /**
   * Start what a trigger's queue holds as its lease allows, oldest first —
   * a group's rows in the order they were admitted, a row whose group is
   * full passed over for a later one whose group is not, and nothing at all
   * while the trigger is at its `max_running:` — answering whether it
   * **stalled**: stopped at a row the journal would not begin, with the rows
   * after it not yet looked at, or left a start this process owes and could
   * not put back on the queue ([`#requeue`]), which is a row ahead of any
   * newcomer too.
   *
   * A row whose recorded admission no longer binds under this build is
   * dropped, naming why, and the drain goes on past it ([`#stillBinds`]):
   * only a start the journal itself would not take stops it, since the next
   * start would meet the same journal.
   *
   * **A start still owed holds its group's slot** ([`#leases`]): its arrival
   * was answered with its intake id and its turn came, so a slot it has not
   * yet been started into is its own — a later row of its group, or one that
   * would take the trigger's last `max_running:`, waits for it rather than
   * running beside the execution it will be.
   */
  async #drain(journal: Journal, trigger: IntakeTrigger): Promise<boolean> {
    const owed = await this.#requeue(journal, trigger);
    const queued = await journal.intakesOf(trigger.name, "queued");
    let waiting = queued.length;
    let stalled = false;
    for (const row of queued) {
      if (this.#stopped) return false;
      const leases = await this.#leases(journal, trigger);
      const max = trigger.concurrency.maxRunning;
      if (max !== undefined && leases.length >= max) break;
      if (!hasSlot(trigger.concurrency, leases, row.group)) continue;
      if (row.admission === undefined) continue;
      if (!(await this.#stillBinds(journal, trigger, row, row.admission))) {
        waiting -= 1;
        continue;
      }
      const answer = await this.#begin(journal, trigger, row.admission, row.execution, row, undefined, false, row.group);
      if (answer.outcome !== "started") {
        stalled = true;
        break;
      }
      waiting -= 1;
    }
    if (waiting > 0 || owed || this.#owed.has(trigger.name)) this.#redrainLater(trigger.name);
    return stalled || owed;
  }

  /** Remember a start this process owes and could not put back on the queue. */
  #owe(name: string, id: string, detail: string): void {
    const owed = this.#owed.get(name) ?? new Map<string, string>();
    owed.set(id, detail);
    this.#owed.set(name, owed);
  }

  /**
   * Put each start this process owes back on the queue ([`#owe`]), where the
   * row still names an execution that never began — answering whether any is
   * still owed. A row that has moved on, or whose execution the journal holds
   * after all, is owed nothing. One the journal will not move yet stays owed,
   * holding its slot ([`#leases`]), and the rest of the queue is looked at
   * past it.
   */
  async #requeue(journal: Journal, trigger: IntakeTrigger): Promise<boolean> {
    const owed = this.#owed.get(trigger.name);
    if (owed === undefined) return false;
    for (const [id, detail] of [...owed]) {
      const row = await journal.intake(id);
      if (
        row === undefined ||
        row.state !== "started" ||
        this.#executions.has(row.execution) ||
        (await journal.execution(row.execution)) !== undefined
      ) {
        owed.delete(id);
        continue;
      }
      const moved = await journal
        .moveIntake(id, "started", { state: "queued", detail })
        .catch(() => false);
      if (moved) {
        owed.delete(id);
        sayIntake(trigger.name, `${id} is back on the queue: ${detail}`);
      }
    }
    if (owed.size > 0) return true;
    this.#owed.delete(trigger.name);
    return false;
  }

  /** Look at a trigger's queue again in [`DRAIN_RETRY_MS`], once. */
  #redrainLater(name: string): void {
    if (this.#stopped || this.#drains.has(name)) return;
    const timer = setTimeout(() => {
      this.#drains.delete(name);
      this.kick(name);
    }, DRAIN_RETRY_MS);
    timer.unref?.();
    this.#drains.set(name, timer);
  }

  /** Arm one open debounce at its deadline, re-read when it falls due. */
  #arm(row: IntakeRow): void {
    if (this.#stopped || row.deadline === undefined) return;
    const held = this.#timers.get(row.id);
    if (held !== undefined) clearTimeout(held);
    const wait = Math.max(0, Math.min(Date.parse(row.deadline) - Date.now(), LONGEST_TIMER_MS));
    const timer = setTimeout(() => {
      this.#timers.delete(row.id);
      void this.#locked(row.trigger, () => this.#close(row.id)).catch((error: unknown) => {
        sayIntake(row.trigger, `${row.id}'s debounce could not be closed (${message(error)}); it is tried again`);
        if (!this.#stopped) {
          const retry = setTimeout(() => this.#arm(row), 1_000);
          retry.unref?.();
        }
      });
    }, wait);
    timer.unref?.();
    this.#timers.set(row.id, timer);
  }

  /**
   * Close one debounce at its deadline (PRD resolved q77 ruling c): bind its
   * newest payload with `payload.coalesced` — the arrivals it closed over —
   * and decide that one execution against the lease, as any arrival is.
   */
  async #close(id: string): Promise<void> {
    if (this.#stopped) return;
    const journal = await openJournal();
    const row = await journal.intake(id);
    if (row === undefined || row.state !== "pending" || row.deadline === undefined) return;
    if (Date.parse(row.deadline) > Date.now()) {
      this.#arm(row);
      return;
    }
    const trigger = this.#triggerOf(row.trigger);
    if (trigger === undefined) {
      sayIntake(row.trigger, `${row.id} stays pending: this build declares no trigger of that name`);
      return;
    }
    // Every counted arrival's gate names the execution about to be decided
    // before it is: a repeat of one after this is a repeat of that execution.
    await this.#fold(journal, row);
    let bound: IntakeAdmission;
    try {
      bound = trigger.bind({ ...(row.payload as Record<string, unknown>), coalesced: row.coalesced });
    } catch (error) {
      const detail = `its payload could not be bound when the debounce closed: ${message(error)}`;
      await journal.moveIntake(row.id, "pending", { state: "dropped", detail });
      sayIntake(trigger.name, `${row.id} dropped: ${detail}`);
      return;
    }
    const stalled = await this.#drain(journal, trigger);
    await this.#place(journal, trigger, row.group, bound, row.execution, undefined, row, stalled);
  }

  /**
   * Cancel one running execution of a group being superseded, through the
   * cancel route's own steps: marked first, recorded, fired, cascaded
   * (`runtime.cancelExecution`, Decision D160).
   */
  async #cancel(execution: string): Promise<void> {
    const record = this.#executions.get(execution);
    const was = record?.cancelling ?? false;
    if (record !== undefined) record.cancelling = true;
    try {
      await cancelExecution(execution);
    } catch (error) {
      if (record !== undefined && !cancelFired(execution)) record.cancelling = was;
      throw error;
    }
  }
}

/** The intake an admission arrives with (see [`Admission.intake`]). */
interface AdmissionIntake {
  readonly trigger: IntakeTrigger;
  /** `concurrency.key`'s answer; absent for the key-less group. */
  readonly group?: string;
  /** The payload as it arrived — what a debounce holds and binds again. */
  readonly payload: unknown;
}

/**
 * What `GET /intakes/:id` answers (PRD resolved q77 ruling d): the row's state
 * and, per state, what a caller holding the id needs next.
 */
function intakeReport(row: IntakeRow): Record<string, unknown> {
  return {
    intake_id: row.id,
    trigger: row.trigger,
    state: row.state,
    admitted_at: row.admittedAt,
    coalesced: row.coalesced,
    ...(row.group === undefined ? {} : { group: row.group }),
    ...(row.state === "pending" && row.deadline !== undefined ? { deadline: row.deadline } : {}),
    ...(row.state === "started"
      ? {
          execution_id: row.execution,
          status_url: `/executions/${row.execution}`,
          ...(row.startedAt === undefined ? {} : { started_at: row.startedAt }),
        }
      : {}),
    ...(row.state === "dropped"
      ? {
          ...(row.heldBy === undefined || row.heldBy === "" ? {} : { running_execution_id: row.heldBy }),
          ...(row.detail === undefined ? {} : { detail: row.detail }),
        }
      : {}),
  };
}

/** A queued or pending arrival, answered with its own id (PRD resolved q77 ruling d). */
function intakeAccepted(reply: FastifyReply, row: IntakeRow): unknown {
  return reply.code(202).send({
    intake_id: row.id,
    status: row.state,
    ...(row.state === "pending" && row.deadline !== undefined
      ? { deadline: row.deadline, coalesced: row.coalesced }
      : {}),
    status_url: `/intakes/${row.id}`,
  });
}

/**
 * The refusal `GET /intakes/:id` has for this request: the `auth:` of the
 * trigger whose arrival the row is, exactly as an execution's routes enforce
 * the trigger that started it ([`guarded`], grammar 13.3, PRD resolved q32).
 */
function intakeGuard(row: IntakeRow, request: FastifyRequest): Refusal | undefined {
  return guardedBy(row.trigger, request);
}

/**
 * Put every execution this project's journal holds open back on the board
 * (PRD resolved q28, `docs/durability.md` §6.1).
 *
 * "**`serve` auto-recovers**: on process start it replays every execution the
 * journal holds open, including executions parked on `human` waits, which
 * re-park with their wait ids intact." That is the whole of what happens here:
 * each open execution is re-run under `resume: true`, so its recorded effects
 * are consumed rather than re-issued and the run arrives back at the pause it
 * was holding — under the same wait id, because a wait id is the node's
 * instance path (grammar 9.4) and nothing about it depends on the process.
 *
 * Two things it deliberately does not do. It does not **re-fire triggers**: the
 * lifecycle row records which trigger started an execution and nothing here
 * reads it as an instruction, so a recovered `http` execution is the one that
 * existed and never a second one (resolved q28). And it does not **wait** for
 * the replays to finish — the executions it recovers are, by definition, ones
 * that were still running, and the commonest of them is parked on a question
 * nobody has answered yet. Registering them is what has to happen before the
 * first request; finishing them is what the resume route is for.
 *
 * A replay that **diverges** (`runtime.ReplayDivergence`) is reported by the
 * status route like any other failed replay, and recovery of one execution never
 * stops the process from serving the others — but the execution's **journal row
 * stays open** (`docs/durability.md` §7). The distinction is the point: what
 * this process reports is what this build saw, while the row records the
 * execution, and a build whose composition has moved under a journal has not
 * decided anything about the executions that journal holds. Put the composition
 * back and the next start recovers them.
 *
 * The deliveries are picked up **beside** the executions and never behind them
 * (`docs/durability.md` §6.1, and [`resumeDeliveries`] for why they are two
 * things): a journal read that throws is transient — the open path waits out a
 * lock and breaks a stale one — so a start that let one failure carry the other
 * would leave every `pending` row unattempted for the life of the process while
 * the file it could not read a moment ago answers every other query.
 *
 * **A start that cannot read the journal owes the recovery, and pays it the
 * first time the journal opens.** A second `serve` on a dialled journal is the
 * case that makes this more than a transient: the writer guard refuses it the
 * journal for as long as the first holds it (`docs/durability.md` §2.3), and
 * when the first goes, this one's scheduler and consumers start succeeding at
 * the journal (`docs/distributed.md` §8). What the first left open — an
 * execution an event or a tick had started, a run parked on a wait — is only
 * ever put back on a board by this walk, and it ran once, at a start that
 * could not read it. So a half that could not read what it walks stays owed:
 * it is taken again with backoff, and again before every tick and every event
 * this process admits ([`before`], awaited by [`intake`]), so those are never
 * matched against an execution the journal holds open and nothing runs — a
 * repeat answered `held` by a dead hub's execution, a tick repaired as begun
 * — and the ordering ruling a gives a start holds for a takeover too.
 */
class Recovery {
  readonly #executions: Map<string, Execution>;
  /** Whether [`recoverExecutions`] has read the open executions and walked them. */
  #executionsRead = false;
  /** Whether [`resumeDeliveries`] has read the owed deliveries and walked them. */
  #deliveriesRead = false;
  /** Why the newest attempt could not read the journal, for [`before`]'s refusal. */
  #failure = "";
  /** The attempt under way, which a tick, an event and a retry share. */
  #attempt: Promise<boolean> | undefined;
  #timer: ReturnType<typeof setTimeout> | undefined;
  #stopped = false;

  constructor(executions: Map<string, Execution>) {
    this.#executions = executions;
  }

  /** Whether both halves have read what they walk. */
  get done(): boolean {
    return this.#executionsRead && this.#deliveriesRead;
  }

  /**
   * The recovery `onReady` runs before the server accepts a connection — and,
   * where it could not read the journal, the retries that finish it once the
   * journal opens.
   */
  async atStart(): Promise<void> {
    await this.#once(false);
    if (!this.done) this.#retryIn(1_000);
  }

  /**
   * Resolve once the open executions are back on the board — taking the walk
   * again first, where it is still owed — and reject, saying why, where the
   * journal still cannot be read. Awaited before every tick and every event is
   * admitted, so the first time the journal opens in a process whose start
   * could not read it, the executions it holds open are recovered before the
   * scheduler claims a tick or a consumer matches an event against them. The
   * deliveries half is not waited for: no tick or event is answered by one,
   * and the retries pick it up.
   */
  async before(): Promise<void> {
    if (this.#executionsRead) return;
    await this.#once(true);
    if (this.#executionsRead) return;
    throw new Error(
      `the executions this project's journal holds open are not recovered yet, and no tick or event is admitted before they are (\`docs/durability.md\` §6.1): ${this.#failure}`,
    );
  }

  /** No retry after this: the app is closing. */
  stop(): void {
    this.#stopped = true;
    if (this.#timer !== undefined) clearTimeout(this.#timer);
    this.#timer = undefined;
  }

  /**
   * One attempt at whatever is still owed, shared by everyone who asks while it
   * runs. `late` is every attempt but the start's: it says nothing about a
   * journal that still cannot be read — the start already did, and a tick
   * that cannot be admitted says so itself — and names the recovery when one
   * runs, since nothing else in the serve log would say why executions are
   * being recovered by a process that has been serving for a while.
   */
  #once(late: boolean): Promise<boolean> {
    if (this.done) return Promise.resolve(true);
    this.#attempt ??= (async () => {
      try {
        if (!this.#executionsRead) {
          const failure = await recoverExecutions(this.#executions, late);
          this.#executionsRead = failure === undefined;
          if (failure !== undefined) this.#failure = failure;
        }
        if (!this.#deliveriesRead) {
          const failure = await resumeDeliveries(late);
          this.#deliveriesRead = failure === undefined;
          if (failure !== undefined) this.#failure = failure;
        }
      } catch (error) {
        // Neither half throws for a journal it cannot read; this is anything
        // else, and it leaves the halves that did not finish owed.
        this.#failure = message(error);
      } finally {
        this.#attempt = undefined;
      }
      return this.done;
    })();
    return this.#attempt;
  }

  /** Take the recovery again in `delay` milliseconds, backing off while it fails. */
  #retryIn(delay: number): void {
    if (this.#stopped || this.done) return;
    this.#timer = setTimeout(() => {
      this.#timer = undefined;
      if (this.#stopped) return;
      void this.#once(true).then((done) => {
        if (!done) this.#retryIn(Math.min(delay * 2, LONGEST_RECOVERY_RETRY_MS));
      });
    }, delay);
    this.#timer.unref?.();
  }
}

/** The longest a recovery the start could not finish waits between attempts. */
const LONGEST_RECOVERY_RETRY_MS = 30_000;

/**
 * The half of [`Recovery`] that puts open executions back on the board:
 * `undefined` once it has read and walked them, and what stopped it where the
 * journal could not be read.
 */
async function recoverExecutions(
  executions: Map<string, Execution>,
  late: boolean,
): Promise<string | undefined> {
  let open: readonly runtime.ExecutionRow[];
  try {
    open = await openExecutions();
  } catch (error) {
    // A journal that cannot be opened is a project that cannot recover, and it
    // is not a reason to refuse to serve: the app starts, new executions
    // journal (or fail loudly when they cannot), and this says what happened.
    if (!late) process.stderr.write(`this project's journal could not be read: ${message(error)}\n`);
    return message(error);
  }
  if (late) {
    process.stderr.write(
      `this project's journal can be read now, so the ${open.length} execution${open.length === 1 ? "" : "s"} it holds open ${open.length === 1 ? "is" : "are"} recovered now, before any tick or event is admitted\n`,
    );
  }
  const stillOpen = new Set(open.map((row) => row.id));
  for (const row of open) {
    // One generation of one execution per process. At start nothing can be
    // running yet — this hook is what runs before the first connection — so the
    // guard is a statement there; a recovery the start could not finish runs in
    // a process that has been serving since, and there it is a fix: an
    // execution this process started, or is already replaying, is never
    // replayed a second time beside itself.
    if (executions.has(row.id)) continue;
    const flow = flows[row.flow];
    if (flow === undefined) {
      // The composition moved under a journal that still holds an execution of
      // a flow it no longer declares. Said rather than crashed, and left open:
      // a reader who puts the flow back can still resume it.
      process.stderr.write(
        `\`${row.id}\` was running \`${row.flow}\`, which this build does not declare: it stays open in the journal\n`,
      );
      continue;
    }
    // A **child execution** (PRD resolved q65) is recovered one of two ways,
    // decided by its parent's row (`docs/durability.md` §6.1).
    //
    // Where the parent is **open** too, the parent's replay is what resumes it:
    // the replay reaches the detached dispatch again and re-issues it, and the
    // dispatch finds the child's open row and resumes it — admitted behind that
    // node's join, under its bounds, exactly as the first generation ran it.
    // Resuming it here as well would put it in front of the parent's own joined
    // instances in that node's permit queue, delaying the parent on work grammar
    // 8.6 rule 7 says it never waits for.
    //
    // Where the parent has **settled** — the crash the ruling was ratified from
    // — nothing will re-issue it, so it is resumed natively, admitted under the
    // bound its lineage recorded (`runtime.resumeChild`): a fan-out stopped with
    // one child running and three still queued restarts one at a time, not four
    // at once. Either way its own runner puts it on this map ([`trackChild`]); it
    // announced no pauses, because it holds none (Decision D118), and it has no
    // callback of its own to owe.
    if (row.lineage !== undefined) {
      if (stillOpen.has(row.lineage.parent)) {
        process.stderr.write(
          `recovered ${row.id} (${row.flow}), for its open parent ${row.lineage.parent} to re-issue\n`,
        );
        continue;
      }
      void resumeChild(row);
      process.stderr.write(`recovered ${row.id} (${row.flow})\n`);
      continue;
    }
    // The pauses this execution's earlier generations already announced, read
    // **before** the replay starts so the re-parking cannot race the knowledge
    // of it: a recovered execution re-parks under the same wait ids, and a
    // parking webhook for a question already asked would tell a receiver
    // something happened that did not (PRD resolved q35).
    resumeInto(executions, flow, row, await announced(row.id));
    process.stderr.write(`recovered ${row.id} (${row.flow})\n`);
  }
  return undefined;
}

/**
 * The wait ids this execution's `parked` deliveries have already reported.
 *
 * A journal that cannot be read answers with none, which is the conservative
 * half: a receiver may get a parking webhook it has already seen — dedupe on
 * the delivery id is the contract (resolved q35) — rather than never getting
 * one for a question that is really open.
 */
async function announced(execution: string): Promise<Set<string>> {
  try {
    const held = await deliveriesOf(execution);
    return new Set(held.flatMap((record) => [...record.pauses]));
  } catch {
    return new Set();
  }
}

/** Re-run one journaled execution in this process, and track it like any other. */
function resumeInto(
  executions: Map<string, Execution>,
  flow: CompiledFlow,
  row: runtime.ExecutionRow,
  reported: Set<string>,
): Execution {
  const execution: Execution = {
    id: row.id,
    flow: flow.address,
    trigger: row.trigger,
    status: "running",
    // Until the replay stops. See [`Execution.recovering`] for what turns on the
    // window and [`stillReplayingTo`] for what it decides.
    recovering: true,
    parked: false,
    // A recovered row whose cancel a dead process had accepted is one the
    // replay completes (`runtime.completeCancel`), and it says so here too.
    cancelling: row.cancelRequestedAt !== undefined,
    // The webhook the caller who started this execution is still waiting for.
    // It is on the lifecycle row because *this* process is the one that will
    // finish the run, and the request that named the URL reached the one that
    // did not (`docs/durability.md` §6.1). A recovered execution that fired no
    // webhook would leave a caller who was handed a `202` with no signal at all
    // — the contract was push, so nobody is polling the status route.
    ...(row.callback === undefined ? {} : { callback: row.callback }),
    reported,
    deliveries: Promise.resolve(),
    announced: false,
    settled: Promise.resolve(),
  };
  // The replay has stopped: whatever it did or did not reach, nothing more is
  // coming, so an answer that misses now misses for a reason of its own and the
  // resume route stops saying "send it again".
  //
  // Watched rather than awaited, because recovery must not wait for the replays
  // it starts (`docs/durability.md` §6.1) — the commonest execution it recovers
  // is parked on a question nobody has answered, so awaiting one would be
  // awaiting the person. The subscription is dropped when the run ends for the
  // reason [`watchHumanPauses`] gives: a `serve` that has recovered many
  // executions holds no listener per finished one.
  const unwatch = watchPauses(execution);
  execution.settled = settling(
    execution,
    runFlow(flow.address, row.inputs, {
      executionId: row.id,
      sessionKey: row.sessionKey,
      resumable: true,
      trigger: row.trigger,
      resume: true,
      // The `settled` webhook, journaled before this generation closes the row
      // it recovered — see [`closed`]. A recovered execution owes the same push
      // as any other and is the one most likely to be interrupted again.
      closing: (produced, error) => closed(execution, produced, error),
    }),
  )
    .then(() => {
      execution.recovering = false;
    })
    .finally(unwatch);
  executions.set(row.id, execution);
  return execution;
}

/**
 * Run one child execution in this app, tracked like any execution it serves —
 * the runner [`createApp`] hosts for the whole of its life (PRD resolved q65).
 *
 * A child is an execution in every way this app's surfaces can see. It is on
 * [`createApp`]'s map under its own id, so `GET /executions/:id` answers it with
 * its own report, guarded by the `auth:` of the trigger its lifecycle row names
 * — its parent's (`runtime.ChildExecution.trigger`). Its row closes through
 * [`closed`], so its trace export is journaled on **its own** ledger before the
 * row closes, exactly as a request's is, headed by its lineage
 * (`./delivery.ts`'s `shipTrace` reads that off the row). And nothing about it
 * reaches its parent's entry: the parent's report and its webhooks are what
 * they were.
 *
 * What it does **not** have is a callback or a pause: no request asked to be
 * told about it, and Decision D118 refuses the `human` node it would park on —
 * so no parking watch is taken, and [`closed`]'s webhook half is the no-op it
 * is for any execution nobody subscribed to.
 *
 * Answers the run itself, so what stopped a child that did not complete reaches
 * the one line the child machinery writes about it (`runtime.dispatchChild`) as
 * well as this app's report.
 */
function trackChild(
  executions: Map<string, Execution>,
  child: runtime.ChildExecution,
): Promise<FlowRun> {
  const execution: Execution = {
    id: child.id,
    flow: child.flow,
    trigger: child.trigger,
    status: "running",
    // A resumed child is a replay on its way back to its frontier, exactly as a
    // recovered execution is; it holds no pause for the window to be about.
    recovering: child.resume,
    parked: false,
    cancelling: false,
    reported: new Set<string>(),
    deliveries: Promise.resolve(),
    announced: false,
    settled: Promise.resolve(),
  };
  const run = runChildFlow(child, (produced, error) => closed(execution, produced, error));
  execution.settled = settling(execution, run).then(() => {
    execution.recovering = false;
  });
  executions.set(child.id, execution);
  return run;
}

/**
 * Start the run, and record what it does when it stops.
 *
 * Answers the execution's record and **`begun`**: resolved once the run's
 * lifecycle row is journaled — `runFlow`'s `opened` hook — and rejected with
 * whatever stopped the run before it was. What a gated [`admit`] waits for
 * before it says the execution started.
 */
function register(
  executions: Map<string, Execution>,
  admission: Admission,
  id: string,
): { readonly record: Execution; readonly begun: Promise<void> } {
  const { flow, inputs, sessionKey, callback, traceparent } = admission;
  const execution: Execution = {
    id,
    flow: flow.address,
    trigger: admission.trigger,
    status: "running",
    // This request is the execution's first generation, so there is nothing for
    // it to catch up to: a pause it has not reached yet is one nobody has been
    // handed a `resume_url` for. `parked` is what [`stillReplayingTo`] would
    // read, and it is never asked about an execution that is not recovering.
    recovering: false,
    parked: false,
    cancelling: false,
    ...(callback === undefined ? {} : { callback }),
    // Nothing has been announced about an execution that is one line old.
    reported: new Set<string>(),
    deliveries: Promise.resolve(),
    announced: false,
    // Replaced immediately below. The record has to exist before the run does,
    // because the run's own handlers write into it.
    settled: Promise.resolve(),
  };
  const unwatch = watchPauses(execution);
  let opened: () => void = () => {};
  const journaled = new Promise<void>((resolve) => {
    opened = resolve;
  });
  // `resumable: true` is what makes a `human` node a *pause* rather than the end
  // of the run: this app mounts the route that answers one (grammar 8.7,
  // PRD 5.11), which `agent-compose run` does not.
  const run = runFlow(flow.address, inputs, {
    executionId: id,
    sessionKey,
    resumable: true,
    // Which trigger started it, for the journal's lifecycle row. Recorded so a
    // reader of the journal can tell an `http` execution from a `run`, and so
    // the resume and status routes of this execution can enforce the auth of
    // the trigger that started it after a restart (grammar 13.3); never
    // re-fired on recovery (PRD resolved q28).
    trigger: admission.trigger,
    // And where its lifecycle webhooks go, on the same row and for the
    // reason [`resumeInto`] reads it back: the process that finishes this
    // execution may not be this one.
    ...(callback === undefined ? {} : { callback }),
    // …and the caller's trace, on the row for the same reason. Carried as it
    // arrived and validated where it is read (`./otlp.ts`'s
    // `parseTraceparent`), so a malformed header costs the execution nothing
    // — which is the W3C trace-context behaviour and PRD resolved q51's.
    ...(traceparent === undefined ? {} : { traceparent }),
    // The `settled` webhook, journaled before the row closes — see [`closed`].
    closing: (produced, error) => closed(execution, produced, error),
    // The lifecycle row is down: what a gated admission waits for.
    opened,
  });
  execution.settled = settling(execution, run).finally(unwatch);
  executions.set(id, execution);
  // A run resolves only after its row opened, so the race is decided by the
  // hook — or by the run's own rejection, where it stopped before the row.
  const lifecycle = Promise.race([journaled, run.then(() => undefined)]);
  // Nobody waits on it for an `http` start: a run that fails before its row is
  // the status route's and the webhook's to report ([`settling`]).
  lifecycle.catch(() => {});
  return { record: execution, begun: lifecycle };
}

/**
 * The completion webhook this request asked for, where it asked for one
 * (grammar 13.3's `callback:`).
 *
 * A URL the payload does not yield is **no webhook** rather than a bad request:
 * a completion webhook is optional, and `callback: "payload.body.callback_url"`
 * — the natural spelling, and the one the grammar's own example uses — reads a
 * key most callers will not have sent (grammar 4.1, Decision D110). So a
 * refusal here is an absence.
 *
 * Read when the request arrives rather than when the run ends, which is what
 * lets it be **recorded**. `payload` is fixed the moment the route is entered,
 * so the URL is the same either way; what differs is that a run finishing in
 * another process can still deliver it (`docs/durability.md` §6.1).
 */
/**
 * The request as an intake row may hold it (`docs/durability.md` §3.15, §8):
 * without the header the trigger's `auth:` verified, which carries a
 * credential — a token, or a signature over this request — and is never
 * written to the journal. A debounce binds its newest payload again when it
 * closes, so an `input:` reading that header reads it absent there (Decision
 * D163's Status); nothing else of the request is changed.
 */
function withoutCredential(trigger: HttpTrigger, payload: Payload): Payload {
  const auth = trigger.auth;
  if (auth === undefined) return payload;
  const credential = auth.header.toLowerCase();
  const headers: Record<string, string> = {};
  for (const [name, value] of Object.entries(payload.headers)) {
    if (name.toLowerCase() !== credential) headers[name] = value;
  }
  return { ...payload, headers };
}

function callbackOf(trigger: HttpTrigger, payload: Payload): string | undefined {
  if (trigger.callback === undefined) return undefined;
  try {
    return trigger.callback(payload);
  } catch {
    return undefined;
  }
}

/**
 * Record what a run did on its execution, and deliver its `settled` webhook.
 *
 * One tail for both ways a run reaches this process — a request that started it
 * and a recovery that picked it up — because the two differ in how a run
 * *begins* and in nothing after it. An execution recovered at start finishes
 * like any other, and a caller holding a `202` is owed the same push whichever
 * process got there.
 *
 * **The webhook is owed to a run that finished**, which is not every run that
 * stopped. A resume that meets a [`runtime.ReplayDivergence`] leaves the journal
 * row **open** on purpose (`docs/durability.md` §7): the disagreement is this
 * build's, not the execution's, so the composition can be put back and the next
 * start replays it again. Reporting `failed` to the caller would be this
 * process's opinion delivered as the execution's outcome — and then the recovery
 * that completes it would deliver a *second* webhook for one execution, the
 * first of them wrong.
 *
 * **So the push happens where the row closes**, which is inside the run:
 * [`closed`] is `runFlow`'s `closing` hook, reached only on the paths that close
 * the lifecycle row, and it journals the delivery's intent before the row goes.
 * A process killed a millisecond later leaves a `pending` row a later start
 * finishes; one that journaled the intent *after* the close would leave nothing
 * at all, for an execution `recover` will never look at again.
 *
 * What is left for this tail is the one run the hook is never reached for: a
 * failure raised **before** [`runtime.openExecution`] — a recovered execution
 * whose recorded inputs this build's `inputs:` no longer accept, a
 * `session_key:` the composition has since started requiring, a journal that
 * could not be opened. Nothing is journaled about such a run at all, so nothing
 * will ever recover it, and the absence of a row is exactly what says so.
 */
function settling(execution: Execution, run: Promise<FlowRun>): Promise<void> {
  return run
    .then((answer) => {
      recorded(execution, answer, undefined);
    })
    .catch(async (error: unknown) => {
      // The status route still reports what *this* process saw — a reader
      // polling it is asking about this build — and that is the whole of the
      // difference: the report is this process's, the push is the execution's.
      recorded(execution, undefined, error);
      if (execution.announced) return;
      // The one settle [`closed`] never hears about: a run that failed **before
      // it was journaled at all** — its inputs refused by a flow whose
      // `inputs:` this build has narrowed, a `session_key:` the composition has
      // since started requiring, a journal that could not be opened. No row
      // means no start will ever recover it, so the caller holding a `202` is
      // owed the failure now and this process is the only one that can push it.
      if (await unjournaled(execution.id)) void deliver(execution, "settled", []);
    })
    .finally(() => {
      // The run has stopped and its lifecycle row has closed (or stayed open,
      // which frees nothing): a slot of its trigger's lease may be free, and
      // the queue it held back is looked at (PRD resolved q77 ruling b).
      released(execution.trigger);
    });
}

/**
 * Record what the run did on the execution's own report.
 *
 * Said in two places for one reason: [`closed`] makes this statement while the
 * lifecycle row is still open, because the webhook it journals carries the
 * report and the report is this. [`settling`] makes it again off the settled
 * promise, which is where a run that never reached the hook — one that failed
 * before it was journaled — is recorded. The two agree by construction: the same
 * answer, written the same way.
 */
function recorded(execution: Execution, answer: FlowRun | undefined, error: unknown): void {
  if (answer !== undefined) {
    execution.status = "completed";
    execution.outputs = answer.outputs;
    execution.trace = answer.trace;
    if (answer.finallyError !== undefined) execution.finallyError = answer.finallyError;
    return;
  }
  // A run somebody cancelled is `cancelled`, with no `error`: nothing went
  // wrong with it (PRD resolved q75, Decision D160).
  if (endedByCancel(error)) {
    execution.status = "cancelled";
  } else {
    execution.status = "failed";
    execution.error = message(error);
  }
  const trace = (error as { trace?: readonly runtime.TraceEntry[] }).trace;
  if (trace !== undefined) execution.trace = trace;
  const finallyError = (error as { finallyError?: string } | null | undefined)?.finallyError;
  if (finallyError !== undefined) execution.finallyError = finallyError;
}

/**
 * Record this run's outcome and journal its `settled` delivery, **while the
 * lifecycle row is still open**.
 *
 * `runFlow`'s `closing` hook, and the order is what it is for (resolved q35,
 * `docs/durability.md` §3.7). A delivery whose intent went down *after* the row
 * closed is one no restart can find: `recover` sees no open execution and the
 * ledger holds no row, so a process killed in that window would owe a caller a
 * push nothing would ever send — and by q34's own reasoning that caller is not
 * polling, because the contract it was handed was a push.
 *
 * It is also what makes "a replay divergence fires nothing" (resolved q29)
 * structural rather than checked: the hook is only reached where the row is
 * about to close, and a divergence leaves it open.
 *
 * Only the **intent** is waited for. The attempts are not — [`opening`] sets
 * them going and returns — so a run does not stay `running` to a reader for as
 * long as a receiver takes to answer.
 *
 * And the intent is **insisted on** rather than tried once, because this is the
 * last moment anything comes back to it: see [`insisting`] and the comment on
 * the write itself.
 *
 * **Two intents, each guarded by its own question**, and that is the whole of
 * why the webhook's guard is written around it rather than in front of both. A
 * settle journals a `callback` row and a `trace_sink` row where a target
 * declares each, and a process killed between them leaves an execution that is
 * still `open` with only the first one down. The start that recovers it replays
 * it back to this hook, and a guard covering both would read the webhook row a
 * dead generation left, conclude the settle was done, and return — leaving that
 * execution's trace unexported for the life of the journal, against
 * `docs/trace.md` §1.4's "a recovered execution's" alike. So each intent asks
 * about **its own** kind: [`settledAlready`] about the webhook,
 * [`shipTrace`]'s `exportedAlready` about the export.
 */
async function closed(
  execution: Execution,
  produced: FlowRun | undefined,
  error: unknown,
): Promise<void> {
  recorded(execution, produced, error);
  execution.announced = true;
  // **A settle is once per execution**, and the journal is what says so across
  // generations — [`Execution.announced`] only knows about this one. The window
  // this closes is the one the ordering above opens: a process killed between
  // the intent going down and the row closing leaves an execution that is still
  // `open` *and* already has its `settled` row, so the start that recovers it
  // replays it to the same end. Its predecessor's row is picked up by
  // [`resumeDeliveries`] and delivered under the ordinal it was allocated;
  // announcing a second one here would tell a receiver that an execution
  // finished twice.
  if (!(await settledAlready(execution.id))) {
    // **The one write in this file that nothing at all would come back to**, and
    // so the one that is insisted on where it stands rather than left for a later
    // write to carry ([`insisting`]). The row closes as this returns: after that
    // `recover` walks no `open` execution for it and [`resumeDeliveries`] finds no
    // `pending` row, so an intent the journal would not take here — a second
    // process holding the file past the lock wait, a disk momentarily full, both
    // states `docs/durability.md` §2 says a healthy deployment reaches — is a
    // settle nothing ever announces, owed to a caller who was handed a `202` and
    // by resolved q34's own reasoning is not polling.
    //
    // The ladder runs **while the row is still open**, which is also what makes a
    // process killed part-way through it recoverable: the execution is still
    // `open`, so the next start replays it to the same end and reaches this hook
    // again. Bounded like every other ladder here (§3.7): its end is a sentence on
    // stderr and a status route that still holds the answer.
    await insisting(() => deliver(execution, "settled", []));
  }
  // Reached whether or not the webhook was journaled a moment ago, and that is
  // this hook's second half rather than its continuation: the export is owed by
  // **every** settled execution (grammar §14.5), including one whose webhook a
  // dead generation already put down. [`shipping`] is a no-op under a target
  // that declares no sink and idempotent under one that does, so arriving here
  // twice across generations costs a journal read.
  //
  // The outcome is derived from what the run answered rather than read back off
  // the execution, for the reason [`recorded`] derives it there: the two must
  // agree, and one derivation cannot disagree with itself. A cancelled run is
  // one that settled (PRD resolved q75); the envelope status a run holding a
  // pause carries is unreachable here — its row stays open and never reaches
  // this hook (`./graph.ts`'s `closing`).
  const outcome =
    produced !== undefined ? "completed" : endedByCancel(error) ? "cancelled" : "failed";
  await insisting(() => shipping(execution, outcome));
}

/**
 * Journal this execution's trace export, on the same chain and in the same
 * moment (grammar 14.5, PRD resolved q50).
 *
 * The **second** thing [`closed`] owes, and it is owed by every settled
 * execution rather than only by one somebody subscribed to: `callback:` is a
 * request's subscription and `trace_sink:` is the deployment's standing one, so
 * a run nobody asked about still ships its trace.
 *
 * On [`Execution.deliveries`] for the ordinal's sake: the chain is what
 * serializes ordinal allocation per execution, and a parking still journaling
 * when the run stopped would otherwise be racing this for the next number.
 *
 * The attempts are set going **after** the intent lands and are not awaited,
 * which is the whole of resolved q50's "never blocks or fails the run it
 * describes": what the lifecycle row waits for is a row in the journal, not a
 * collector.
 */
function shipping(
  execution: Execution,
  status: "completed" | "failed" | "cancelled",
): Promise<boolean> {
  if (!sinkConfigured()) return Promise.resolve(true);
  const journaled = execution.deliveries
    .then(() =>
      shipTrace({
        execution: execution.id,
        flow: execution.flow,
        status,
        ...(execution.error === undefined ? {} : { error: execution.error }),
        ...(execution.finallyError === undefined
          ? {}
          : { finallyError: execution.finallyError }),
        entries: execution.trace ?? [],
      }),
    )
    .then((record) => {
      if (record !== undefined) void workDelivery(record, sinkAuth(), true);
      return true;
    })
    .catch((error: unknown) => {
      process.stderr.write(
        `\`${execution.id}\`'s trace could not be journaled: ${message(error)}\n`,
      );
      return false;
    });
  execution.deliveries = journaled.then(() => undefined);
  return journaled;
}

/**
 * Whether this execution's `settled` delivery is already in the journal.
 *
 * A journal this process cannot read answers `false`, for [`unjournaled`]'s
 * reason turned the other way: the delivery contract is at-least-once and
 * receivers dedupe, so an event announced twice is the recoverable half.
 */
async function settledAlready(execution: string): Promise<boolean> {
  try {
    return (await deliveriesOf(execution)).some(
      // **The kind as well as the event**: a settle puts two rows on the ledger
      // where a target declares a sink, and both are `settled` events. Asking
      // about the event alone would let the trace export a first generation
      // journaled stand in for the webhook a second one still owes.
      (record) => record.kind === "callback" && record.event === "settled",
    );
  } catch {
    return false;
  }
}

/**
 * Whether the journal holds **no row at all** for this execution.
 *
 * [`settling`]'s question about a run that failed before it could be journaled,
 * which is the one settle [`closed`] is not reached for. A journal this process
 * cannot read answers `false`, because the honest reading of "I cannot tell" is
 * the conservative one: a push that should not have gone cannot be taken back,
 * while a push that was owed is still delivered by whichever process does close
 * the row.
 */
async function unjournaled(execution: string): Promise<boolean> {
  try {
    return (await journaledExecution(execution)) === undefined;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------------------
// Lifecycle webhooks (grammar 13.3, PRD resolved q34, q35)
// ---------------------------------------------------------------------------

/**
 * Be told when this execution's set of open pauses moves **or its work runs
 * out**, and answer the unsubscribe.
 *
 * Two things read the first of those. [`Execution.parked`] is one, and it is
 * the board's own fact rather than an assertion about the replay. The other is
 * the **parking webhook**: a quiescence that opened pauses nothing has announced
 * is the event resolved q34 says a receiver subscribes to — and a quiescence is
 * why the second subscription is here, because the moment an execution becomes
 * one is often a branch *finishing* rather than a pause opening.
 */
function watchPauses(execution: Execution): () => void {
  const consider = (): void => {
    if (humanWaits(execution.id).length > 0) execution.parked = true;
    if (execution.callback === undefined) return;
    parking(execution);
  };
  // **Two subscriptions, because a quiescence is not a pause event**
  // (resolved q34). A `map` over a flow with `human` nodes opens N pauses in one
  // quiescence, and the webhook owed is one listing all N — so the report has to
  // be taken once the *last* branch has parked, and that branch may be the one
  // whose model call was still running when the first pause opened. The pause
  // watch says a pause moved; the quiescence watch says a unit of work
  // finished; and `runtime.quiescent` is what decides, off the work in flight,
  // whether either of them was the last thing this execution was going to do.
  const unpause = watchHumanPauses(execution.id, consider);
  const unquiet = watchQuiescence(execution.id, consider);
  // …and a **third**, because `docs/distributed.md` §6.6 puts a placement wait
  // on the same board: "a subscribed system learns 'waiting for the Mac'
  // exactly the way it learns 'waiting for a human'". A quiescence whose only
  // open wait is a placed node with nobody to run it is a parking, and this is
  // the event that says so.
  const unplaced = watchPlacementWaits(execution.id, consider);
  return () => {
    unpause();
    unquiet();
    unplaced();
  };
}

/**
 * Deliver a `parked` webhook where this quiescence opened a pause nothing has
 * announced (resolved q34).
 *
 * The gate is the **set of pauses**, not the fact of parking, and that is what
 * makes recovery quiet: a recovered execution re-parks under the ids its
 * predecessor published (`docs/durability.md` §6.1), so nothing here is new and
 * nothing is sent. A resume that leads to another parking with a pause the set
 * does not hold fires the next one.
 *
 * The body lists **every pause then open** rather than only the new ones,
 * because it is the status route's report and that is what the status route
 * says — a receiver builds its view from a push exactly as it would from a
 * poll, and the two surfaces cannot disagree.
 */
function parking(execution: Execution): void {
  if (execution.status !== "running") return;
  // **One webhook per parking, not one per pause** (resolved q34): until every
  // branch of this execution has parked or finished, a pause that just opened is
  // one of a set still being assembled, and a delivery taken now would report a
  // growing prefix of it — N reports of one event to a receiver building its
  // view from the push.
  if (!quiescent(execution.id)) return;
  // Both kinds of wait, in one set. A parking is a *quiescence with something
  // open*, and `docs/distributed.md` §6.6 makes a placed node with no worker one
  // of the things that can be open — so an execution waiting only on a Mac fires
  // the webhook, and one waiting on a human and a Mac fires one listing both.
  const open = [
    ...humanWaits(execution.id).map((wait) => wait.id),
    // …and only the ones **no worker has taken**, which is the line
    // `docs/distributed.md` §6.4 draws: "no worker has taken the node yet" is
    // the pause, and a node a worker took is running rather than waiting. The
    // status route publishes both with their status; a `parked` delivery is
    // about what the execution is waiting for.
    ...placementWaits(execution.id)
      .filter((wait) => wait.status === "parked")
      .map((wait) => wait.id),
  ];
  if (open.length === 0 || open.every((id) => execution.reported.has(id))) return;
  // Marked **before** the journaling below rather than after it, because the
  // guard above is synchronous and the journaling is not: a second quiescence
  // arriving while the intent is still being written would find nothing reported
  // and take a second delivery of one set. [`announcing`] is what takes the
  // marks off again where the journal will not hold the row they stand for.
  for (const id of open) execution.reported.add(id);
  void announcing(execution, open);
}

/**
 * Journal one parking's intent, and leave nothing marked as announced that the
 * journal does not hold.
 *
 * The marks [`parking`] puts on are this process's reading of the `pauses` on
 * the delivery rows — a later start reads them straight back off those rows
 * ([`announced`]) — so a parking the journal would not take is a parking
 * nothing announced. Left marked it would be announced *never*: every later
 * quiescence of this execution finds the set already reported, and a receiver
 * that subscribed to the question hears only the settle.
 *
 * What is retried here is the **journaling**, not the delivery: a row that was
 * written has a schedule of its own ([`attempts`]) and this is only about
 * getting one written. It is retried at all because nothing else would come
 * back to it — an execution that has parked is waiting on a person, so the
 * quiescence that reached [`parking`] is the last event it raises until somebody
 * answers, and by then the question is closed and was never asked. Bounded for
 * `docs/durability.md` §3.7's reason: a webhook is a courtesy the status route
 * backstops, so the ladder's end is the marks coming off, never a queue that
 * grows.
 */
async function announcing(execution: Execution, open: readonly string[]): Promise<void> {
  for (let index = 0; ; index += 1) {
    if (await deliver(execution, "parked", open)) return;
    const wait = JOURNAL_RETRY_MS[index];
    // Every round has already said its own reason on stderr ([`deliver`]).
    if (wait === undefined) break;
    await pause(wait);
    // A run that has stopped is one [`closed`] has already reported on, and the
    // report it carries is the settle. Announcing a `parked` event off it would
    // push a body saying `completed` with no `interrupts` in it at all.
    if (execution.status !== "running") break;
  }
  for (const id of open) execution.reported.delete(id);
}

/**
 * Put one lifecycle event on this execution's delivery chain, and say whether
 * the journal took its intent.
 *
 * The **ordinal** is what a receiver orders by (grammar 13.3), so allocating it
 * is serialized per execution; the attempts are not, because a delivery that is
 * retrying for ten minutes must not hold up the next event's ordinal and
 * receivers are already told that deliveries can arrive out of order.
 *
 * The chain is answered as well as extended, for [`closed`]: the one caller that
 * has to *wait* for a delivery to be journaled is the one holding a lifecycle
 * row open until it is. It resolves when the intent is recorded, never when the
 * receiver answers.
 *
 * **The report is taken here**, where the event is, and not inside the chain.
 * `report` reads the execution's status and the pauses the runtime is holding as
 * it is called, and the chain ahead of it can be seconds long — a first delivery
 * still opening the journal while a resume lands, runs on and finishes the
 * execution. A body serialized then would announce a `parked` event whose report
 * says `completed` and carries no `interrupts` at all: a push a receiver cannot
 * render or answer, where resolved q34 promises it the same report a poll at the
 * moment of the event would have served. Fixing it at the intent is the only
 * place there is, because the intent's bytes are what every attempt resends.
 */
function deliver(
  execution: Execution,
  event: runtime.DeliveryEvent,
  pauses: readonly string[],
): Promise<boolean> {
  const url = execution.callback;
  // Nothing is owed and nothing failed: an execution nobody subscribed to has
  // no delivery to journal.
  if (url === undefined) return execution.deliveries.then(() => true);
  const body = report(execution);
  // The chain below is what awaits it. A handler is attached here so that a
  // journal failure inside `report` is not an unhandled rejection for as long as
  // the chain is still ahead of it — the rejection still reaches [`opening`],
  // and the `catch` below still says it.
  body.catch(() => undefined);
  const journaled = execution.deliveries
    .then(() => opening(execution, event, pauses, url, body))
    .then(() => true)
    .catch((error: unknown) => {
      // A delivery that could not even be *recorded* is not the execution's
      // failure — the run has produced whatever it produced and the status
      // route still holds it — but it is not something to swallow either: what
      // failed here is the journal, and a reader has no other way to learn it.
      process.stderr.write(
        `\`${execution.id}\`'s \`${event}\` webhook could not be journaled: ${message(error)}\n`,
      );
      return false;
    });
  // The chain carries no answer and never rejects: what it orders is the
  // *allocation* of ordinals, and a round that failed to allocate one still has
  // to let the next event through.
  execution.deliveries = journaled.then(() => undefined);
  return journaled;
}

/**
 * Record one delivery's intent, and set its attempts going.
 *
 * Three things happen here and the order is the contract (resolved q35): the
 * URL is held to what its trigger admits, the intent is journaled, and only
 * then is anything sent. The intent goes down **before** the first attempt so
 * that a process which dies mid-attempt leaves a row a later start finishes,
 * under the delivery id the receiver dedupes on.
 *
 * The `body` is the report [`deliver`] took at the event, awaited here rather
 * than built here: it is the bytes every attempt resends, so it has to be the
 * execution as it was when the thing being announced happened.
 *
 * **A trigger this build no longer declares leaves a `pending` row** rather than
 * nothing at all, which is what `docs/durability.md` §3.7 promises about it: the
 * event happened, this process cannot say what identity that trigger's
 * `callback_auth:` promised its receiver, and a build that declares it again
 * picks the row up ([`resumeDeliveries`]). Recording nothing would make the
 * event unrecoverable *and* invisible — the status route would show a settle
 * nobody was told about — so the row is written and the reason is said on
 * stderr. The allowlist is that trigger's too, so it is matched by the build
 * that has one, never skipped.
 */
async function opening(
  execution: Execution,
  event: runtime.DeliveryEvent,
  pauses: readonly string[],
  url: string,
  body: Promise<Record<string, unknown>>,
): Promise<void> {
  const trigger = httpTriggers.find((one) => one.name === execution.trigger);
  const intent = {
    execution: execution.id,
    kind: "callback" as const,
    // On the row itself, because the one delivery whose execution has **no**
    // lifecycle row to read it off is journaled here: [`settling`] fires a
    // `settled` for a run that failed before it was journaled at all, and a
    // later start that could not name that row's trigger could neither deliver
    // it nor end it (`docs/durability.md` §3.7, [`resumeDelivery`]).
    trigger: execution.trigger,
    event,
    url,
    body: JSON.stringify(await body),
    pauses,
  };
  if (trigger === undefined) {
    // The composition moved under an execution that still owes a delivery.
    // Journaled and said rather than sent: delivering without the identity that
    // trigger promised would be a request the receiver is written to refuse — or
    // worse, to accept — and the row left behind is what `recover` leaves for an
    // execution of a flow this build no longer has.
    await intendDelivery(intent);
    process.stderr.write(
      `\`${execution.id}\` was started by \`${execution.trigger}\`, which this build does not declare: its \`${event}\` webhook stays undelivered in the journal\n`,
    );
    return;
  }
  // **Held when the URL is read**, which is at the delivery rather than at the
  // start (grammar 13.3, Decision D110, D127): the callback URL comes out of the
  // request payload and is attacker-controlled by construction. A URL its
  // trigger does not admit is a *refused delivery* — journaled, visible on the
  // status route, never retried, and never anybody's failure.
  const refusal = unroutable(trigger, url);
  if (refusal !== undefined) {
    await refuseDelivery(intent, refusal);
    return;
  }
  const record = await intendDelivery(intent);
  // Set going and not awaited: a run must not stay `running` to a reader for as
  // long as a receiver takes to answer. `true` because this is the process that
  // waits out a schedule — see `workDelivery`.
  void workDelivery(record, trigger.callbackAuth, true);
}

/**
 * Why this trigger may not deliver to this URL, or `undefined` where it may.
 *
 * Both gates ask it — [`opening`] before the first attempt and
 * [`resumeDelivery`] before a restart puts an owed row back on its schedule — so
 * a URL one of them would send is never one the other refuses.
 */
function unroutable(trigger: HttpTrigger, url: string): string | undefined {
  if (credentialed(url)) return refusedUserinfo(trigger.name);
  if (trigger.callbackAllow !== undefined && !admits(trigger.callbackAllow, url)) {
    return refusedUrl(trigger.name);
  }
  return undefined;
}

/** Why a delivery was refused, in the one sentence both places that refuse use. */
function refusedUrl(trigger: string): string {
  return `the callback URL matches no \`callback_allow\` entry of the trigger \`${trigger}\``;
}

/** The other refusal, in the same shape. See [`credentialed`]. */
function refusedUserinfo(trigger: string): string {
  return `the callback URL of the trigger \`${trigger}\` carries userinfo before its host, so the authority \`callback_allow\` matches against is not the authority the request would reach`;
}

/**
 * Whether this URL puts **userinfo** before its host — the `user:pass@` an
 * authority may carry.
 *
 * A refused delivery, and refused whether or not the trigger declares a list.
 * Two reasons, and either alone would be enough.
 *
 * `callback_allow:` is matched against the URL **as text** (Decision D127), and
 * userinfo is where the text and the destination part company: a URL reading
 * `http://hooks.example.com:9000@attacker.test/hook` begins with
 * `http://hooks.example.com:`, so an entry carrying a wildcard where the port
 * goes — what an author writes for a receiver whose port the operating system
 * chose — admits it, while the host the request reaches is `attacker.test`. What
 * would travel is this execution's whole report under `X-AgentCompose-Signature`
 * and a `callback_auth: bearer` token, which is the guarantee the redirect rule
 * ([`attemptDelivery`]) keeps one step further on. Recognising it here rather
 * than inside the match keeps the matcher the bounded text scan
 * [`entryAdmits`] describes.
 *
 * And the two runtimes a built project runs under do not agree about such a
 * URL: one drops the userinfo and delivers to the host after the `@`, the other
 * refuses to construct the request at all. A wire contract that turned on which
 * one `serve` found would be no contract, so the URL is refused before either
 * of them is asked.
 */
function credentialed(url: string): boolean {
  const scheme = url.indexOf("://");
  if (scheme === -1) return false;
  const from = scheme + 3;
  // The authority ends at the first `/`, `?` or `#` after it; an `@` beyond that
  // is an ordinary path or query character and names no host.
  let end = url.length;
  for (let index = from; index < url.length; index += 1) {
    const character = url[index];
    if (character === "/" || character === "?" || character === "#") {
      end = index;
      break;
    }
  }
  const at = url.indexOf("@", from);
  return at !== -1 && at < end;
}

/**
 * Pick up every delivery the journal still holds pending
 * (`docs/durability.md` §6.1).
 *
 * Beside the executions [`Recovery`] replays, and separately from them: a
 * delivery reports on an execution that may have **ended** in the process that
 * died, so there is nothing to recover and something still to send. Its
 * remaining schedule is computed from the recorded intent, so a delivery whose
 * next offset has already passed goes at once.
 *
 * **One row's failure is one delivery left owed, never a process that refuses to
 * serve.** This runs in the `onReady` hook, so a rejection out of here is an
 * `app.listen()` that rejects — a deployment whose executions, routes and
 * journal are otherwise healthy would not bind a port. Every read below can fail
 * transiently for a reason that has nothing to do with the row it was about: a
 * second process on the same project contends the file, and `docs/durability.md`
 * §2 and §12 both make that a state a healthy deployment reaches. So each row is
 * picked up inside its own guard, in the posture [`recoverExecutions`] takes for
 * the read it guards and §3.7 takes for a webhook — a courtesy the status route
 * backstops, and one the next start picks up again.
 *
 * `undefined` once it has read the owed rows and walked them; what stopped it
 * where the journal could not be read, which leaves the walk owed
 * ([`Recovery`]). A row this process has already put on its schedule is never
 * put on a second one ([`resumeDelivery`]), so a walk a standby takes late
 * leaves alone a delivery it started itself.
 */
async function resumeDeliveries(late: boolean): Promise<string | undefined> {
  let pending: readonly runtime.DeliveryRecord[];
  try {
    pending = await undeliveredDeliveries();
  } catch (error) {
    if (!late) {
      process.stderr.write(
        `this project's undelivered deliveries could not be read: ${message(error)}\n`,
      );
    }
    return message(error);
  }
  for (const record of pending) {
    try {
      await resumeDelivery(record);
    } catch (error) {
      // The row is left as this start found it — a delivery it could not read is
      // one it does not rewrite — so the next start reads the same `pending` row
      // and picks it up from the same recorded intent.
      process.stderr.write(
        `\`${record.id}\` could not be picked up and stays undelivered in the journal: ${message(error)}\n`,
      );
    }
  }
  return undefined;
}

/** One owed delivery, put back on its schedule. See [`resumeDeliveries`]. */
async function resumeDelivery(record: runtime.DeliveryRecord): Promise<void> {
  // A row this start has already put on its schedule is not put on a second one.
  // See `./delivery.ts`'s `working` for how one row is reached from two
  // directions in one start.
  if (beingWorked(record.id)) return;
  if (record.kind === "trace_sink") {
    // **A sink row needs no trigger and meets no list** (grammar 14.5, PRD
    // resolved q50). Its identity is the deploy layer's own, so a build that
    // still declares a sink signs it and picks it up; one whose target no longer
    // declares one leaves it, the posture below takes for a trigger this build
    // has lost. There is nothing to refuse it against: the address was the
    // operator's, not a caller's.
    if (!sinkConfigured()) {
      process.stderr.write(
        `\`${record.id}\` is a trace export and this target declares no \`trace_sink:\`: it stays undelivered in the journal\n`,
      );
      return;
    }
    void workDelivery(record, sinkAuth(), true);
    process.stderr.write(`resumed delivery ${record.id} (trace)\n`);
    return;
  }
  // **Off the delivery row**, which is what lets a delivery be finished without
  // its execution: a `settled` journaled for a run that failed before it was
  // journaled at all has no lifecycle row, and reading the trigger there would
  // leave that row `pending` at every start for ever (`docs/durability.md`
  // §3.7). The lifecycle row is the fallback for a delivery written before the
  // ledger recorded its trigger, which is the only row that answers nothing.
  const named = record.trigger ?? (await journaledExecution(record.execution))?.trigger;
  const trigger = httpTriggers.find((one) => one.name === named);
  if (trigger === undefined) {
    // The trigger that promised this delivery its identity is not one this
    // build has, so the row stays pending for a build that does — the posture
    // [`opening`] takes, and the one `recover` takes for a flow it no longer
    // declares.
    process.stderr.write(
      `\`${record.id}\` was to be delivered for \`${named ?? "an unknown trigger"}\`, which this build does not declare: it stays undelivered in the journal\n`,
    );
    return;
  }
  // **The URL is held to its trigger here too**, and this is where a row
  // [`opening`] could not hold reaches that trigger at last: a delivery
  // journaled by a build with no declaration of the trigger has never been
  // held to it, and picking it up unchecked would POST an attacker-supplied
  // URL that the trigger admits nowhere (grammar 13.3, Decision D127). The
  // refusal is written onto the row it is about rather than opened as a second
  // event: the ordinal is the lifecycle event's, and the event has not
  // happened twice.
  const refusal = unroutable(trigger, record.url);
  if (refusal !== undefined) {
    await refuseRecordedDelivery(record.execution, record.ordinal, refusal);
    process.stderr.write(`refused delivery ${record.id} (${record.event})\n`);
    return;
  }
  void workDelivery(record, trigger.callbackAuth, true);
  process.stderr.write(`resumed delivery ${record.id} (${record.event})\n`);
}

/**
 * Whether `callback_allow:` admits this URL (grammar 13.3, Decision D127).
 *
 * An entry is matched against the **whole** URL as text, with `*` standing for
 * any run of characters — one wildcard kind, crossing `/` and `?` like any
 * other character, because a URL has no delimiter a second kind would be
 * significant about. The entry is compared as written, which is why the
 * compiler refuses a scheme spelled in another case: it would match nothing.
 *
 * Matching text against a URL is only a statement about where a delivery lands
 * for a URL whose text names its own host, which is [`credentialed`]'s subject
 * and why [`unroutable`] asks that first.
 */
function admits(patterns: readonly string[], url: string): boolean {
  return patterns.some((pattern) => entryAdmits(pattern, url));
}

/**
 * One entry against one URL, scanned rather than compiled.
 *
 * **The URL is attacker-supplied**: it comes out of the request payload
 * (`callback:` over grammar 4.1), which is the whole reason the allowlist is
 * matched at the delivery rather than at the start. So the matcher has to be
 * bounded in the length of what it is given, and a regular expression is not:
 * `https://*.example.com/hooks*-callback` compiled into one — two wildcards
 * with a literal after each, all of which grammar 13.3 admits — gives an engine
 * a split point to try at every occurrence of the literal between them, and a
 * URL naming a megabyte of them backtracks for minutes on the one thread that
 * answers every route and works every delivery. A refused delivery is a
 * courtesy; a wedged process is not.
 *
 * An entry is a run of literals separated by `*`, so a scan settles it: the
 * head has to be a prefix, the tail a suffix, and each literal between them the
 * **leftmost** occurrence after the one before. Leftmost is not a heuristic
 * here — a `*` matches any run including the empty one, so consuming less can
 * never lose a match that consuming more would have found — which is what makes
 * this the same answer the expression gave, in one pass rather than in
 * exponentially many.
 */
function entryAdmits(pattern: string, url: string): boolean {
  const runs = pattern.split("*");
  // No wildcard at all: the entry is the URL, and `split` handed back the
  // whole of it.
  if (runs.length === 1) return pattern === url;
  const head = runs[0] ?? "";
  const tail = runs[runs.length - 1] ?? "";
  // The two anchors first, and the length check between them: `head` and `tail`
  // may not claim the same characters, or `*` between them would have matched a
  // negative run.
  if (!url.startsWith(head)) return false;
  if (url.length < head.length + tail.length) return false;
  if (!url.endsWith(tail)) return false;
  const limit = url.length - tail.length;
  let at = head.length;
  for (let index = 1; index < runs.length - 1; index += 1) {
    const run = runs[index] ?? "";
    // `**` in an entry is one wildcard written twice, and the empty run between
    // them consumes nothing.
    if (run === "") continue;
    const found = url.indexOf(run, at);
    if (found === -1 || found + run.length > limit) return false;
    at = found + run.length;
  }
  return true;
}

/**
 * What both the status route and the callback report about an execution.
 *
 * The document itself is `runtime.executionReport`'s, which is what keeps the
 * webhook's body and the status route's answer the same document rather than
 * two that agree today: `agent-compose resume` closes a `serve`-started
 * execution too, and the `settled` webhook it journals carries this report
 * without an app anywhere to serve it (`docs/durability.md` §6.2).
 *
 * What is added here is the half only an app can say — the **pauses**, with the
 * URL that answers each. `interrupts` is on exactly the report whose `status` is
 * `interrupted`: every pause the execution is holding, with what the human is
 * shown, the schema their answer has to fit, and the URL that delivers it
 * (grammar 8.7). It is what makes a status poll enough to *present* the question
 * rather than only to notice that there is one. Ordered by `wait_id` —
 * `runtime.humanWaits`'s order — so that two runs of one composition publish the
 * same questions in the same order whatever order their instances were scheduled
 * in.
 */
function report(execution: Execution): Promise<Record<string, unknown>> {
  const waits = execution.status === "running" ? humanWaits(execution.id) : [];
  // The placement waits of `docs/distributed.md` §6.1, on the same report and
  // under a key of their own. Not folded into `interrupts:`, because that key
  // means "a question a human can answer" (grammar §8.7) and a placed node is
  // waiting for a machine: an execution holding one is `running`, not
  // `interrupted`, and a UI told otherwise would publish a question with no
  // schema and a `resume_url` nothing would take.
  const placed = execution.status === "running" ? placementWaits(execution.id) : [];
  return executionReport({
    id: execution.id,
    flow: execution.flow,
    trigger: execution.trigger,
    status: statusOf(execution),
    interrupts: waits.map((wait) => question(execution, wait)),
    placements: placed.map((wait) => ({
      wait_id: wait.id,
      dispatch_id: wait.dispatch,
      placement: wait.placement,
      node: wait.node,
      instance_path: wait.site,
      parked_at: wait.parkedAt,
      status: wait.status,
    })),
    ...(execution.outputs === undefined ? {} : { outputs: execution.outputs }),
    ...(execution.error === undefined ? {} : { error: execution.error }),
    ...(execution.finallyError === undefined ? {} : { finallyError: execution.finallyError }),
    ...(execution.trace === undefined ? {} : { trace: execution.trace }),
  });
}

function accepted(reply: FastifyReply, execution: Execution): unknown {
  return reply.code(202).send({
    execution_id: execution.id,
    status: statusOf(execution),
    status_url: `/executions/${execution.id}`,
  });
}

function unknownExecution(reply: FastifyReply, id: string): unknown {
  return reply
    .code(404)
    .send({ error: `no execution \`${id}\` was started by this process` });
}

/**
 * Whether `settled` finished inside `budget`.
 *
 * The timer is cleared either way, and it is `unref`ed where the runtime offers
 * it, so a pending budget never keeps the process alive past the work it was
 * bounding.
 */
function within(settled: Promise<void>, budget: number): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    const timer: unknown = setTimeout(() => resolve(false), budget);
    if (typeof (timer as { unref?: () => void }).unref === "function") {
      (timer as { unref: () => void }).unref();
    }
    void settled.then(() => {
      clearTimeout(timer as Parameters<typeof clearTimeout>[0]);
      resolve(true);
    });
  });
}

/** A header or query bag, as the map of strings grammar 13.3 declares. */
function strings(held: unknown): Record<string, string> {
  const flattened: Record<string, string> = {};
  for (const [name, value] of Object.entries((held ?? {}) as Record<string, unknown>)) {
    if (value === undefined || value === null) continue;
    // A repeated header or query parameter arrives as a list; the payload's
    // declared type is a map of **string**, so the values are joined the way an
    // HTTP field with repeated values is written.
    flattened[name] = Array.isArray(value) ? value.map(String).join(", ") : String(value);
  }
  return flattened;
}

/**
 * Start the app and announce where it is listening.
 *
 * The readiness line is one JSON object on stdout, before anything else is
 * written there — `{"base_url":"http://127.0.0.1:8787"}` — so a caller that asked
 * for `--port 0` learns which port it got without guessing and without racing.
 *
 * **A signal ends the process with its executions in flight**, child executions
 * included, and that is the durability contract rather than a gap in it
 * (`docs/durability.md` §6.1): every one of them is an open row in the journal,
 * so the next start recovers each — a child whose parent had already settled
 * among them — and replays it to its frontier. What a stop costs is the one
 * effect each was in the middle of, which is §2's window for any execution.
 *
 * **What the executions that already settled left over the stores is finished
 * first** (PRD resolved q69, `./stores.ts`'s `drainStores`): each one's ledger
 * prune still in flight, each one still waiting on a detached delivery the
 * exit is about to cut off, and the store-wide sweeps this start began. A
 * settled execution is the one thing the next start does not come back for —
 * recovery takes open rows (PRD resolved q65) — so a prune a stop cut off
 * would be rows no process ever removes, and a `serve` stopped on every deploy
 * would grow its ledger with history. Nothing is closed under the executions
 * still running meanwhile, and a dial the wait needs is bounded, so a store
 * whose server does not answer holds the stop for ten seconds and no longer.
 * The stop happens once: a second signal while it is under way — the launcher
 * forwarding one the terminal also delivered, or `builtin.bash`'s group sweep
 * raising the signal again (`./runtime.ts`) — joins it.
 */
export async function serve(options: ServeOptions = {}): Promise<FastifyInstance> {
  const app = createApp();
  const host = options.host ?? "127.0.0.1";
  const port = options.port ?? 0;
  await app.listen({ host, port });
  const address = app.server.address();
  const bound = typeof address === "object" && address !== null ? address.port : port;
  const shown = host === "0.0.0.0" || host === "::" ? "127.0.0.1" : host;
  process.stdout.write(`${JSON.stringify({ base_url: `http://${shown}:${bound}` })}\n`);
  // A process start, which is half of PRD resolved q69's cleanup schedule: every
  // `kv` store the composition binds starts shedding the keys that have expired
  // in it, in the background and behind every request (`./stores.ts`).
  sweepBoundStores();
  let stopping = false;
  const stop = (): void => {
    if (stopping) return;
    stopping = true;
    // The app first, so no request starts an execution while the stores are
    // drained; then the stores, with the executions still in flight running on
    // beside the wait (see above).
    void app.close().then(async () => {
      await drainStores();
      process.exit(0);
    });
  };
  for (const signal of ["SIGINT", "SIGTERM"] as const) process.on(signal, stop);
  return app;
}
