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
//
// It is byte-identical in every project this compiler release builds, like
// `./runtime.ts`: what differs between two projects is `./triggers.ts`.
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
// q26-q29), and [`recover`] puts every one the journal holds open back on this
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
// The subtle clause is that `auth:` covers **three** routes rather than one.
// `resume` injects data into a parked run and `status` publishes what a run is
// holding, and both are per execution rather than per trigger — so each enforces
// the auth of **the trigger that started that execution**, which the journal's
// lifecycle row records (`docs/durability.md` §3.5). An execution a no-auth
// trigger began keeps open routes; one an authenticated trigger began never
// answers an unauthenticated poll, in this process or in the one that recovers
// it after a restart.
//
// # Lifecycle webhooks (grammar 13.3, PRD resolved q34, q35)
//
// A trigger's `callback:` is a subscription to the execution's **lifecycle**
// rather than only to its end. Two events reach it: a **parking**, one webhook
// per quiescence listing every pause then open, and the **settle** that closes
// the lifecycle row. Each carries the report the status route serves and the
// `X-AgentCompose-*` headers grammar 13.3 tabulates, signed and identified by
// `callback_auth:` where the trigger declares one, and refused before it is
// sent where `callback_allow:` admits its URL nowhere.
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

import { type CompiledFlow, type FlowRun, flows, runFlow } from "./graph.ts";
import {
  TRACE_VERSION,
  deliverHumanAnswer,
  deliveriesOf,
  humanWaits,
  intendDelivery,
  journaledExecution,
  openExecutions,
  recordDeliveryAttempt,
  refuseDelivery,
  undeliveredDeliveries,
  watchHumanPauses,
} from "./runtime.ts";
import type * as runtime from "./runtime.ts";
import { type HttpTrigger, httpTriggers } from "./triggers.ts";

/** What an execution this process started is doing (PRD 5.11). */
export type ExecutionStatus = "running" | "completed" | "failed" | "interrupted";

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
   * Whether this is an execution [`recover`] picked up whose replay is **still
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
  outputs?: Record<string, unknown>;
  trace?: readonly runtime.TraceEntry[];
  error?: string;
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
  // Read here, before a route exists, so an unreadable override is a command
  // that could not be run rather than a schedule nobody notices until the first
  // delivery is already late (Decision D50, `docs/durability.md` §3.7).
  retrySchedule();
  decodeBodies(app);

  for (const trigger of httpTriggers) {
    app.route({
      method: trigger.method,
      url: trigger.path,
      handler: (request, reply) => start(executions, trigger, request, reply),
    });
  }

  // Recovery, on the hook Fastify runs **before** the server accepts a
  // connection: `onReady` is awaited by `listen`, so every execution the
  // journal holds open has been put back on the board before the first request
  // arrives (PRD resolved q28). A resume request that lands the instant after
  // `listen` resolves therefore finds its wait, which is the whole promise —
  // the wait id is deterministic (node path + ordinal), so it is the same id the
  // caller was given by the process that died.
  app.addHook("onReady", async () => {
    await recover(executions);
  });

  app.get("/executions/:id", async (request, reply) => {
    const id = (request.params as { id: string }).id;
    const execution = executions.get(id);
    if (execution === undefined) return unknownExecution(reply, id);
    const refusal = guarded(execution, request);
    if (refusal !== undefined) return refuse(reply, execution.trigger, refusal);
    return reply.code(200).send(await report(execution));
  });

  app.post("/executions/:id/resume", (request, reply) => {
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
    if (execution.status !== "running") {
      return reply.code(409).send({
        execution_id: id,
        status: execution.status,
        error: `this execution has already ${execution.status}, so nothing is waiting for an answer`,
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
    const outcome = deliverHumanAnswer(id, named, request.body);
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

  return app;
}

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
function verified(trigger: HttpTrigger, request: FastifyRequest): Refusal | undefined {
  const auth = trigger.auth;
  if (auth === undefined) return undefined;
  const sent = header(request, auth.header);
  if (sent === undefined) {
    return {
      scheme: auth.scheme,
      detail: `no \`${auth.header}\` header carried a credential`,
    };
  }
  if (!sent.startsWith(auth.prefix)) {
    return {
      scheme: auth.scheme,
      detail: `the \`${auth.header}\` header does not begin with the expected prefix`,
    };
  }
  const offered = sent.slice(auth.prefix.length);
  if (auth.scheme === "bearer") {
    return equal(offered, secret(auth.tokenEnv))
      ? undefined
      : { scheme: "bearer", detail: "the credential does not match" };
  }
  // The signature is over the **raw request body bytes**, before any JSON
  // decoding and after none of it (grammar 13.3): a re-serialized body is a
  // different byte string and would fail every signature a vendor computed. See
  // [`decodeBodies`], which is what keeps those bytes.
  const signed = createHmac(auth.algorithm, secret(auth.secretEnv))
    .update(raw(request))
    .digest(auth.encoding);
  return equal(offered, signed)
    ? undefined
    : { scheme: "hmac", detail: "the signature does not verify over this request's body" };
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
  const trigger = httpTriggers.find((one) => one.name === execution.trigger);
  if (trigger !== undefined) return verified(trigger, request);
  if (execution.trigger === MANUAL) return undefined;
  return {
    detail: `it was started by the trigger \`${execution.trigger}\`, which this build does not declare, so the authentication that trigger required cannot be read`,
  };
}

/**
 * One request header by name, matched **case-insensitively** (grammar 13.3).
 *
 * Header names are case-insensitive by definition and HTTP/2 lowercases every
 * one on the wire, so a check that compared the author's capitalisation would
 * refuse every genuine delivery over HTTP/2 while passing a `curl` that
 * happened to preserve case. The framework presents them lowercased, which is
 * the same reason `payload.headers` does.
 *
 * A header sent **more than once** carries no credential: the values arrive as
 * a list, and a request offering two is one this route cannot say verified.
 */
function header(request: FastifyRequest, name: string): string | undefined {
  const value = request.headers[name.toLowerCase()];
  return typeof value === "string" ? value : undefined;
}

/**
 * The resolved value of one credential's `${ENV}` reference.
 *
 * Present by construction: `src/env.ts` lists every credential this
 * composition's triggers name and `src/index.ts` resolves the whole list at
 * module scope, so a deployment missing one never reaches a route (grammar 4.3,
 * PRD resolved q15). The fallback is what keeps a comparison constant-time
 * rather than a `TypeError` that would answer `500` on the one request that
 * mattered.
 */
function secret(variable: string): string {
  return process.env[variable] ?? "";
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

  let inputs: Record<string, unknown>;
  let sessionKey = "";
  try {
    inputs = trigger.input(payload);
    if (trigger.sessionKey !== undefined) sessionKey = trigger.sessionKey(payload);
  } catch (error) {
    return reply.code(400).send({
      error: `the trigger \`${trigger.name}\` could not read this request: ${message(error)}`,
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

  const execution = register(executions, flow, trigger, inputs, sessionKey, payload);

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
 */
async function recover(executions: Map<string, Execution>): Promise<void> {
  let open: readonly runtime.ExecutionRow[];
  try {
    open = await openExecutions();
  } catch (error) {
    // A journal that cannot be opened is a project that cannot recover, and it
    // is not a reason to refuse to serve: the app starts, new executions
    // journal (or fail loudly when they cannot), and this says what happened.
    process.stderr.write(`this project's journal could not be read: ${message(error)}\n`);
    return;
  }
  for (const row of open) {
    // One generation of one execution per process. Nothing can be running yet —
    // this hook is what runs before the first connection — so the guard is a
    // statement rather than a fix: an execution this process is already replaying
    // is never replayed a second time beside itself.
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
    // The pauses this execution's earlier generations already announced, read
    // **before** the replay starts so the re-parking cannot race the knowledge
    // of it: a recovered execution re-parks under the same wait ids, and a
    // parking webhook for a question already asked would tell a receiver
    // something happened that did not (PRD resolved q35).
    resumeInto(executions, flow, row, await announced(row.id));
    process.stderr.write(`recovered ${row.id} (${row.flow})\n`);
  }
  await resumeDeliveries();
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
    // The webhook the caller who started this execution is still waiting for.
    // It is on the lifecycle row because *this* process is the one that will
    // finish the run, and the request that named the URL reached the one that
    // did not (`docs/durability.md` §6.1). A recovered execution that fired no
    // webhook would leave a caller who was handed a `202` with no signal at all
    // — the contract was push, so nobody is polling the status route.
    ...(row.callback === undefined ? {} : { callback: row.callback }),
    reported,
    deliveries: Promise.resolve(),
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
    }),
  )
    .then(() => {
      execution.recovering = false;
    })
    .finally(unwatch);
  executions.set(row.id, execution);
  return execution;
}

/** Start the run, and record what it does when it stops. */
function register(
  executions: Map<string, Execution>,
  flow: CompiledFlow,
  trigger: HttpTrigger,
  inputs: Record<string, unknown>,
  sessionKey: string,
  payload: Payload,
): Execution {
  const id = `exec_${globalThis.crypto.randomUUID()}`;
  const callback = callbackOf(trigger, payload);
  const execution: Execution = {
    id,
    flow: flow.address,
    trigger: trigger.name,
    status: "running",
    // This request is the execution's first generation, so there is nothing for
    // it to catch up to: a pause it has not reached yet is one nobody has been
    // handed a `resume_url` for. `parked` is what [`stillReplayingTo`] would
    // read, and it is never asked about an execution that is not recovering.
    recovering: false,
    parked: false,
    ...(callback === undefined ? {} : { callback }),
    // Nothing has been announced about an execution that is one line old.
    reported: new Set<string>(),
    deliveries: Promise.resolve(),
    // Replaced immediately below. The record has to exist before the run does,
    // because the run's own handlers write into it.
    settled: Promise.resolve(),
  };
  const unwatch = watchPauses(execution);
  // `resumable: true` is what makes a `human` node a *pause* rather than the end
  // of the run: this app mounts the route that answers one (grammar 8.7,
  // PRD 5.11), which `agent-compose run` does not.
  execution.settled = settling(
    execution,
    runFlow(flow.address, inputs, {
      executionId: id,
      sessionKey,
      resumable: true,
      // Which trigger started it, for the journal's lifecycle row. Recorded so a
      // reader of the journal can tell an `http` execution from a `run`, and so
      // the resume and status routes of this execution can enforce the auth of
      // the trigger that started it after a restart (grammar 13.3); never
      // re-fired on recovery (PRD resolved q28).
      trigger: trigger.name,
      // And where its lifecycle webhooks go, on the same row and for the
      // reason [`resumeInto`] reads it back: the process that finishes this
      // execution may not be this one.
      ...(callback === undefined ? {} : { callback }),
    }),
  ).finally(unwatch);
  executions.set(id, execution);
  return execution;
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
 * first of them wrong. So the guard is the lifecycle row: what stays open sends
 * nothing, and the process that finally closes the row is the one that pushes,
 * once.
 *
 * **Which is read off the row itself**, rather than inferred from the error.
 * "This error is not one that keeps the row open" is a different question from
 * "this generation closed the row", and the two part company on every failure
 * raised *before* [`runtime.openExecution`] — a recovered execution whose
 * recorded inputs this build's `inputs:` no longer accept, a `session_key:` the
 * composition has since started requiring, a journal written by another compiler
 * release. Every one of those leaves the lifecycle row untouched and **open**,
 * and every one of them is an ordinary `Error` that [`runtime.staysOpen`] says
 * nothing about — so the inference pushes `failed`, and the start that finally
 * replays the execution pushes again. The row is the fact; this asks it.
 */
function settling(execution: Execution, run: Promise<FlowRun>): Promise<void> {
  return run
    .then((answer) => {
      execution.status = "completed";
      execution.outputs = answer.outputs;
      execution.trace = answer.trace;
      return true;
    })
    .catch(async (error: unknown) => {
      execution.status = "failed";
      execution.error = message(error);
      const trace = (error as { trace?: readonly runtime.TraceEntry[] }).trace;
      if (trace !== undefined) execution.trace = trace;
      // The status route still reports what *this* process saw — a reader
      // polling it is asking about this build — and that is the whole of the
      // difference: the report is this process's, the push is the execution's.
      return !(await stillOpen(execution.id));
    })
    .then((finished) => {
      if (!finished) return;
      // Queued rather than awaited, for [`Execution.deliveries`]'s reason and
      // for one more: a run must not stay `running` to a reader of
      // [`Execution.settled`] for as long as a receiver takes to answer, and a
      // receiver that never does would otherwise hold a recovery's
      // catching-up window open for the whole retry schedule.
      deliver(execution, "settled", []);
    });
}

/**
 * Whether the journal still holds this execution **open** — the one question
 * [`settling`] has to answer before it pushes.
 *
 * An id the journal holds no row for is **not** open, and that is the right
 * answer rather than a missing case: it is a request whose run failed before it
 * could be journaled at all, so no start will ever recover it and the caller who
 * was handed a `202` is owed the failure now. What has a row and is still open
 * is the execution somebody else will finish.
 *
 * A journal this process cannot read answers `true`, because the honest reading
 * of "I cannot tell" here is the conservative one: a push that should not have
 * gone cannot be taken back, while a push that was owed is still delivered by
 * whichever process does close the row.
 */
async function stillOpen(execution: string): Promise<boolean> {
  try {
    return (await journaledExecution(execution))?.status === "open";
  } catch {
    return true;
  }
}

// ---------------------------------------------------------------------------
// Lifecycle webhooks (grammar 13.3, PRD resolved q34, q35)
// ---------------------------------------------------------------------------

/**
 * Be told when this execution's set of open pauses moves, and answer the
 * unsubscribe.
 *
 * Two things read that. [`Execution.parked`] is one, and it is the board's own
 * fact rather than an assertion about the replay. The other is the **parking
 * webhook**: a quiescence that opened pauses nothing has announced is the event
 * resolved q34 says a receiver subscribes to.
 */
function watchPauses(execution: Execution): () => void {
  let scheduled = false;
  return watchHumanPauses(execution.id, () => {
    if (humanWaits(execution.id).length > 0) execution.parked = true;
    if (execution.callback === undefined || scheduled) return;
    // **One webhook per parking, not one per pause** (resolved q34): a `map`
    // over a flow with `human` nodes opens N pauses inside one quiescence, and
    // a delivery per pause would spray a receiver with N reports of one event.
    // So the announcements of a turn are coalesced and the report is taken once
    // the turn has run out — every pause that opened in it is then on the board
    // together, and the body lists them all.
    scheduled = true;
    const timer: unknown = setTimeout(() => {
      scheduled = false;
      parking(execution);
    }, 0);
    if (typeof (timer as { unref?: () => void }).unref === "function") {
      (timer as { unref: () => void }).unref();
    }
  });
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
  const open = humanWaits(execution.id).map((wait) => wait.id);
  if (open.length === 0 || open.every((id) => execution.reported.has(id))) return;
  for (const id of open) execution.reported.add(id);
  deliver(execution, "parked", open);
}

/**
 * Put one lifecycle event on this execution's delivery chain.
 *
 * The **ordinal** is what a receiver orders by (grammar 13.3), so allocating it
 * is serialized per execution; the attempts are not, because a delivery that is
 * retrying for ten minutes must not hold up the next event's ordinal and
 * receivers are already told that deliveries can arrive out of order.
 */
function deliver(
  execution: Execution,
  event: runtime.DeliveryEvent,
  pauses: readonly string[],
): void {
  execution.deliveries = execution.deliveries
    .then(() => opening(execution, event, pauses))
    .catch((error: unknown) => {
      // A delivery that could not even be *recorded* is not the execution's
      // failure — the run has produced whatever it produced and the status
      // route still holds it — but it is not something to swallow either: what
      // failed here is the journal, and a reader has no other way to learn it.
      process.stderr.write(
        `\`${execution.id}\`'s \`${event}\` webhook could not be journaled: ${message(error)}\n`,
      );
    });
}

/**
 * Record one delivery's intent, and set its attempts going.
 *
 * Three things happen here and the order is the contract (resolved q35): the
 * URL is matched against `callback_allow:`, the intent is journaled, and only
 * then is anything sent. The intent goes down **before** the first attempt so
 * that a process which dies mid-attempt leaves a row a later start finishes,
 * under the delivery id the receiver dedupes on.
 */
async function opening(
  execution: Execution,
  event: runtime.DeliveryEvent,
  pauses: readonly string[],
): Promise<void> {
  const url = execution.callback;
  if (url === undefined) return;
  const trigger = httpTriggers.find((one) => one.name === execution.trigger);
  if (trigger === undefined) {
    // The composition moved under an execution that still owes a delivery. Said
    // rather than sent: this build cannot know what identity that trigger's
    // `callback_auth:` promised its receiver, and delivering without it would be
    // a request the receiver is written to refuse — or worse, to accept. Left
    // for a build that declares the trigger, which is what `recover` does with
    // an execution of a flow it no longer has.
    process.stderr.write(
      `\`${execution.id}\` was started by \`${execution.trigger}\`, which this build does not declare: its \`${event}\` webhook is not delivered\n`,
    );
    return;
  }
  const intent = {
    execution: execution.id,
    event,
    url,
    body: JSON.stringify(await report(execution)),
    pauses,
  };
  // **Matched when the URL is read**, which is at the delivery rather than at
  // the start (grammar 13.3, Decision D110, D127): the callback URL comes out of
  // the request payload and is attacker-controlled by construction. A URL the
  // list admits nowhere is a *refused delivery* — journaled, visible on the
  // status route, never retried, and never anybody's failure.
  if (trigger.callbackAllow !== undefined && !admits(trigger.callbackAllow, url)) {
    await refuseDelivery(
      intent,
      `the callback URL matches no \`callback_allow\` entry of the trigger \`${trigger.name}\``,
    );
    return;
  }
  const record = await intendDelivery(intent);
  void attempts(record, trigger);
}

/**
 * Work one delivery's remaining schedule (resolved q35).
 *
 * The offsets are measured from the **recorded intent**, not from now, which is
 * what makes a restart pick a delivery up where it left it rather than start
 * its schedule again: a row with two attempts on it resumes at the third
 * offset, due at `intendedAt + schedule[2]`, which may already be in the past.
 *
 * Exhaustion is recorded and is **never the execution's failure**: a webhook is
 * a courtesy the status route backstops, not a contract worth an unbounded
 * queue.
 */
async function attempts(record: runtime.DeliveryRecord, trigger: HttpTrigger): Promise<void> {
  const schedule = retrySchedule();
  const intended = Date.parse(record.intendedAt);
  for (let index = record.attempts.length; index < schedule.length; index += 1) {
    await pause(intended + (schedule[index] ?? 0) - Date.now());
    const outcome = await attemptDelivery(record, trigger);
    const last = index === schedule.length - 1;
    try {
      await recordDeliveryAttempt(
        record.execution,
        record.ordinal,
        {
          at: new Date().toISOString(),
          outcome: outcome.ok ? "delivered" : "failed",
          detail: outcome.detail,
        },
        outcome.ok ? "delivered" : last ? "exhausted" : "pending",
      );
    } catch (error) {
      process.stderr.write(
        `\`${record.id}\`'s attempt could not be journaled: ${message(error)}\n`,
      );
    }
    if (outcome.ok) return;
  }
}

/**
 * POST one delivery, and say what the receiver did with it.
 *
 * Every attempt sends the **same bytes** under the same delivery id: the body
 * was serialized once, at the intent, so a signature computed here is a
 * signature over what a receiver will verify and a retry is the same delivery
 * rather than a second one wearing its id.
 *
 * The headers are grammar 13.3's table, and the two `callback_auth:` halves may
 * both apply — a receiver that checks a token and a receiver that verifies a
 * signature are two receivers, and one trigger may deliver to one that does
 * both.
 */
async function attemptDelivery(
  record: runtime.DeliveryRecord,
  trigger: HttpTrigger,
): Promise<{ readonly ok: boolean; readonly detail: string }> {
  const headers: Record<string, string> = {
    "content-type": "application/json",
    "X-AgentCompose-Event": record.event,
    "X-AgentCompose-Delivery": record.id,
    "X-AgentCompose-Ordinal": String(record.ordinal),
    // The **intent's** instant, so every attempt of one delivery agrees about
    // when the event happened. A per-attempt stamp would make two copies of one
    // delivery disagree about the thing they report.
    "X-AgentCompose-Timestamp": record.intendedAt,
  };
  const auth = trigger.callbackAuth;
  if (auth?.hmac !== undefined) {
    // Fixed at HMAC-SHA256 written in hex, with no keys of its own, so one
    // receiver-side recipe verifies every agent-compose deployment (D127).
    headers["X-AgentCompose-Signature"] = `sha256=${createHmac(
      "sha256",
      secret(auth.hmac.secretEnv),
    )
      .update(record.body)
      .digest("hex")}`;
  }
  if (auth?.bearer !== undefined) {
    // Written as authored: capitalisation is the receiver's to read, never to
    // match, and the compiler has already refused a name the delivery writes
    // itself (grammar 13.3, D127).
    headers[auth.bearer.header] = `${auth.bearer.prefix}${secret(auth.bearer.tokenEnv)}`;
  }
  try {
    const answered = await fetch(record.url, {
      method: "POST",
      headers,
      body: record.body,
    });
    const ok = answered.status >= 200 && answered.status < 300;
    return { ok, detail: `the receiver answered ${answered.status}` };
  } catch (error) {
    return { ok: false, detail: message(error) };
  }
}

/**
 * Pick up every delivery the journal still holds pending
 * (`docs/durability.md` §6.1).
 *
 * Beside the executions [`recover`] replays, and separately from them: a
 * delivery reports on an execution that may have **ended** in the process that
 * died, so there is nothing to recover and something still to send. Its
 * remaining schedule is computed from the recorded intent, so a delivery whose
 * next offset has already passed goes at once.
 */
async function resumeDeliveries(): Promise<void> {
  let pending: readonly runtime.DeliveryRecord[];
  try {
    pending = await undeliveredDeliveries();
  } catch (error) {
    process.stderr.write(
      `this project's undelivered callbacks could not be read: ${message(error)}\n`,
    );
    return;
  }
  for (const record of pending) {
    const row = await journaledExecution(record.execution);
    const trigger = httpTriggers.find((one) => one.name === row?.trigger);
    if (trigger === undefined) {
      // The trigger that promised this delivery its identity is not one this
      // build has, so the row stays pending for a build that does — the posture
      // [`opening`] takes, and the one `recover` takes for a flow it no longer
      // declares.
      process.stderr.write(
        `\`${record.id}\` was to be delivered for \`${row?.trigger ?? "an unknown trigger"}\`, which this build does not declare: it stays undelivered in the journal\n`,
      );
      continue;
    }
    void attempts(record, trigger);
    process.stderr.write(`resumed delivery ${record.id} (${record.event})\n`);
  }
}

/** Wait out one retry offset, without holding the process open for it. */
function pause(milliseconds: number): Promise<void> {
  if (milliseconds <= 0) return Promise.resolve();
  return new Promise<void>((resolve) => {
    const timer: unknown = setTimeout(resolve, milliseconds);
    if (typeof (timer as { unref?: () => void }).unref === "function") {
      (timer as { unref: () => void }).unref();
    }
  });
}

/**
 * Whether `callback_allow:` admits this URL (grammar 13.3, Decision D127).
 *
 * An entry is matched against the **whole** URL as text, with `*` standing for
 * any run of characters — one wildcard kind, crossing `/` and `?` like any
 * other character, because a URL has no delimiter a second kind would be
 * significant about. The entry is compared as written, which is why the
 * compiler refuses a scheme spelled in another case: it would match nothing.
 */
function admits(patterns: readonly string[], url: string): boolean {
  return patterns.some((pattern) => {
    const source = pattern.split("*").map(literally).join("[\\s\\S]*");
    return new RegExp(`^${source}$`).test(url);
  });
}

/** One run of an allowlist entry, as a regular expression matching itself. */
function literally(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * The retry schedule a delivery is worked on, in milliseconds from its intent.
 *
 * The default is `docs/durability.md`'s normative one — five attempts across
 * fifteen minutes — and `AGENT_COMPOSE_CALLBACK_RETRY` overrides it with a
 * comma-separated list of grammar 4.4 durations, which is what a test or a
 * diagnostic run shortens it with. A value that is not one is a **usage error**
 * refused at launch rather than a setting nobody read, which is D50's posture
 * applied to an environment variable: a schedule silently ignored is a
 * deployment that believes its callbacks retry in seconds and finds out
 * otherwise ten minutes later.
 */
function retrySchedule(): readonly number[] {
  const written = process.env[CALLBACK_RETRY];
  if (written === undefined || written === "") return CALLBACK_SCHEDULE;
  const offsets: number[] = [];
  for (const entry of written.split(",")) {
    const matched = /^([0-9]+)(ms|s|m|h)$/.exec(entry.trim());
    const unit = matched === null ? undefined : UNITS[matched[2] ?? ""];
    if (matched === null || unit === undefined) {
      throw new CallbackRetryError(
        `\`${CALLBACK_RETRY}=${written}\` is not a comma-separated list of durations: \`${entry.trim()}\` is not one of \`<integer>ms\`, \`<integer>s\`, \`<integer>m\` or \`<integer>h\` (grammar 4.4)`,
      );
    }
    offsets.push(Number(matched[1]) * unit);
  }
  if (offsets.length === 0) {
    throw new CallbackRetryError(
      `\`${CALLBACK_RETRY}\` names no attempt at all: give it at least one offset, such as \`0s\`, or unset it for the schedule \`docs/durability.md\` states`,
    );
  }
  return offsets;
}

/** What `AGENT_COMPOSE_CALLBACK_RETRY` was set to and could not mean. */
export class CallbackRetryError extends Error {
  constructor(detail: string) {
    super(detail);
    this.name = "CallbackRetryError";
  }
}

/** The variable that shortens the schedule, for a test or a diagnostic run. */
const CALLBACK_RETRY = "AGENT_COMPOSE_CALLBACK_RETRY";

/**
 * The normative schedule of `docs/durability.md` §3.7: five attempts at `+0s`,
 * `+15s`, `+60s`, `+240s` and `+600s` from the intent.
 */
const CALLBACK_SCHEDULE: readonly number[] = [0, 15_000, 60_000, 240_000, 600_000];

/** Grammar 4.4's units, in milliseconds. */
const UNITS: Readonly<Record<string, number>> = {
  ms: 1,
  s: 1_000,
  m: 60_000,
  h: 3_600_000,
};

/**
 * What both the status route and the callback report about an execution.
 *
 * `trace_version` travels **with** the trace and only with it (`docs/trace.md`):
 * a version key describing nothing would be a number a reader could pin against
 * no format at all. Two reports have nothing for it to describe — a run still
 * going, which has recorded nothing yet, and a run that **failed** carrying no
 * trace at all, which is a failure raised before the graph ran — and both carry
 * neither key. The gate is whether a trace exists, not whether it has entries in
 * it: a run that failed *inside* the graph having recorded nothing carries
 * `trace: []` and the version beside it, because an empty trace is a statement
 * about the run and an absent one is not. The rule a reader is given is the one
 * this expresses — wherever a `trace` appears, the version that describes it
 * appears beside it, and wherever one is absent so is the other — and it holds on
 * the two surfaces this function feeds, the status route and the completion
 * webhook, exactly as it does for `run`'s JSON record and the trace file.
 *
 * `interrupts` is the third key that comes and goes, and it is on exactly the
 * report whose `status` is `interrupted`: every pause the execution is holding,
 * with what the human is shown, the schema their answer has to fit, and the URL
 * that delivers it (grammar 8.7). It is what makes a status poll enough to
 * *present* the question rather than only to notice that there is one. Ordered
 * by `wait_id` — `runtime.humanWaits`'s order — so that two runs of one
 * composition publish the same questions in the same order whatever order their
 * instances were scheduled in.
 *
 * `deliveries` is the fourth, and it is on the report of every execution that
 * has made one. It is what makes a **refused** delivery — a callback URL
 * `callback_allow:` admits nowhere — and an **exhausted** one visible rather
 * than silent, which is what resolved q33 and q35 ask of them: neither is the
 * execution's failure, so the run's own `status` says nothing about either and
 * this is the only place a reader can see them. No credential appears in it,
 * and neither does a delivered body: what is published is what happened.
 */
async function report(execution: Execution): Promise<Record<string, unknown>> {
  const waits = execution.status === "running" ? humanWaits(execution.id) : [];
  let delivered: readonly runtime.DeliveryRecord[] = [];
  try {
    delivered = await deliveriesOf(execution.id);
  } catch {
    // A journal this process cannot read is not a reason to refuse the report:
    // what a reader is asking about is the run, and the rest of it is here.
  }
  return {
    execution_id: execution.id,
    flow: execution.flow,
    trigger: execution.trigger,
    status: statusOf(execution),
    ...(waits.length === 0
      ? {}
      : { interrupts: waits.map((wait) => question(execution, wait)) }),
    ...(delivered.length === 0 ? {} : { deliveries: delivered.map(delivery) }),
    ...(execution.outputs === undefined ? {} : { outputs: execution.outputs }),
    ...(execution.error === undefined ? {} : { error: execution.error }),
    ...(execution.trace === undefined
      ? {}
      : { trace_version: TRACE_VERSION, trace: execution.trace }),
  };
}

/**
 * One callback delivery, as the status route publishes it.
 *
 * `snake_case` for [`question`]'s reason: these are document keys. The
 * **body** is not among them — it is the report a receiver was sent, which a
 * reader already has in front of them — and neither is anything
 * `callback_auth:` resolved.
 */
function delivery(record: runtime.DeliveryRecord): Record<string, unknown> {
  return {
    delivery_id: record.id,
    ordinal: record.ordinal,
    event: record.event,
    url: record.url,
    status: record.status,
    intended_at: record.intendedAt,
    ...(record.settledAt === undefined ? {} : { settled_at: record.settledAt }),
    attempts: record.attempts.map((attempt) => ({
      at: attempt.at,
      outcome: attempt.outcome,
      ...(attempt.detail === undefined ? {} : { detail: attempt.detail }),
    })),
    ...(record.detail === undefined ? {} : { detail: record.detail }),
  };
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

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/**
 * Start the app and announce where it is listening.
 *
 * The readiness line is one JSON object on stdout, before anything else is
 * written there — `{"base_url":"http://127.0.0.1:8787"}` — so a caller that asked
 * for `--port 0` learns which port it got without guessing and without racing.
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
  for (const signal of ["SIGINT", "SIGTERM"] as const) {
    process.on(signal, () => {
      void app.close().then(() => process.exit(0));
    });
  }
  return app;
}
