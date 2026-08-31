//
// The hub: the five `/workers/*` routes, the dispatch board behind them, and the
// seam a placed node reaches the mesh through (`docs/distributed.md`, PRD
// resolved q37–q46).
//
// This module is the whole of what "the hub" means in that document. It is
// byte-identical in every project this compiler release builds, like
// `./runtime.ts` and `./serve.ts`; what differs between two deployments is
// `./deployment.ts`, which carries the placements a worker may claim and the
// environment partition §9.1 fixes, and `./artifact.ts`, which carries the tree's
// identity.
//
// # Why it is here and not in `./serve.ts`
//
// The routes join the serve surface — §2 puts them "under the same server, the
// same process, and the same auth story" — but the machinery behind them is a
// scheduler and a board, not a request handler. Keeping it beside the app would
// make the one module a reader goes to for "what does a trigger do" also the
// module they have to read past for "how does a dispatch get superseded".
//
// # The shape, in one paragraph
//
// Work queues to a **placement**, never to a worker (§2, §6.3). A placed node
// calls [`dispatchPlaced`], which puts a row on the journal's dispatch board and
// waits; a worker claiming that placement long-polls, is handed at most one
// unsettled dispatch, streams its effects home, and posts a result. Everything
// durable is the journal's — §8 rule 3 forbids dispatch state anywhere else — and
// everything in memory here is either advisory (the session table, §5) or a
// promise this process is holding for a node it is running.
//
// A result ends a dispatch in one of **three** ways, and the third is why
// [`dispatchPlaced`] is a loop rather than one await: a `human:` node reached on
// a worker settles its dispatch *paused* (§3.4, PRD resolved q46), the wait is
// planted on the hub's one board — `./runtime.ts`'s, the same board a local
// pause goes on — and the answer sends the node back through dispatch with the
// answered pause in its effect history.
//
// # What is deliberately absent
//
// * **No heartbeat route.** A poll is the liveness signal and §3.2 says there
//   MUST NOT be a second one.
// * **No capacity knob.** A session holds exactly one unsettled dispatch. §13
//   files raising that as a question for the PRD rather than a hub's setting, so
//   there is no field, no option and no environment variable for it here; the
//   wire reserves an OPTIONAL join field for whatever the answer turns out to be
//   and this hub neither sends nor reads one.
// * **No containment.** §13's second open row; a worker runs the artifact with
//   its own privileges, and nothing here says otherwise.
// * **No worker-to-worker anything.** Every edge goes through this process.

import { Buffer } from "node:buffer";
import { createHash, timingSafeEqual } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

import type { FastifyInstance, FastifyReply, FastifyRequest } from "fastify";

import { ARTIFACT_FILES, ARTIFACT_HASH, COMPILER_VERSION } from "./artifact.ts";
import { joinTokenEnv, placements } from "./deployment.ts";
import {
  type DispatchRow,
  type EffectKind,
  type Journal,
  type JournalOutcome,
  type JournalRecord,
  effectKey,
  openJournal,
  replayedFailure,
} from "./journal.ts";
import { holdRemotePause, registerParkedWork, registersHumanNode } from "./runtime.ts";
import type * as runtime from "./runtime.ts";

// ---------------------------------------------------------------------------
// The constants a join agrees on (§3.1, §4.1, §10)
// ---------------------------------------------------------------------------

/**
 * The version of `docs/distributed.md` this hub speaks (§10).
 *
 * Carried on every join and compared before anything about the deployment.
 * `compose_core::codegen::mesh::PROTOCOL_VERSION` is the compiler's copy of this
 * number and a test reads this line back to keep the two from drifting — the
 * same cross-language pin the trace format and the journal version already have.
 *
 * **`2` since PRD resolved q46**: a result may settle a dispatch *paused*
 * (§3.4), and a peer of version `1` would read that result as a node that
 * answered with no output rather than as a wait for a person — §10.3's "a change
 * that would make a peer of the previous version behave **wrongly** rather than
 * be refused", exactly. §10 records the bump and why it costs nothing.
 */
export const PROTOCOL_VERSION = 2;

/**
 * The runtime a worker executes this artifact under, as §4.1 compares it: a
 * name and a major version, and nothing finer.
 *
 * "Workers are Bun-only" (§4.2) is a scoped exception to PRD resolved q18's Node
 * fallback — the fallback is about a generated project somebody runs by hand,
 * and this is the surface *around* the artifact. What pins the major is the
 * compiler release, so a worker satisfying `compiler` satisfies this.
 */
const WORKER_RUNTIME = { name: "bun", major: 1 };

/**
 * How large a body the two routes a worker **POSTs** to will take.
 *
 * Fastify's default is a megabyte, and a megabyte is the wrong number for these
 * two: §3.3 and §3.4 give each route a closed table of statuses, and neither
 * table has a `413` in it. A framework answering one outside the table is read
 * by a worker as a refusal, and §10.1 lets it be — so an effect record larger
 * than the default would take a placement down rather than cost a batch, and it
 * is a record this protocol positively expects to see. An effect carries the
 * canonical request *and* the whole outcome (`docs/durability.md` §3), so one
 * long tool loop's `model` call, or a placed `tool.*` answering with a document,
 * is past a megabyte without anything having gone wrong.
 *
 * The number is the artifact's rather than a message's, and it is the same one
 * `crates/agent-compose/src/worker/wire.rs` reads an answer under: the two
 * directions of this wire carry the same things — an `effect_history` out, the
 * effects that grew it back — so a ceiling on one that the other does not have
 * is a mesh that can dispatch what it cannot be told about.
 */
const BODY_LIMIT = 256 * 1024 * 1024;

/** How long a poll is held before it is answered `204` (§2). */
const POLL_HOLD_MS = 25_000;

/** How long a session may go without a request before the hub gives up on it (§2). */
const LIVENESS_WINDOW_MS = 90_000;

/** The variable that shortens the poll hold, for a test or a diagnostic run. */
const POLL_HOLD = "AGENT_COMPOSE_MESH_POLL_HOLD_MS";

/** The variable that shortens the liveness window, for the same reason. */
const LIVENESS_WINDOW = "AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS";

/**
 * What the two timings resolve to in this process.
 *
 * §2 makes them configurable and fixes the one relationship an implementation
 * MUST keep: "a hold shorter than most intermediary idle timeouts and a liveness
 * window several holds wide". A window that is not wider than a hold would
 * declare a worker gone while its own poll was still being held — the hub
 * killing the very request that proves the worker is there — so the pair is
 * refused at launch rather than served, which is the posture `./serve.ts` takes
 * to a retry schedule that is not one.
 */
export function meshTimings(): { readonly holdMs: number; readonly windowMs: number } {
  const holdMs = duration(POLL_HOLD, POLL_HOLD_MS);
  const windowMs = duration(LIVENESS_WINDOW, LIVENESS_WINDOW_MS);
  if (holdMs >= windowMs) {
    throw new MeshTimingError(
      `\`${POLL_HOLD}=${holdMs}\` is not shorter than \`${LIVENESS_WINDOW}=${windowMs}\`: a poll held past the liveness window would let the hub declare the very worker whose request it is holding gone (docs/distributed.md §2)`,
    );
  }
  return { holdMs, windowMs };
}

/** One timing override, in whole milliseconds. */
function duration(variable: string, fallback: number): number {
  const written = process.env[variable];
  if (written === undefined || written.trim() === "") return fallback;
  const parsed = Number(written.trim());
  if (!Number.isSafeInteger(parsed) || parsed <= 0) {
    throw new MeshTimingError(
      `\`${variable}=${written}\` is not a whole number of milliseconds above zero`,
    );
  }
  return parsed;
}

/** What a mesh timing variable was set to and could not mean. */
export class MeshTimingError extends Error {
  constructor(detail: string) {
    super(detail);
    this.name = "MeshTimingError";
  }
}

/** What `hub.join_token:` named and this process cannot use. */
export class MeshCredentialError extends Error {
  constructor(variable: string) {
    super(
      `\`${variable}\` is the join token of this target's \`hub:\` and is set to the empty string: an empty credential admits every worker into the mesh, so this app refuses to serve until it is given a value (docs/distributed.md §3, grammar §14.2)`,
    );
    this.name = "MeshCredentialError";
  }
}

// ---------------------------------------------------------------------------
// Sessions (§5)
// ---------------------------------------------------------------------------

/**
 * One worker session, held **in memory only**.
 *
 * §5 makes sessions advisory: "every re-join re-derives everything from the
 * journal", so nothing here is state the protocol depends on and a restart loses
 * exactly this map and nothing else. Every session-carrying route answers `410`
 * for an id this table does not hold, which is the one status that means "join
 * again" (§3).
 *
 * **§5's redeployment rule needs no code of its own here, and that is a property
 * of v1 rather than an omission.** "A hub that begins serving a new artifact
 * under a stable name MUST forget every session issued under the one it
 * replaced" — and a v1 hub serves exactly one artifact, its own tree, so
 * beginning to serve another *is* replacing this process. The sessions go with
 * it, the workers meet `410` on their next request, and the join that follows is
 * answered with the new artifact. A hub that learned to serve two would need the
 * rule written out; nothing here may quietly become that hub without it.
 */
interface Session {
  readonly id: string;
  /** The placement names this worker claimed (§3.1). */
  readonly claims: readonly string[];
  /**
   * Whether the hub may hand this session a dispatch.
   *
   * `false` for a **provisioning join** — one that carried no `env_ok`, because
   * the worker does not yet hold the artifact whose manifest it would report
   * against (§3.1). Such a session is answered normally and dispatched nothing;
   * the worker fetches, materialises and joins again, and that join is where the
   * `403` fires and where dispatch becomes possible.
   */
  readonly dispatchable: boolean;
  /** When this session last made a request — what the window is measured from. */
  seen: number;
}

/** Every session this process has issued and not forgotten. */
const sessions = new Map<string, Session>();

/** Whoever is waiting for a poll to have something to answer with. */
const pollers = new Set<() => void>();

/**
 * How many times the board has moved in this process.
 *
 * Read **before** a poll looks at the board and compared **after** it has
 * subscribed, which is what closes the window between the two: a dispatch parked
 * in that gap wakes nobody, and without the counter the poll would then wait out
 * its whole hold — twenty-five seconds of latency, in production, for work that
 * was ready the instant the poll asked. The stir is a broadcast rather than a
 * queue because §2 queues to a **placement**: which session a wake is for is
 * decided by re-scanning the board, never by who was woken.
 */
let boardMoved = 0;

/**
 * Wake every held poll, because the board may have something for it now.
 *
 * Called at **both** kinds of move, and the second is as load-bearing as the
 * first. Work arriving is one: a wait that just parked is work a held poll asked
 * about a moment too early. Work *leaving* is the other: a session is answered
 * nothing while it holds an unsettled dispatch (§2), so the instant a dispatch
 * stops being unsettled — settled by a result, superseded by a deadline, dropped
 * with the execution — the session behind it became eligible for the next item
 * in the queue. A stir that only ever announced arrivals would drain a queue at
 * one item per hold: §2 promises "a small latency floor on dispatch — one round
 * trip after the hold is answered", and a `map` of eight onto a pool of one would
 * spend seven holds idling instead.
 */
function stirPolls(): void {
  boardMoved += 1;
  for (const wake of [...pollers]) wake();
}

// ---------------------------------------------------------------------------
// The dispatch board, from the node's side (§6, §7)
// ---------------------------------------------------------------------------

/** What a placed node's dispatch answered, once a worker settled it. */
export interface PlacedAnswer {
  /** The node's result object, which its `writes:` map into channels. */
  readonly output: unknown;
  /** What it contributes to the shared conversation history (grammar §10.4). */
  readonly history?: readonly unknown[];
  /** Which member of its route served each model call (PRD 5.9). */
  readonly models?: readonly runtime.ModelCall[];
  /** The subflows a model invoked (PRD §9.20). */
  readonly toolDispatches?: readonly runtime.DispatchRecord[];
  /**
   * What its store ops did (PRD 5.8), for the trace entry the **hub** writes.
   *
   * The fourth collector a node execution fills, and the one that has to travel
   * because it has no other way home: the three above are returned by the node
   * function itself, while store records are pushed into
   * `runtime.RunContext.storeRecords` as they happen — an array a worker's
   * process holds a copy of and the hub's node execution never sees. Left
   * behind, a placed agent's `stores:` would be missing from the entry that
   * reports it and present on the same agent unplaced, which is a placement
   * changing what a run *reports* (§4.3, PRD 5.6).
   */
  readonly stores?: readonly runtime.StoreRecord[];
}

/**
 * Where a placed node's activity sits, as the process that runs it addresses it.
 *
 * The same two facts an ordinary dispatched instance carries
 * (`runtime.DispatchSite`): the frames grammar §9.4 keys effects by, and the
 * execution identity the node's own bindings read.
 */
export interface PlacedSite {
  readonly path: readonly string[];
  readonly execution: runtime.ExecutionIdentity;
  /**
   * The conversation turns an `agent:` node is given (grammar §10.4), where the
   * node takes any — §3.2's `history`.
   *
   * Absent for every dispatch that is not an `agent:` node, and for a
   * `map`-dispatched agent, which grammar §8.6 rule 10 runs on a fresh
   * conversation of its own (D105). Absent is **empty**, and never "work it out
   * here": a worker has no `messages` channel to read.
   */
  readonly history?: readonly runtime.Turn[];
  /**
   * Grammar §9.3's level 1 for this instance — §3.2's `policy`.
   *
   * What crosses into a subflow an attached `flow.*` starts (D79), which is the
   * half a placed agent would otherwise lose: its tool loop would instantiate
   * that subflow under a different retry/timeout ladder from the one the same
   * agent unplaced instantiates it under.
   */
  readonly policy?: runtime.InstancePolicy;
}

/**
 * What a placed component's node **does**, where it is executed rather than
 * dispatched.
 *
 * `./graph.ts` exports one of these per placed call site, keyed by the address
 * a dispatch names, and `./worker-node.ts` is what calls them
 * (`docs/distributed.md` §3.2). The hub never does: on this side of the wire a
 * placed node is [`dispatchPlaced`], and the registry is carried in the artifact
 * because the artifact is what a worker is served (§4).
 */
export type PlacedRun = (
  input: unknown,
  context: runtime.RunContext,
  site: PlacedSite,
) => Promise<PlacedAnswer>;

/**
 * How a process that is **executing** placed nodes answers a dispatch.
 *
 * `undefined` in a hub, which is every process that mounts these routes: a
 * dispatch there goes on the board and waits for a worker. Set in a worker's
 * node runner ([`executeLocally`]), where the only honest answer to "run this
 * placed node" is to run it.
 */
type PlacedExecutor = (options: DispatchOptions) => Promise<PlacedAnswer>;

/**
 * What a placed node's call site knows about the node it is dispatching.
 *
 * The four after `inputs` are §3.2's four OPTIONAL payload fields, and they are
 * options rather than something the far side derives for the reason §3.2 gives:
 * each is a fact the **hub** holds about this node execution that the node would
 * have read for itself had it run here, so a worker that defaulted one would be
 * running a different node (§4.3, PRD 5.6).
 */
interface DispatchOptions {
  readonly placement: string;
  readonly node: string;
  readonly execution: string;
  readonly path: readonly string[];
  readonly inputs: unknown;
  /** Grammar §4.1's `execution.item_index`, where a `map` encloses this node. */
  readonly itemIndex?: number;
  /** [`PlacedSite.history`]. */
  readonly history?: readonly runtime.Turn[];
  /** [`PlacedSite.policy`]. */
  readonly policy?: runtime.InstancePolicy;
  readonly signal?: AbortSignal;
  /**
   * The node execution's own store-record collector — `context.storeRecords`.
   *
   * Where [`PlacedAnswer.stores`] is emptied into, which is what puts a placed
   * node's store ops on the same trace entry an unplaced one puts them on
   * (`runtime.runNode` reads the array, not the answer). Absent exactly where
   * the context's own is: a detached `map` delivery, whose records are dropped
   * by design (D94).
   */
  readonly stores?: runtime.StoreRecord[];
}

/** See [`executeLocally`]. */
let executor: PlacedExecutor | undefined;

/**
 * Run placed nodes **here** instead of dispatching them.
 *
 * Called once, by `./worker-node.ts`, and by nothing else. What it is for is
 * nesting: grammar §14.1 rule 4 colocates whatever an agent attaches, so a
 * placed agent whose attached `flow.*` reaches another placed component reaches
 * one of *this worker's own* placement — and the node the compiler lowered for
 * it is [`dispatchPlaced`], because a component is lowered once for both sides
 * of the wire (§4.3: the whole artifact is everywhere, so a placement decides
 * which process runs a node rather than which code exists where). Without this
 * seam that inner node would ask a worker to find a mesh to join, from inside
 * the placement it is already holding.
 */
export function executeLocally(run: PlacedExecutor): void {
  executor = run;
}

/** One placement wait, as a status report publishes it. */
export interface PlacementWait {
  /**
   * `<instance path>/<ordinal>` — the identity §6.1 fixes.
   *
   * **Deterministic**, which is the whole of what §6.1 asks of it: a resumed
   * execution re-parks under the identity it parked under before, so a report
   * taken from one process names the same wait as a report taken from the one
   * that replaced it. The `dispatch_id` beside it is not — it is the issuing
   * hub's own handle (§10.1) — which is why the two are separate fields rather
   * than one.
   */
  readonly id: string;
  /** `dsp_…` — what a poll answers with and a result is attributed by (§3.4). */
  readonly dispatch: string;
  readonly execution: string;
  readonly placement: string;
  readonly node: string;
  readonly site: string;
  /** When it went on the board, which is also the park order (§6.2). */
  readonly parkedAt: string;
  /** Whether a session is holding it, or it is still waiting for one. */
  readonly status: "parked" | "dispatched";
}

/** One dispatch this process is holding a node's promise for. */
interface Awaiting {
  readonly execution: string;
  readonly site: string;
  readonly settle: (outcome: JournalOutcome) => void;
  /** Whether a worker is holding it, for [`parkedWork`]. */
  taken: boolean;
}

/**
 * How one dispatch ended, from the node's side.
 *
 * §3.4 gives a dispatch **three** endings, and the third is what PRD resolved
 * q46 added: a result may settle a dispatch *paused*, carrying the wait a
 * `human:` node opened on the worker. Paused is a way of being **settled** — the
 * dispatch ended with a result, the session is free, and a re-post is `204` — so
 * it is a shape of the settled outcome rather than a fourth `DispatchStatus`.
 * What the node does with it is [`dispatchPlaced`]'s loop: plant the wait, wait
 * for the answer, and re-enter dispatch with the answered pause in the history.
 */
type Ending =
  | { readonly kind: "answer"; readonly answer: PlacedAnswer }
  | { readonly kind: "paused"; readonly pause: runtime.RemotePause };

/** The dispatches this process is awaiting, by `dispatch_id`. */
const awaiting = new Map<string, Awaiting>();

/** Next placement-wait ordinal per execution and instance path (§6.1). */
const ordinals = new Map<string, Map<string, number>>();

/** Who is told when one execution's placement waits move. */
const waitWatchers = new Map<string, Set<() => void>>();

/**
 * Whether this process can dispatch at all — that is, whether an app has mounted
 * the routes a worker joins through.
 *
 * `agent-compose run` builds no app, so a placed node reached there has no mesh
 * to reach: the run is refused at the node with [`PlacementUnreachable`], which
 * is the shape `runtime.HumanInterrupt` already has for a pause a run has no way
 * to answer. The alternative — parking on a board nothing will ever poll — is a
 * command that never returns.
 */
let mounted = false;

/** A placed node reached by a run with no mesh to dispatch it into. */
export class PlacementUnreachable extends Error {
  readonly placement: string;

  constructor(node: string, placement: string) {
    super(
      `\`${node}\` is placed on \`${placement}\`, and this invocation has no mesh for a worker to join: a placed node executes on whichever worker claims its placement, so it needs a served hub with a worker beside it rather than a single process (docs/distributed.md §1, §3)`,
    );
    this.name = "PlacementUnreachable";
    this.placement = placement;
  }
}

/** The hub gave up on the session holding a dispatch, and the attempt fails (§6.3). */
export class DispatchSuperseded extends Error {
  constructor(node: string, placement: string, detail: string) {
    super(`\`${node}\` on \`${placement}\`: ${detail}`);
    this.name = "DispatchSuperseded";
  }
}

/**
 * Run one placed node **somewhere else**: journal the dispatch, wait for a
 * worker to settle it, and answer as if the node had run here (§3.2, §7).
 *
 * The whole of what makes this a *dispatch-and-await* rather than a call is the
 * journal row. Three things read it and each is a rule of the document:
 *
 *  * a **resumed hub** reaches this node again and finds the row its predecessor
 *    left, under the identity §6.1 fixes — settled, and the answer is consumed
 *    rather than the work redone; settled *paused*, and the wait is re-derived
 *    onto this hub's board unless its answer is already journaled; superseded,
 *    and the failure is replayed so the node's `retry:` ladder does now what it
 *    did then; still open, and this process goes on holding it;
 *  * a **worker** is handed the row's inputs and its `effect_history`, and
 *    replays to the frontier before going live (§7.2);
 *  * the **liveness sweep** supersedes it where the session holding it stopped
 *    making requests (§6.3), which fails this attempt under the node's own
 *    `retry:`/`on_error:` chain exactly as a local failure would.
 *
 * The node's `timeout:` is **not** held still while this waits. §6.5 makes the
 * chain run from dispatch and §2 says so outright — "a `timeout:` on a placed
 * node is a bound on queueing plus execution, not on execution alone" — which is
 * the opposite of a `human` node's rule (D102) and is why `runtime.pausesUnder`
 * is left alone and only `runtime.quiescent` learns about placement waits.
 */
export async function dispatchPlaced(options: DispatchOptions): Promise<PlacedAnswer> {
  // A process that executes placed nodes answers this itself ([`executeLocally`]),
  // and is asked before anything is journaled: a worker holds no board, and the
  // journal of this execution is the hub's alone (§3.3, §8 rule 3).
  if (executor !== undefined) return absorb(options, await executor(options));
  const site = options.path.join("/");
  if (!mounted) throw new PlacementUnreachable(options.node, options.placement);
  const journal = await openJournal();
  // **A loop, because a pause is not the end of the node** (§3.4, PRD resolved
  // q46). A dispatch that settles paused hands this node a question rather than
  // an answer: the wait goes on the hub's board under the identity the worker
  // derived, the answer is journaled as the pause's own effect record, and the
  // node **re-enters dispatch** — a fresh row at the next ordinal of this
  // instance path, parking again if nothing claims the placement (resolved q39),
  // and carrying the answered pause in its `effect_history` so the redispatched
  // node replays past the question rather than asking it again (§7.2).
  //
  // Every other ending leaves this loop on its first turn, which is why the two
  // reads that decide it are the first thing after the park.
  for (;;) {
    const wait = `${site}/${nextOrdinal(options.execution, site)}`;
    const row = journal.park({
      execution: options.execution,
      wait,
      id: `dsp_${globalThis.crypto.randomUUID()}`,
      placement: options.placement,
      node: options.node,
      site,
      inputs: options.inputs,
      // §3.2's OPTIONAL payload fields, journaled beside `inputs` and for its
      // reason: the row **is** the dispatch (§8 rule 3), so the poll answer is
      // read out of the row at hand-over rather than out of this process's
      // memory, and a hub restarted mid-dispatch hands over what its predecessor
      // would.
      ...(options.itemIndex === undefined ? {} : { itemIndex: options.itemIndex }),
      ...(options.history === undefined ? {} : { history: options.history }),
      ...(options.policy === undefined ? {} : { policy: options.policy }),
      status: "parked",
      parkedAt: new Date().toISOString(),
    });

    // A row a previous generation already finished with. Consumed rather than
    // redone, which is `docs/durability.md` §5's replay discipline reaching the
    // one effect this module owns.
    if (row.status === "settled") {
      const ending = endingOf(row);
      if (ending.kind === "answer") return absorb(options, ending.answer);
      // **A pause a dead hub was holding**, re-derived rather than remembered
      // (§5's first rule). Which of the two things this generation owes it is
      // decided by the journal and by nothing else: an answered pause has its
      // effect record, and the wait is over — the redispatch below is what
      // replays past it. A pause with no record is one nobody answered, so this
      // process plants it again, under the identity and the instants its
      // predecessor published (`docs/durability.md` §9) — and spending what is
      // left of the wait's budget, which is why the row's own `settled_at` is
      // what the timer is armed from rather than this process's start.
      if (journal.lookup(options.execution, ending.pause.effect.key) === undefined) {
        await answered(journal, options, ending.pause, row.settledAt ?? new Date().toISOString());
      }
      continue;
    }
    if (row.status === "superseded") {
      throw new DispatchSuperseded(
        options.node,
        options.placement,
        row.detail ?? "the hub ended this dispatch without a result",
      );
    }
    // …and a row some dead process had handed to a session: sessions are this
    // process's only (§5), so one it did not issue is one whose holder this hub
    // has ended. **Superseded, never put back on the board** — see
    // [`supersedeOrphans`], which is where the same rule is applied at start and
    // where the reasoning is. This is its belt-and-braces: a row that reached
    // here still `dispatched` is one that start could not read.
    if (row.status === "dispatched" && !sessions.has(row.session ?? "")) {
      journal.supersedeDispatch(row.id, ORPHANED);
      throw new DispatchSuperseded(options.node, options.placement, ORPHANED);
    }
    rows.set(row.id, row);

    const ending = await awaited(journal, options, row, site);
    if (ending.kind === "answer") return absorb(options, ending.answer);
    // The instant the paused result settled the row, read back off it rather
    // than taken here: it is the same field a restarted process would re-read
    // above, so one wait's budget is spent from one instant however many
    // processes hold it.
    await answered(
      journal,
      options,
      ending.pause,
      journal.dispatchOf(row.id)?.settledAt ?? new Date().toISOString(),
    );
  }
}

/**
 * Hold this process's promise for one dispatch, until a worker settles it or the
 * hub ends it.
 *
 * Everything about the wait that is not the loop above: the entry the status
 * report and the parked-work counter read, the abort that supersedes a dispatch
 * whose node ran out of time, and the stir that hands a held poll the wait that
 * has just gone on the board.
 */
function awaited(
  journal: Journal,
  options: DispatchOptions,
  row: DispatchRow,
  site: string,
): Promise<Ending> {
  return new Promise<Ending>((resolve, reject) => {
    const held: Awaiting = {
      execution: options.execution,
      site,
      taken: row.status === "dispatched",
      settle: (outcome) => {
        finish();
        if (outcome.kind === "error") reject(replayedFailure(outcome));
        else resolve(endingIn(outcome.value));
      },
    };
    const abort = (): void => {
      // The node's own deadline ran out (§6.5: the chain runs from dispatch, so
      // queueing is inside the budget). The hub is done with the dispatch, so
      // the row is ended rather than left for a worker to take work nothing
      // will read the answer of. **Synchronously**, and the journal is the
      // caller's for that reason: a row left `parked` for one turn of the loop
      // is a row a poll in that turn could claim, handing a worker work whose
      // answer nothing will read.
      const reason =
        options.signal?.reason instanceof Error
          ? options.signal.reason.message
          : "the node execution ended";
      journal.supersedeDispatch(row.id, `the hub stopped waiting for this dispatch: ${reason}`);
      rows.delete(row.id);
      held.settle({ kind: "error", name: "DispatchSuperseded", message: reason });
      // A dispatch that has stopped being unsettled frees whichever session was
      // holding it, and the queue behind it is what that session is answered
      // next — see [`stirPolls`].
      stirPolls();
    };
    const finish = (): void => {
      awaiting.delete(row.id);
      options.signal?.removeEventListener("abort", abort);
      announceWaits(options.execution);
    };
    if (options.signal?.aborted === true) {
      abort();
      return;
    }
    options.signal?.addEventListener("abort", abort, { once: true });
    awaiting.set(row.id, held);
    announceWaits(options.execution);
    // A worker may already be holding a poll: this is a wait that has just gone
    // on the board, and §6.2's wake is "a joining worker's claims are scanned
    // against the open placement-waits" — which a held poll re-runs when stirred.
    stirPolls();
  });
}

/**
 * Plant one worker's pause on **this hub's** wait board, wait for it to be
 * answered, and journal the answer as the pause's own effect record
 * (§3.4, PRD resolved q46).
 *
 * The two halves are the whole of what q46 asks for, and neither is new
 * machinery: `runtime.holdRemotePause` puts the wait on the one board every
 * local pause goes on — so the status route publishes it, the resume route
 * answers it, the `parked` webhook fires for it and the dispatching node's
 * deadline is held still while it waits — and the record below is exactly the
 * one `runtime.runHuman` writes for a pause the hub held itself, at the key the
 * worker's own recorder claimed. The redispatch after it hands that record back
 * in `effect_history` (§7.2), and the replay consumes it at the very claim that
 * paused: the node goes live *past* the question, re-issuing nothing already
 * paid for.
 *
 * **The write is the node's to fail on**, exactly as it is locally: a journal
 * that will not take the answer leaves a run that would ask the person again on
 * its next resume (`docs/durability.md` §3.4), so the throw travels as this
 * node's failure rather than being swallowed.
 *
 * `since` is when **this hub** took the pause, on this hub's clock: the row's
 * `settled_at`, which is the instant the paused result settled the dispatch and
 * the instant a restarted process re-reads. It is what the wait's `timeout:` is
 * spent from — never the worker-stamped `expires_at`, which is another machine's
 * clock and is carried for a reader rather than for a timer
 * (`runtime.holdRemotePause`).
 */
async function answered(
  journal: Journal,
  options: DispatchOptions,
  pause: runtime.RemotePause,
  since: string,
): Promise<void> {
  const settled = await holdRemotePause(options.execution, pause, since);
  journal.append({
    execution: options.execution,
    key: pause.effect.key,
    site: pause.effect.site,
    kind: "human",
    ordinal: pause.effect.ordinal,
    // The identity the **worker's** recorder derived, carried home on the pause
    // rather than derived a second time here: a second derivation is a
    // `ReplayDivergence` on the redispatch the day the two spellings part.
    request: pause.effect.request,
    outcome: { kind: "value", value: settled },
    refused: false,
    recordedAt: new Date().toISOString(),
  });
}

/** The next placement-wait ordinal at one instance path (§6.1). */
function nextOrdinal(execution: string, site: string): number {
  let counters = ordinals.get(execution);
  if (counters === undefined) {
    counters = new Map();
    ordinals.set(execution, counters);
  }
  const ordinal = counters.get(site) ?? 0;
  counters.set(site, ordinal + 1);
  return ordinal;
}

/**
 * Put what a placed node did into the collectors of the node execution that
 * dispatched it, and answer it.
 *
 * The one field of a [`PlacedAnswer`] the caller cannot read off the return
 * value, because `runtime.runNode` does not read store records off an answer:
 * it reads the array it handed the activity, on both of its ways out. So the
 * hub's copy of that array is where a worker's records have to land, and this is
 * the one place a dispatch's answer is produced — the three ways it can be
 * (executed locally, replayed off a settled row, settled by a result) all come
 * through here.
 *
 * Applied on a **replayed** row too, and deliberately: the entry this generation
 * writes is a fresh entry, and a resumed execution that dropped the records
 * would report a node that did nothing to its stores where its predecessor
 * reported one that did (`docs/durability.md` §5).
 */
function absorb(options: DispatchOptions, answer: PlacedAnswer): PlacedAnswer {
  if (answer.stores !== undefined && options.stores !== undefined) {
    options.stores.push(...answer.stores);
  }
  return answer;
}

/** Why a dispatch a replaced process was holding is over. See [`supersedeOrphans`]. */
const ORPHANED =
  "the hub process holding this dispatch was replaced, so the session it was issued to ended (docs/distributed.md §5)";

/** How a settled row ended: with the node's answer, or with a pause. */
function endingOf(row: DispatchRow): Ending {
  const outcome = row.outcome;
  if (outcome === undefined) {
    throw new Error(`\`${row.id}\` is settled and carries no outcome`);
  }
  if (outcome.kind === "error") throw replayedFailure(outcome);
  return endingIn(outcome.value);
}

/**
 * The same reading of a settled outcome's **value**, for the live path.
 *
 * A paused settlement is `{ paused: … }` and a node's answer is a
 * [`PlacedAnswer`], which carries `output` and never `paused` — so the key is
 * the discriminant, and it is written by [`pauseOf`] alone: nothing a worker
 * sends reaches this shape except through that reader.
 */
function endingIn(value: unknown): Ending {
  if (value !== null && typeof value === "object") {
    const paused = (value as Record<string, unknown>)["paused"];
    if (paused !== undefined) {
      return { kind: "paused", pause: paused as runtime.RemotePause };
    }
  }
  return { kind: "answer", answer: value as PlacedAnswer };
}

/**
 * One paused settlement off the wire, or `undefined` where it is not one
 * (§3.4).
 *
 * Read whole and refused whole, the way [`recordOf`] reads an effect record and
 * for a sharper version of the same reason: a pause short of its wait identity
 * is a wait no resume could address, and one short of its effect fields is an
 * answer the journal could not record — so a half-read pause would put a
 * question on the board that nothing could ever take off it.
 *
 * **Held to the dispatch's own instance path**, both halves of it, which is
 * §3.3's rule about a record's `site` reaching the one route that also plants a
 * wait: the hub is the single writer (§8), so no session may journal an effect
 * into a node it was never dispatched — and no session may put a question on the
 * board under another node's identity, which is the same rule about the other
 * ledger. `row` is what the hub already holds; nothing here is read off the body.
 *
 * **And the `key` is held to it too**, because the key is what the record is
 * actually written under (`answered`) and a `site` inside this dispatch with a
 * key outside it would journal a `human` record into some other node's effect
 * slot — where that node's own replay claims it and raises a `ReplayDivergence`
 * an execution this session was never dispatched into cannot absorb. An effect
 * key is `<site>#<kind>/<ordinal>` (`./journal.ts`'s `EffectRecorder.claim`), so
 * the check is the derivation: this pause's own site, `human`, and the ordinal it
 * carries. Nothing is *taken* from the derivation — the worker's spelling is what
 * travels and what is journaled, since a second derivation is the divergence §3.4
 * carries the field to avoid — but a spelling that is not the one this hub would
 * have written is not a pause it can read.
 */
function pauseOf(row: DispatchRow, value: unknown): runtime.RemotePause | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const held = value as Record<string, unknown>;
  const wait = held["wait"];
  const flow = held["flow"];
  const node = held["node"];
  const pausedAt = held["paused_at"];
  const shown = held["shown"];
  if (typeof wait !== "string" || wait === "") return undefined;
  if (typeof flow !== "string" || typeof node !== "string") return undefined;
  // A node this artifact declares, asked here rather than where the wait is
  // planted: the answer a person gives is held to that node's `output:`, so a
  // pause naming a node the hub does not register is a question nothing could
  // validate an answer against — and the honest cost of it is the dispatch, one
  // failure with one name, rather than a raw throw out of a settlement the
  // route has already answered `204`.
  if (!registersHumanNode(`${flow}.${node}`)) return undefined;
  if (typeof pausedAt !== "string") return undefined;
  if (shown === null || typeof shown !== "object" || Array.isArray(shown)) return undefined;
  const effect = held["effect"];
  if (effect === null || typeof effect !== "object") return undefined;
  const record = effect as Record<string, unknown>;
  const key = record["key"];
  const site = record["site"];
  const ordinal = record["ordinal"];
  const request = record["request"];
  if (typeof key !== "string" || typeof site !== "string" || typeof request !== "string") {
    return undefined;
  }
  if (typeof ordinal !== "number" || !Number.isInteger(ordinal)) return undefined;
  if (!under(wait, row.site) || !under(site, row.site)) return undefined;
  if (key !== effectKey(site, "human", ordinal)) return undefined;
  const expiresAt = held["expires_at"];
  return {
    wait,
    flow,
    node,
    shown: shown as Record<string, unknown>,
    pausedAt,
    ...(typeof expiresAt === "string" ? { expiresAt } : {}),
    effect: { key, site, ordinal, request },
  };
}

/**
 * Drop everything one execution's placement waits held, however the run ended.
 *
 * The counterpart of `runtime.releaseHumanWaits`, called from the same place and
 * for the same reason: a dispatch nothing is waiting for would still be work a
 * worker could take, and its result would be posted against a node execution
 * that is gone. What is **not** touched is a row this process is not holding a
 * promise for — a hub that is being restarted leaves those exactly as they are,
 * which is what lets the process that replaces it pick them up (§5).
 */
export function releasePlacementWaits(execution: string): void {
  ordinals.delete(execution);
  const ended = "the execution this dispatch belonged to ended";
  let freed = false;
  for (const [id, held] of [...awaiting]) {
    if (held.execution !== execution) continue;
    freed = true;
    rows.delete(id);
    // Settled as a failure rather than left pending: the promise belongs to a
    // node execution nothing is waiting for any more, and a promise nobody
    // resolves is a `serve` process holding one per abandoned branch.
    held.settle({ kind: "error", name: "DispatchSuperseded", message: ended });
    void openJournal()
      .then((journal) => {
        journal.supersedeDispatch(id, ended);
      })
      .catch(() => {
        // A journal this process cannot write is not a reason to fail a run that
        // has already stopped: the row stays as it is and a later start reads it.
      });
  }
  announceWaits(execution);
  // A session that was holding one of these is free now, and the queue behind it
  // is what it is answered next (see [`stirPolls`]).
  if (freed) stirPolls();
}

/**
 * The placement waits one execution is holding, in park order.
 *
 * Read off **this process's** awaiting map rather than off the journal, for the
 * reason `runtime.humanWaits` is read off the board: a report is about the run
 * this process is holding, and the journal's rows include ones a predecessor
 * left that nothing here is waiting for.
 */
export function placementWaits(execution: string): readonly PlacementWait[] {
  const found: PlacementWait[] = [];
  for (const [id, held] of awaiting) {
    if (held.execution !== execution) continue;
    const row = rows.get(id);
    found.push({
      // The journal's identity, not this process's handle: see [`PlacementWait`].
      id: row?.wait ?? held.site,
      dispatch: id,
      execution,
      placement: row?.placement ?? "",
      node: row?.node ?? "",
      site: held.site,
      parkedAt: row?.parkedAt ?? "",
      status: held.taken ? "dispatched" : "parked",
    });
  }
  return found.sort(byParkOrder);
}

/**
 * Park order, as §6.2 means it and as `journal.unsettledDispatches` sorts by:
 * when it went on the board, and then the order it went on in.
 *
 * **The tiebreak is the board's own**, not a second rule that resembles it.
 * `parkedAt` is milliseconds and a fan-out parks its instances from one
 * synchronous burst, so the tie is the ordinary case; `./journal.ts` breaks it
 * with `ORDER BY parked_at ASC, rowid ASC`, and `DispatchRow.order` is that
 * rowid carried on the row so this comparison can be the same comparison. A
 * report that ordered a draining queue by anything else could name the next item
 * as one other than the one the hub will hand out.
 *
 * `natural` is the fallback for a wait whose row this process has not cached —
 * and it is a *natural* compare rather than `<` because a wait id is `<instance
 * path>/<ordinal>` and both halves carry integers, so a string compare puts
 * `sign/10` before `sign/2` and `fan/0/11/0` before `fan/0/2/0`.
 */
function byParkOrder(left: PlacementWait, right: PlacementWait): number {
  if (left.parkedAt !== right.parkedAt) return left.parkedAt < right.parkedAt ? -1 : 1;
  const here = rows.get(left.dispatch)?.order;
  const there = rows.get(right.dispatch)?.order;
  if (here !== undefined && there !== undefined && here !== there) return here < there ? -1 : 1;
  return natural(left.id, right.id);
}

/** Compare two `/`-separated paths, reading a run of digits as the number it is. */
function natural(left: string, right: string): number {
  const here = left.split("/");
  const there = right.split("/");
  for (let index = 0; index < Math.max(here.length, there.length); index += 1) {
    const one = here[index];
    const other = there[index];
    if (one === undefined) return -1;
    if (other === undefined) return 1;
    if (one === other) continue;
    const first = Number(one);
    const second = Number(other);
    if (Number.isInteger(first) && Number.isInteger(second)) return first < second ? -1 : 1;
    return one < other ? -1 : 1;
  }
  return 0;
}

/**
 * The rows this process has parked, by `dispatch_id`.
 *
 * A read-through cache of the journal for the two synchronous readers that
 * cannot await one — the parked-work counter `runtime.quiescent` calls and the
 * status report. The journal stays the authority: nothing is decided from this
 * map that is not also written there.
 */
const rows = new Map<string, DispatchRow>();

/**
 * Be told when one execution's placement waits move; answers the unsubscribe.
 *
 * `./serve.ts` subscribes beside `runtime.watchHumanPauses`, so a quiescence
 * whose only open wait is "waiting for the Mac" fires the `parked` lifecycle
 * webhook exactly as one waiting for a human does (§6.6, PRD resolved q34).
 */
export function watchPlacementWaits(execution: string, listener: () => void): () => void {
  let watchers = waitWatchers.get(execution);
  if (watchers === undefined) {
    watchers = new Set();
    waitWatchers.set(execution, watchers);
  }
  const held = watchers;
  held.add(listener);
  return () => {
    held.delete(listener);
    if (held.size === 0 && waitWatchers.get(execution) === held) waitWatchers.delete(execution);
  };
}

/** Tell this execution's watchers that its set of placement waits moved. */
function announceWaits(execution: string): void {
  const watchers = waitWatchers.get(execution);
  if (watchers === undefined) return;
  for (const watcher of [...watchers]) watcher();
}

/**
 * How many placement waits `execution` is holding at or inside `site` that
 * **no worker has taken**.
 *
 * Registered with `runtime.registerParkedWork` at module scope, so
 * `runtime.quiescent` counts a node waiting *for* a worker as parked rather than
 * as work still advancing — which is what makes §6.6's webhook fire for a
 * placement and what stops a `parked` delivery from being taken while a sibling
 * branch is still running.
 *
 * **A dispatch a worker is holding is not one of them**, and that is §6.4's own
 * line: "no worker has taken the node yet" is the pause, and "a worker took it"
 * is a node that is *running* — somewhere else, but running, and an execution
 * with one in flight has not stopped advancing on its own. Counting a taken
 * dispatch would make `runtime.quiescent` true for the whole four minutes a
 * placed build takes, so a `human` pause opening on a parallel branch would fire
 * a `parked` delivery reporting a run that is mid-node as parked.
 */
registerParkedWork((execution, site) => {
  let parked = 0;
  for (const held of awaiting.values()) {
    if (held.execution !== execution || held.taken) continue;
    if (held.site === site || held.site.startsWith(`${site}/`)) parked += 1;
  }
  return parked;
});

// ---------------------------------------------------------------------------
// The routes (§3)
// ---------------------------------------------------------------------------

/**
 * Mount the five routes of §3 on the served app, where this target has
 * placements to mount them for.
 *
 * A composition that places nothing mounts nothing: there is no mesh, no session
 * table and no board, and the app is exactly the app it was before this module
 * existed. That is also why a trigger may claim a `/workers/*` path on such a
 * target and not on a mesh one — where the routes are mounted, a colliding
 * trigger makes the app refuse to start with `FST_ERR_DUPLICATED_ROUTE`, which
 * is what two colliding triggers already do (`./serve.ts`, grammar §13.3).
 */
export function mountWorkerRoutes(app: FastifyInstance): void {
  if (placements.length === 0) return;
  if (joinTokenEnv === undefined) {
    // Unreachable over a composition `validate` accepted — grammar §14.2 makes
    // `hub.join_token:` required wherever placements are — and said rather than
    // assumed, because an unauthenticated mesh is the one failure this whole
    // module has no way to report later.
    throw new Error(
      "this target declares `placements:` and no `hub.join_token:`, so there is no credential to authenticate a worker with (grammar §14.2)",
    );
  }
  if (credential() === "") throw new MeshCredentialError(joinTokenEnv);
  // Read before a route exists, so a pair that cannot be served is a command
  // that could not be run rather than a window nobody notices.
  const timings = meshTimings();
  mounted = true;

  // Registered **before** `./serve.ts`'s own recovery hook, so the rows a dead
  // process left `dispatched` are ended before the replays that reach them start
  // (§5: sessions are this process's, so a row naming one it never issued is a
  // row whose session has ended).
  app.addHook("onReady", async () => {
    await supersedeOrphans();
  });

  app.post("/workers/join", async (request, reply) => join(request, reply));
  app.get("/workers/poll", async (request, reply) => poll(request, reply, timings.holdMs));
  // The two a worker POSTs to carry [`BODY_LIMIT`] rather than the framework's
  // megabyte: a `413` is a status §3.3 and §3.4 do not give, and a worker acting
  // on one outside their tables is a placement lost over one large record.
  app.post("/workers/effects", { bodyLimit: BODY_LIMIT }, async (request, reply) =>
    effects(request, reply),
  );
  app.post("/workers/result", { bodyLimit: BODY_LIMIT }, async (request, reply) =>
    result(request, reply),
  );
  app.get("/workers/artifact/:hash", async (request, reply) => artifact(request, reply));

  sweeping(timings.windowMs);
}

/**
 * End every dispatch a replaced process was holding, without a result.
 *
 * A `dispatched` row names a session, sessions live in memory (§5), and this
 * process has issued none yet — so every such row was issued by the process this
 * one replaced, and §5 is explicit about what that means: "an unsettled dispatch
 * on an ended session is superseded exactly as §6.3 supersedes one, so its
 * node's attempt fails under that node's `retry:`/`on_error:` chain". The
 * sessions of a replaced process are ended by definition.
 *
 * **Putting them back on the board instead would be the one thing this protocol
 * is written to prevent.** A worker two hundred seconds into a placed node knows
 * nothing about the restart: its poll meets `410`, it joins again, and a re-
 * parked row is then handed to whichever session claims that placement —
 * possibly the very worker still running it, since `holding` is read off rows
 * *this* process has handed out and it has handed out none. Two executions of
 * one instance path would then run at once, the second handed an `effect_history`
 * that does not yet hold what the first is issuing, and the model call §7.3
 * promises is not paid for twice is paid for twice.
 *
 * What it costs is one attempt per in-flight placed node, which is exactly what
 * §6.3 costs for the same reason (a dispatch nobody can be shown to be holding)
 * and what §6.4 tells an author to answer with a `retry:`. The effects the
 * superseded attempt streamed home are in the journal, so the retry replays them
 * rather than re-issuing them (§7.2), and a late result from the worker that was
 * running it meets the `409` §3.4 gives a superseded dispatch.
 */
async function supersedeOrphans(): Promise<void> {
  try {
    const journal = await openJournal();
    for (const row of journal.unsettledDispatches()) {
      if (row.status === "dispatched") journal.supersedeDispatch(row.id, ORPHANED);
    }
  } catch (error) {
    // A journal this process cannot read is not a reason to refuse to serve:
    // the app starts, and the rows are picked up by whatever start can read it.
    process.stderr.write(`this project's dispatch board could not be read: ${message(error)}\n`);
  }
}

/** `POST /workers/join` (§3.1). */
async function join(request: FastifyRequest, reply: FastifyReply): Promise<unknown> {
  // 1. The credential. A refused one is told nothing about why.
  if (!authenticated(request)) return reply.code(401).send();

  const body = (request.body ?? {}) as Record<string, unknown>;

  // 2. The wire, before anything about the deployment (§3.1, §10).
  const protocol = body["protocol"];
  if (protocol !== PROTOCOL_VERSION) {
    return reply.code(409).send({
      protocol: PROTOCOL_VERSION,
      worker_protocol: protocol ?? null,
      error: `this hub speaks protocol ${PROTOCOL_VERSION} and the worker speaks ${describeVersion(protocol)}: ${behind(protocol)}`,
    });
  }

  // 3. The handshake triple's two required members (§4.1). The hash is the
  //    member the hub can *repair*, so it is not checked here at all.
  const compiler = body["compiler"];
  if (compiler !== COMPILER_VERSION) {
    return reply.code(409).send({
      compiler: COMPILER_VERSION,
      worker_compiler: compiler ?? null,
      error: `refused: this hub was built by agent-compose ${COMPILER_VERSION} and the worker runs ${describeVersion(compiler)} — upgrade the worker, or point it at a hub of its own release`,
    });
  }
  const runtime = typeof body["runtime"] === "string" ? (body["runtime"] as string) : undefined;
  if (runtime === undefined || !runs(runtime)) {
    return reply.code(409).send({
      runtime: `${WORKER_RUNTIME.name} ${WORKER_RUNTIME.major}.x`,
      worker_runtime: runtime ?? null,
      error: `refused: a worker executes the generated artifact under ${WORKER_RUNTIME.name.replace(/^./, (first) => first.toUpperCase())} ${WORKER_RUNTIME.major}.x and this one runs ${describeVersion(body["runtime"])} — install ${WORKER_RUNTIME.name}, or run this placement on a machine that has it`,
    });
  }

  // 4. The claims, which describe the target and are told only to a worker that
  //    got this far.
  const known = placements.map((placement) => placement.name);
  const offered = body["claims"];
  // **REQUIRED of every join, cold start included** (§3.1's field list), and
  // this is the whole of what "required" buys. Reading an absent, mis-typed or
  // empty value as "claims nothing" would issue a *dispatchable* session that no
  // work can ever reach: [`taken`] matches a row against `session.claims`, so
  // every hold that session takes is answered `204`, the placement's work parks
  // behind a worker both ends believe is healthy, and no request anywhere
  // carries a diagnostic. §3.1's posture is the opposite of that — every join
  // that cannot work is refused with a body naming why — so a join that did not
  // say what it claims is refused where one naming an unknown placement is, at
  // that row's status and with that row's body. The `agent-compose worker` this
  // repository ships refuses the same join before it sends it ("a worker claims
  // at least one placement"), so no conforming client meets this.
  if (
    !Array.isArray(offered) ||
    offered.length === 0 ||
    (offered as readonly unknown[]).some((claim) => typeof claim !== "string")
  ) {
    return reply.code(400).send({
      claims: offered ?? null,
      placements: known,
      error: `a join names the placements this worker claims: \`claims\` is required of every join and is a non-empty list of placement names, and this target declares ${known.map((name) => `\`${name}\``).join(", ")}`,
    });
  }
  const claims = offered as readonly string[];
  for (const claim of claims) {
    if (known.includes(claim)) continue;
    return reply.code(400).send({
      claim,
      placements: known,
      error: `\`${claim}\` names no placement of this target: it declares ${known.map((name) => `\`${name}\``).join(", ")}`,
    });
  }

  // 5. `env_ok` — checked against the manifest in the artifact the worker holds,
  //    which is the only artifact whose manifest it could have read (§9.2).
  const held = typeof body["artifact_hash"] === "string" ? body["artifact_hash"] : undefined;
  const current = held === ARTIFACT_HASH;
  const reported = Array.isArray(body["env_ok"])
    ? (body["env_ok"] as unknown[]).map((name) => String(name))
    : undefined;
  if (reported !== undefined && current) {
    const missing = unsatisfied(claims, reported);
    if (missing.length > 0) {
      return reply.code(403).send({
        // **Names, never values, and never whether the hub holds them** (§9).
        variables: missing,
        error: `this worker reports none of ${missing.map((name) => `\`${name}\``).join(", ")}, which the placements it claims need`,
      });
    }
  }
  // **One direction only.** A join carrying the current hash and no report is
  // refused: there is a manifest it could have read and it did not, and
  // dispatching to it would skip the check §9.2 exists for. The other direction
  // is **not** a refusal, and that is §3.1's stale-hash row applied to the whole
  // request: a report made against an artifact this hub is not serving is a
  // report about the wrong manifest, so it is ignored the way the stale hash
  // beside it is — the join succeeds, the answer carries the current artifact,
  // and the worker fetches and joins again.
  //
  // Refusing that pair instead would make the two rows unsatisfiable together
  // for the one worker that meets both: a redeployment leaves a worker holding a
  // stale hash **and** a report, and §3.1 requires the report to be present on a
  // current-hash join — so it cannot pre-decide which case it is in, and a
  // refusal would leave it answering a refused join with another join, which §5
  // and §10.1 both forbid.
  if (current && reported === undefined) {
    return reply.code(400).send({
      artifact: ARTIFACT_HASH,
      error:
        "a join whose `artifact_hash` is the artifact this hub serves must carry `env_ok`: the report is against the manifest in the artifact the worker holds (docs/distributed.md §3.1)",
    });
  }

  const session: Session = {
    id: `wrk_${globalThis.crypto.randomUUID()}`,
    claims,
    // A provisioning join is answered normally and dispatched nothing (§3.1).
    dispatchable: current,
    seen: Date.now(),
  };
  sessions.set(session.id, session);
  // §6.2's wake: a join that may be dispatched to is what a parked placement
  // wait has been waiting for, and the held polls re-scan the board.
  if (session.dispatchable) stirPolls();
  return reply.code(200).send({
    protocol: PROTOCOL_VERSION,
    compiler: COMPILER_VERSION,
    worker_session: session.id,
    artifact: { hash: ARTIFACT_HASH, url: `/workers/artifact/${ARTIFACT_HASH}` },
    poll_url: "/workers/poll",
  });
}

/** Which of the claimed placements' variables this report does not name (§9.2). */
function unsatisfied(claims: readonly string[], reported: readonly string[]): readonly string[] {
  const missing: string[] = [];
  for (const claim of claims) {
    const manifest = placements.find((placement) => placement.name === claim);
    if (manifest === undefined) continue;
    for (const variable of manifest.environment) {
      if (!reported.includes(variable) && !missing.includes(variable)) missing.push(variable);
    }
  }
  return missing;
}

/** Whether a worker's `runtime` is one this artifact executes under (§4.1). */
function runs(reported: string): boolean {
  const [name, version] = reported.trim().split(/\s+/, 2);
  if ((name ?? "").toLowerCase() !== WORKER_RUNTIME.name) return false;
  const major = Number((version ?? "").split(".")[0]);
  return Number.isInteger(major) && major === WORKER_RUNTIME.major;
}

/** A version a refusal has to name, however the request spelled it. */
function describeVersion(value: unknown): string {
  return value === undefined || value === null ? "nothing" : String(value);
}

/**
 * Which end of a protocol mismatch is behind — and the third answer, for the
 * request where neither is.
 *
 * §3.1 asks this refusal to name "both versions, and which end is behind", and
 * two branches cannot say the true thing about a **malformed** field: a join
 * that omits `protocol`, or sends `"1"` as a string, is not evidence that this
 * hub is old, and telling an operator to upgrade the hub over a request field
 * their client spelled wrong sends them to the wrong machine. The version is a
 * number on the wire (§3.1's body), so anything else is a client that did not
 * send one this hub can compare, and the refusal says so.
 */
function behind(protocol: unknown): string {
  if (!Number.isInteger(protocol)) {
    return "the worker did not send a version this hub can compare — `protocol` is REQUIRED of every join, as the whole number of the wire contract it speaks";
  }
  return (protocol as number) < PROTOCOL_VERSION
    ? "the worker is behind — upgrade it"
    : "this hub is behind — upgrade it";
}

/** `GET /workers/poll` (§3.2). */
async function poll(
  request: FastifyRequest,
  reply: FastifyReply,
  holdMs: number,
): Promise<unknown> {
  if (!authenticated(request)) return reply.code(401).send();
  const session = touched(request);
  if (session === undefined) return reply.code(410).send(gone());

  // **Whether the worker is still on the other end of this poll**, latched once
  // for the whole handler.
  //
  // A hold ends for two reasons and only one of them means "ask again": the hold
  // expired, or the connection closed. A worker that was SIGKILLed — or a poll an
  // intermediary dropped — is the second, and a hub that went round its loop
  // anyway would do two wrong things with one dead socket. It would refresh
  // `session.seen` for a request that has demonstrably ended, extending §6.3's
  // "90 seconds since the last request on a session" by up to a whole hold; and
  // it would `claimDispatch` parked work for a socket nothing can be written to,
  // marking the row `dispatched` against a session that never received it. What
  // that second one costs is worth stating exactly, because it is *not* one
  // liveness window: a worker whose poll an intermediary dropped is still there
  // and still polling, so §6.3 never fires and nothing supersedes the row until
  // that node's `timeout:` chain runs out from dispatch (§6.5) — the whole
  // budget, spent on a message that was never delivered. So the latch is read
  // twice, once before the board is looked at and once after the claim, and
  // [`released`] is what the second read does about it.
  //
  // **Both events, because the two runtimes emit different ones.** A hangup
  // during a hold raises `aborted` on the *request* under Bun and `close` on the
  // *response* under Node, and this module is the compiler's constant under
  // both. The request's `close` is deliberately not among them, and that is the
  // asymmetry: a `GET` has no body, so under Node its request stream is complete
  // the moment the headers are parsed and `close` fires at once — a hold
  // watching it would end immediately on every poll an idle mesh makes.
  // `aborted` fires only on a premature end, which is the property wanted.
  //
  // The latch resolves whatever hold is in flight as well as being read at the
  // top of the loop, because the failure this exists for is work parking
  // *inside* the hold a dead poll was in: a wake that only came at the hold's
  // own deadline would leave the whole rest of that hold claimable.
  let hungUp = false;
  const holds = new Set<() => void>();
  const hangUp = (): void => {
    hungUp = true;
    for (const wake of [...holds]) wake();
  };
  // `socket.destroyed` beside the latch, for a runtime that emits neither
  // event: it is never true of a connection a hold is still open on, and it is
  // what both runtimes agree about after a hangup.
  //
  // A function rather than the expression written twice, because it is **asked
  // twice** and the second answer is not the first: everything between them is
  // an `await`, and both halves of this can become true inside one. Written
  // inline, a compiler that has narrowed the first read would fold the second
  // into `false`, which is the one thing it may not be.
  const away = (): boolean => hungUp || reply.raw.socket?.destroyed === true;
  request.raw.once("aborted", hangUp);
  reply.raw.once("close", hangUp);
  try {
    const deadline = Date.now() + holdMs;
    for (;;) {
      if (away()) return reply.code(204).send();
      // The session's own request keeps it alive for the whole hold, not only
      // for the instant it arrived: a hold that outlived the window would let
      // the sweep declare the very worker whose poll it is holding gone.
      session.seen = Date.now();
      // Read before the board is looked at, compared after this poll is
      // subscribed: see [`boardMoved`].
      const seen = boardMoved;
      const dispatch = await taken(session);
      if (dispatch !== undefined) {
        // **Checked again, because the claim is not free of time.** The latch at
        // the top of the loop was read before the board was; opening the journal
        // and claiming a row is an `await`, and a worker that hung up inside it
        // is one this hub has just marked a row `dispatched` for over a socket
        // nothing can be written to. Nothing was sent, so §7's "a hub that
        // cannot tell whether a dispatch arrived re-issues it" is not even a
        // judgement call here: it demonstrably did not arrive, and the row goes
        // back on the board in its own place rather than sitting `dispatched`
        // until that node's `timeout:` fires (§6.5) — which is what it would do,
        // because the worker on the other end is *alive* and its next poll
        // refreshes the very liveness window that would otherwise supersede it.
        if (away()) {
          await released(String(dispatch["dispatch_id"]), session.id);
          return reply.code(204).send();
        }
        return reply.code(200).send(dispatch);
      }
      const left = deadline - Date.now();
      if (left <= 0) return reply.code(204).send();
      await held(Math.min(left, holdMs), seen, holds);
    }
  } finally {
    // Off with the handler: `close` fires on an ordinary answer too, once the
    // response has been written, and a listener left behind would be one per
    // poll on a connection the runtime warns about at ten.
    request.raw.removeListener("aborted", hangUp);
    reply.raw.removeListener("close", hangUp);
  }
}

/**
 * The dispatch this session may be handed, if the board has one.
 *
 * Three rules of §2 and §6.2 in one function, and the order is each of them:
 * a session that has not settled its dispatch is answered nothing however deep
 * the queue; a provisioning session is answered nothing at all; and what is left
 * is scanned **in park order**, over the placements this session claims.
 */
async function taken(session: Session): Promise<Record<string, unknown> | undefined> {
  if (!session.dispatchable) return undefined;
  if (holding(session.id)) return undefined;
  const journal = await openJournal();
  for (const row of journal.unsettledDispatches()) {
    if (row.status !== "parked") continue;
    if (!session.claims.includes(row.placement)) continue;
    // A row this process is not awaiting is one whose node is not running here
    // — a predecessor's, on an execution nothing has replayed yet. Left alone:
    // handing it out would dispatch work no node is waiting for the answer to.
    if (!awaiting.has(row.id)) continue;
    const claimed = journal.claimDispatch(row.id, session.id);
    if (claimed === undefined) continue;
    rows.set(claimed.id, claimed);
    const local = awaiting.get(claimed.id);
    if (local !== undefined) {
      local.taken = true;
      announceWaits(claimed.execution);
    }
    // §3.2's `session_key`, read off the execution's own lifecycle row rather
    // than off the dispatch: it is a fact about the execution, not about this
    // node, and §5's first rule is that everything is read out of the journal at
    // the moment it is needed. A worker that had to default it would fail a
    // `scope: session` store with a diagnostic telling the operator to pass a
    // `--session` the run already passed.
    const sessionKey = journal.execution(claimed.execution)?.sessionKey ?? "";
    return {
      dispatch_id: claimed.id,
      execution_id: claimed.execution,
      node: claimed.node,
      instance_path: claimed.site,
      inputs: claimed.inputs,
      ...(sessionKey === "" ? {} : { session_key: sessionKey }),
      ...(claimed.itemIndex === undefined ? {} : { item_index: claimed.itemIndex }),
      ...(claimed.history === undefined ? {} : { history: claimed.history }),
      ...(claimed.policy === undefined ? {} : { policy: claimed.policy }),
      // What a redispatched node replays to the frontier before going live
      // (§3.2, §7.2). Read out of the journal at the moment it is handed over,
      // which is §5's first rule: nothing is held from an earlier session.
      effect_history: journal.effectsUnder(claimed.execution, claimed.site).map(wireEffect),
    };
  }
  return undefined;
}

/**
 * Undo one claim [`poll`] made for a worker that had already gone.
 *
 * The inverse of the claim in [`taken`], and everything that claim did is undone
 * in the same order: the journal's row goes back to `parked` under its original
 * `parked_at` — so it keeps its place in the park order §6.2 drains in, ahead of
 * whatever queued behind it — this process's cache follows the journal, the wait
 * this hub is holding is a pause again rather than a node running elsewhere, and
 * the held polls are stirred so the placement's next worker is answered with it
 * now instead of a hold later.
 *
 * A row the journal will not hand back is left alone and announced to nobody:
 * `releaseDispatch` takes only a row still `dispatched` to *this* session, so a
 * result that arrived in the meantime, or a deadline that superseded it, wins.
 */
async function released(id: string, session: string): Promise<void> {
  const journal = await openJournal();
  if (!journal.releaseDispatch(id, session)) return;
  const parked = journal.dispatchOf(id);
  if (parked !== undefined) rows.set(id, parked);
  const local = awaiting.get(id);
  if (local !== undefined) {
    local.taken = false;
    announceWaits(local.execution);
  }
  stirPolls();
}

/**
 * Whether this session is holding a dispatch it has not settled (§2).
 *
 * Read against the rows this process has handed out rather than against a
 * counter, so a restart re-derives it from what it re-attaches to rather than
 * from something it remembered. What it is **not** written against is a worker
 * that keeps two polls in flight: §2 requires "exactly one poll in flight from
 * the moment it joins", and a session that breaks that could interleave two
 * checks around one claim. That is a worker in breach of the wire rather than a
 * race this hub arbitrates, and the cost is bounded — the second dispatch is a
 * real dispatch, journaled, and settled or superseded like any other.
 */
function holding(session: string): boolean {
  for (const row of rows.values()) {
    if (row.session === session && row.status === "dispatched" && awaiting.has(row.id)) return true;
  }
  return false;
}

/** One journaled effect, as the wire carries it. */
function wireEffect(record: JournalRecord): Record<string, unknown> {
  return {
    key: record.key,
    site: record.site,
    kind: record.kind,
    ordinal: record.ordinal,
    request: record.request,
    outcome: record.outcome,
    refused: record.refused,
    recorded_at: record.recordedAt,
  };
}

/**
 * Wait out the rest of a hold, until the board moves, or until the worker hangs
 * up.
 *
 * `seen` is the board's generation as of **before** the caller looked at it, so
 * a dispatch parked between that look and this subscription resolves the hold at
 * once rather than a whole hold later (see [`boardMoved`]).
 *
 * `hangUps` is [`poll`]'s own set of "wake whatever hold is in flight", and this
 * puts itself in it for the duration: a worker that hung up is a hold with
 * nobody to answer, so the process is not left holding one timer per abandoned
 * connection — and the loop above gets to notice inside the hold rather than at
 * its deadline. Which *events* mean a hangup is [`poll`]'s to decide, because
 * the two runtimes emit different ones and one of them is a trap.
 */
function held(milliseconds: number, seen: number, hangUps: Set<() => void>): Promise<void> {
  if (boardMoved !== seen) return Promise.resolve();
  return new Promise<void>((resolve) => {
    let done = false;
    const finish = (): void => {
      if (done) return;
      done = true;
      clearTimeout(timer as Parameters<typeof clearTimeout>[0]);
      pollers.delete(finish);
      hangUps.delete(finish);
      resolve();
    };
    const timer: unknown = setTimeout(finish, milliseconds);
    if (typeof (timer as { unref?: () => void }).unref === "function") {
      (timer as { unref: () => void }).unref();
    }
    pollers.add(finish);
    hangUps.add(finish);
  });
}

/**
 * `POST /workers/effects` (§3.3).
 *
 * §3.3's table is five rows and this is each of them: `204` when the batch is
 * journaled, `401` for a credential that did not verify, `410` for a session
 * this hub does not know, `400` for a body that is not a batch — a missing
 * `dispatch_id`, a missing `effects` array, or a record short of a field or
 * naming a `site` outside the dispatch's — and `409` for a `dispatch_id` this
 * hub cannot attribute. The last two are refusals rather than a `204` because a
 * `204` over a batch nothing was written for would tell a worker its effects are
 * in the journal when they are not, and the redispatch of §7.2 would then hand
 * the next attempt a history short of the frontier — the one failure that whole
 * route exists to prevent.
 *
 * **A batch is taken whatever the dispatch's state is**, superseded included.
 * §3.3 keys records by effect key and scopes them to their execution, "not by
 * session, and not by dispatch", precisely so the journal takes them from
 * whichever session hands them over — and a superseded attempt's effects are
 * exactly the ones its retry must replay rather than re-issue.
 *
 * **And it is read whole before any of it is written.** §3.3 says nothing about
 * atomicity, so this is a choice rather than a rule — but it is the only one
 * that makes the `400` mean what a worker reads it as. A refusal on this route
 * is the one answer no re-send improves, so a worker takes it as the end of that
 * dispatch; a `400` sent after half the batch was already appended would fail
 * that attempt with a partly applied batch behind it, and the next reader of the
 * journal could not tell which half. Building every record first costs one pass
 * over a batch that is usually one record, and buys a refusal that changed
 * nothing.
 */
async function effects(request: FastifyRequest, reply: FastifyReply): Promise<unknown> {
  if (!authenticated(request)) return reply.code(401).send();
  const session = touched(request);
  if (session === undefined) return reply.code(410).send(gone());

  const body = (request.body ?? {}) as Record<string, unknown>;
  const id = typeof body["dispatch_id"] === "string" ? body["dispatch_id"] : undefined;
  const batch = Array.isArray(body["effects"]) ? (body["effects"] as unknown[]) : undefined;
  if (id === undefined || batch === undefined) {
    return reply
      .code(400)
      .send({ error: "a batch names its `dispatch_id` and carries an `effects` array" });
  }
  const journal = await openJournal();
  const row = journal.dispatchOf(id);
  if (row === undefined) {
    return reply.code(409).send({ dispatch_id: id, error: `no dispatch \`${id}\`` });
  }
  const records: JournalRecord[] = [];
  for (const entry of batch) {
    const record = recordOf(row, entry);
    if (record === undefined) {
      return reply.code(400).send({
        dispatch_id: id,
        error:
          "every record carries a `site` at or inside the dispatch's `instance_path`, a `kind`, an `ordinal`, a canonical `request`, an `outcome`, and the `key` those three derive — `<site>#<kind>/<ordinal>`",
      });
    }
    records.push(record);
  }
  for (const record of records) {
    // **Workers SEND, the hub INSERTS** (§3.3), idempotently by effect key: a
    // record the journal already holds is accepted and dropped, which is what
    // makes a batch safe to re-send after a transport failure.
    journal.append(record);
    // …and the one field an insert cannot carry, because it is written *after*
    // the row: `refused` says the generation that produced this answer had its
    // own contract refuse it (`docs/durability.md` §5, `./journal.ts`'s
    // `refuseRecorded`). A worker sends the record a second time with the mark
    // on it, the insert above drops the duplicate, and this is what makes the
    // mark stick — without which the retry of §7.3 would read an unmarked record
    // and call the node's own mismatch a replay divergence.
    if (record.refused) journal.refuse(record.execution, record.key);
  }
  return reply.code(204).send();
}

/** One record off the wire, or `undefined` where it is not one. */
function recordOf(row: DispatchRow, entry: unknown): JournalRecord | undefined {
  if (entry === null || typeof entry !== "object") return undefined;
  const held = entry as Record<string, unknown>;
  const key = held["key"];
  const site = held["site"];
  const kind = held["kind"];
  const ordinal = held["ordinal"];
  const request = held["request"];
  const outcome = held["outcome"];
  if (typeof key !== "string" || typeof site !== "string" || typeof request !== "string") {
    return undefined;
  }
  if (typeof kind !== "string" || !["model", "tool", "store", "human"].includes(kind)) {
    return undefined;
  }
  if (typeof ordinal !== "number" || !Number.isInteger(ordinal)) return undefined;
  // **The execution is the hub's, never the batch's.** A worker names a dispatch
  // and the hub reads the execution off the row, so no session can write an
  // effect into an execution it was never dispatched — and the site is held to
  // the dispatch's own instance path for the same reason (§8's single writer).
  // The **key** with it, since the key is what the row is written under: a site
  // inside this dispatch carrying a key outside it would put this record in
  // another node's slot, where that node's replay claims it and diverges. See
  // [`pauseOf`], which holds the other ledger to the same derivation.
  if (!under(site, row.site)) return undefined;
  if (key !== effectKey(site, kind as EffectKind, ordinal)) return undefined;
  const settled = outcomeOf(outcome);
  if (settled === undefined) return undefined;
  const at = held["recorded_at"];
  return {
    execution: row.execution,
    key,
    site,
    kind: kind as EffectKind,
    ordinal,
    request,
    outcome: settled,
    refused: held["refused"] === true,
    recordedAt: typeof at === "string" ? at : new Date().toISOString(),
  };
}

/**
 * Whether a path is **at, or inside,** one instance path (grammar §9.4).
 *
 * The prefix relation the paths already carry, which is what §3.3 and §3.4 hold
 * a record's `site` and a pause's identity to: `sign/0` holds `sign/0/tool.sign/0`
 * and holds nothing of `stamp/0`'s.
 */
function under(path: string, root: string): boolean {
  return path === root || path.startsWith(`${root}/`);
}

/** One outcome off the wire, or `undefined` where it is not one. */
function outcomeOf(value: unknown): JournalOutcome | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const held = value as Record<string, unknown>;
  if (held["kind"] === "value") return { kind: "value", value: held["value"] };
  if (held["kind"] === "error") {
    return {
      kind: "error",
      name: typeof held["name"] === "string" ? held["name"] : "Error",
      message: typeof held["message"] === "string" ? held["message"] : "",
    };
  }
  return undefined;
}

/** `POST /workers/result` (§3.4). */
async function result(request: FastifyRequest, reply: FastifyReply): Promise<unknown> {
  if (!authenticated(request)) return reply.code(401).send();
  const session = touched(request);
  if (session === undefined) return reply.code(410).send(gone());

  const body = (request.body ?? {}) as Record<string, unknown>;
  const id = typeof body["dispatch_id"] === "string" ? body["dispatch_id"] : undefined;
  if (id === undefined) {
    // **`409`, and not the `400` §3.3 gives a malformed batch.** §3.4's table is
    // four rows and none of them is a `400`, and §10.1 lets an implementation
    // rely on "the status this document gives each refusal" — so a body naming
    // no dispatch is answered under the row it belongs to: a result this hub
    // cannot attribute is discarded. The difference is not cosmetic on the other
    // end of the wire, where a `4xx` outside the table is a refusal a worker
    // stops for (`src/worker/node.rs`), and a healthy worker is not worth losing
    // over one unattributable result.
    return reply.code(409).send({
      dispatch_id: null,
      error: "a result names the `dispatch_id` it settles, and this one names none",
    });
  }
  const journal = await openJournal();
  const row = journal.dispatchOf(id);
  // **`409`, not `410`.** §3.4: the hub knows this worker and does not want this
  // result — the execution has moved past it, and re-driving it from a stale
  // result is the divergence `docs/durability.md` §7 refuses.
  //
  // A **parked** row takes the same branch, and by §3.4's own definition rather
  // than by observation: what a result settles is a dispatch that is unsettled,
  // and unsettled is "still in flight, and the one piece of work a session may
  // be holding". A row nobody was handed is neither. Nothing on this wire can
  // reach it — a `dispatch_id` leaves this hub through [`taken`], which claims
  // the row to `dispatched` before it is written into the poll answer, and
  // `claimDispatch` has no path back — so this is a guard over the definition
  // and not over a case: settling a row no attempt exists for would journal an
  // outcome against work that never started.
  if (row === undefined || row.status === "superseded" || row.status === "parked") {
    return reply.code(409).send({
      dispatch_id: id,
      error:
        row === undefined
          ? `no dispatch \`${id}\``
          : row.status === "parked"
            ? `\`${id}\` is parked on this hub's board and has been handed to no session, so there is no attempt of it for a result to settle`
            : `\`${id}\` was superseded by this hub and the execution has moved past it: ${row.detail ?? "the session holding it stopped making requests"}`,
    });
  }
  // A dispatch a result already settled: accepted and dropped, which is what
  // makes at-least-once dispatch safe on the return path too.
  if (row.status === "settled") return reply.code(204).send();

  const outcome: JournalOutcome = settlementOf(row, body);
  journal.settleDispatch(id, outcome);
  rows.delete(id);
  const held = awaiting.get(id);
  if (held !== undefined) held.settle(outcome);
  // This session has settled what it was holding, so it may be handed the next
  // item in the queue — and the poll that will hand it over is one this process
  // is holding right now. Without the stir it waits out its hold first, which
  // turns a queue of eight into eight holds of idling (see [`stirPolls`]).
  //
  // A **paused** result settles the dispatch exactly as an answer does, which is
  // §3.4's own reading of the third ending: the worker is free the moment the
  // pause comes home, and may be dispatched other work while a person thinks.
  stirPolls();
  return reply.code(204).send();
}

/**
 * What one result body settles its dispatch with: an output, a failure, or a
 * pause (§3.4).
 *
 * The third is PRD resolved q46's addition and is read here rather than in the
 * route for one reason worth stating: a `paused` this hub cannot read is
 * answered as a **failure of the dispatch**, not as a status. §3.4's table gives
 * this route four statuses and §10.1 lets a peer rely on them, so a fifth over a
 * malformed body would be a refusal a worker stops for — while the honest cost
 * of an unreadable pause is the node's attempt, under its own `retry:`/
 * `on_error:` chain, exactly as `src/worker/node.rs` costs a dispatch for a body
 * no hub would take. Nothing conforming reaches it: the handshake refuses a peer
 * of another release (§4.1), and this release's own runner writes the shape
 * [`pauseOf`] reads.
 */
function settlementOf(row: DispatchRow, body: Record<string, unknown>): JournalOutcome {
  const paused = body["paused"];
  if (paused !== undefined && paused !== null) {
    const pause = pauseOf(row, paused);
    return pause === undefined
      ? {
          kind: "error",
          name: "PausedResultUnreadable",
          message: `this worker settled \`${row.id}\` paused with a pause this hub cannot read: a paused result names its \`wait\`, a \`flow\` and \`node\` this artifact declares as a \`human:\` node, what the person is \`shown\`, \`paused_at\`, and the \`effect\` record (\`key\`, \`site\`, \`ordinal\`, \`request\`) its answer is journaled under, whose \`key\` is \`<site>#human/<ordinal>\` — and the wait and the record both lie at or inside \`${row.site}\`, which is this dispatch's own instance path (docs/distributed.md §3.4, §8)`,
        }
      : { kind: "value", value: { paused: pause } };
  }
  return body["error"] === undefined || body["error"] === null
    ? {
        kind: "value",
        value: {
          output: body["output"],
          ...(body["history"] === undefined ? {} : { history: body["history"] }),
          ...(body["models"] === undefined ? {} : { models: body["models"] }),
          ...(body["tool_dispatches"] === undefined
            ? {}
            : { toolDispatches: body["tool_dispatches"] }),
          // The fourth collector, which travels because it cannot be read off
          // the answer on this side — see [`PlacedAnswer.stores`]. Journaled
          // with the rest of the outcome, so a replayed row reports what the
          // node's stores did as well as what its models did.
          ...(body["stores"] === undefined ? {} : { stores: body["stores"] }),
        },
      }
    : {
        kind: "error",
        name: String((body["error"] as Record<string, unknown>)["name"] ?? "Error"),
        message: String((body["error"] as Record<string, unknown>)["message"] ?? ""),
      };
}

/** `GET /workers/artifact/{hash}` (§3.5). */
async function artifact(request: FastifyRequest, reply: FastifyReply): Promise<unknown> {
  // **The bearer token alone.** This is the one route that does not require a
  // session, and the exception is deliberate: a worker whose session has aged
  // out re-joins and fetches, and a fetch should not be coupled to a lifetime.
  if (!authenticated(request)) return reply.code(401).send();
  const asked = (request.params as { hash?: string }).hash ?? "";
  if (!/^sha256:[0-9a-f]{64}$/.test(asked)) {
    return reply.code(400).send({
      error: `\`${asked}\` is not an artifact hash: one is \`sha256:\` and 64 lowercase hexadecimal digits`,
    });
  }
  if (asked !== ARTIFACT_HASH) {
    // v1 hubs serve exactly one artifact — their own tree — so an unknown hash
    // names the one this hub has, which is what a worker re-joins over (§3.5).
    return reply.code(404).send({
      hash: asked,
      artifact: ARTIFACT_HASH,
      error: `this hub does not hold \`${asked}\`; it serves \`${ARTIFACT_HASH}\``,
    });
  }
  const body = tarball();
  return reply
    .code(200)
    .header("content-type", "application/gzip")
    .header("content-length", String(body.length))
    .send(body);
}

// ---------------------------------------------------------------------------
// The credential, the session header, and the sweep
// ---------------------------------------------------------------------------

/** The join token this deployment verifies against. */
function credential(): string {
  return joinTokenEnv === undefined ? "" : (process.env[joinTokenEnv] ?? "");
}

/** Whether this request carried the deploy target's join token (§3). */
function authenticated(request: FastifyRequest): boolean {
  const expected = credential();
  if (expected === "") return false;
  const offered = headerOnce(request, "authorization");
  if (offered === undefined || !offered.startsWith("Bearer ")) return false;
  return equal(offered.slice("Bearer ".length), expected);
}

/**
 * The one value this request carried under `name`, or `undefined` where it
 * carried none or more than one.
 *
 * Read off the arrival list rather than the parsed map, and refusing a repeat,
 * for the reason `./serve.ts` gives at its own `headerValues`: the two runtimes a
 * project runs under disagree about which of two `authorization` headers wins,
 * so a request whose credential depends on who resolved that disagreement has
 * not presented one.
 */
function headerOnce(request: FastifyRequest, name: string): string | undefined {
  const arrived = request.raw.rawHeaders;
  const wanted = name.toLowerCase();
  let found: string | undefined;
  for (let index = 0; index + 1 < arrived.length; index += 2) {
    if ((arrived[index] ?? "").toLowerCase() !== wanted) continue;
    if (found !== undefined) return undefined;
    found = arrived[index + 1] ?? "";
  }
  return found;
}

/**
 * Whether two credentials are equal, compared in **constant time**.
 *
 * The same discipline grammar §13.3 requires of an inbound trigger credential
 * and for the same reason, restated here rather than reached for across a module
 * boundary: `./serve.ts` imports this module, so a helper borrowed from it would
 * close an import cycle for eight lines of comparison. A byte-by-byte early
 * return leaks the join token to a caller who can time it, and holding the whole
 * mesh is what that token is (§9.3).
 */
function equal(offered: string, expected: string): boolean {
  if (expected.length === 0) return false;
  const left = new TextEncoder().encode(offered);
  const right = new TextEncoder().encode(expected);
  if (left.length !== right.length) return false;
  return timingSafeEqual(left, right);
}

/** The session this request carries, marked as having been heard from. */
function touched(request: FastifyRequest): Session | undefined {
  const named = headerOnce(request, "x-worker-session");
  const session = named === undefined ? undefined : sessions.get(named);
  if (session !== undefined) session.seen = Date.now();
  return session;
}

/** The body every `410` on a session-carrying route answers with (§3). */
function gone(): Record<string, unknown> {
  return {
    error:
      "this hub does not know that worker session: join again, and make this request again under the session that join returns (docs/distributed.md §3, §5)",
  };
}

/** Whether the liveness sweep is already scheduled in this process. */
let sweeper: unknown;

/**
 * Run the liveness sweep of §6.3 on a repeating, unreferenced timer.
 *
 * A timer rather than a check on the next request, and that is the whole of why
 * it exists: the failure this detects is a worker that **stops making
 * requests**, so an execution whose only worker died has nothing left to arrive
 * and trigger a check. Unreferenced, so a process with nothing else to do still
 * exits.
 */
function sweeping(windowMs: number): void {
  if (sweeper !== undefined) return;
  const tick = (): void => {
    void sweep(windowMs).catch((error: unknown) => {
      process.stderr.write(`the mesh liveness sweep could not run: ${message(error)}\n`);
    });
  };
  const every = Math.max(250, Math.floor(windowMs / 4));
  const timer: unknown = setInterval(tick, every);
  if (typeof (timer as { unref?: () => void }).unref === "function") {
    (timer as { unref: () => void }).unref();
  }
  sweeper = timer;
}

/**
 * Forget every session that has gone quiet, and supersede what each was holding
 * (§6.3).
 *
 * The two cases are §6.3's two, and the difference is the whole of what a closed
 * laptop costs. A session with **no** unsettled dispatch costs nothing: the
 * placement's open waits stay on the board and the next join takes them in park
 * order. A session **holding** one supersedes it, which fails the node's attempt
 * under its own `retry:`/`on_error:` chain — and where `retry:` grants another,
 * the node re-enters dispatch and parks again if nothing is claiming the
 * placement, which is the sense in which heartbeat loss re-parks.
 */
async function sweep(windowMs: number): Promise<void> {
  const stale: string[] = [];
  const now = Date.now();
  for (const session of sessions.values()) {
    if (now - session.seen > windowMs) stale.push(session.id);
  }
  if (stale.length === 0) return;
  for (const id of stale) sessions.delete(id);
  const journal = await openJournal();
  for (const [id, row] of [...rows]) {
    if (row.session === undefined || !stale.includes(row.session)) continue;
    if (row.status !== "dispatched") continue;
    const detail = `the session holding this dispatch made no request for ${windowMs}ms, so the hub gave up on it (docs/distributed.md §6.3)`;
    journal.supersedeDispatch(id, detail);
    rows.delete(id);
    const held = awaiting.get(id);
    if (held === undefined) continue;
    awaiting.delete(id);
    held.settle({ kind: "error", name: "DispatchSuperseded", message: detail });
  }
  stirPolls();
}

// ---------------------------------------------------------------------------
// The tarball (§3.5, §4)
// ---------------------------------------------------------------------------

/** The emitted project's root: the directory `src/` sits in. */
const PROJECT_ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

/** The served body, built once. See [`buildTarball`]. */
let packed: Buffer | undefined;

/**
 * The artifact as §3.5 serves it: a gzipped tar of exactly the files this build
 * emitted.
 *
 * Built by hand rather than by a dependency, because §2's "no new runtime
 * dependency" is a rule about this whole surface and a tar writer is sixty lines
 * of a format that has not moved since 1988. Every header field that is not the
 * name, the size and the mode is written as a constant — no modification time,
 * no owner, no group — which is what makes two hubs built from one composition
 * serve byte-identical tarballs as well as one hash.
 */
function tarball(): Buffer {
  packed ??= buildTarball();
  return packed;
}

function buildTarball(): Buffer {
  const blocks: Buffer[] = [];
  for (const relative of ARTIFACT_FILES) {
    const bytes = fs.readFileSync(path.join(PROJECT_ROOT, relative));
    blocks.push(tarHeader(relative, bytes.length), pad(bytes));
  }
  // Two zero blocks end an archive, and the reader that would accept one is not
  // one this has to be written for.
  blocks.push(Buffer.alloc(1024));
  // The gzip header this writes carries no modification time — the runtime
  // leaves it zero — which is the other half of what makes two hubs built from
  // one composition serve identical bytes.
  return zlib.gzipSync(Buffer.concat(blocks), { level: 9 });
}

/** One ustar header block. */
function tarHeader(name: string, size: number): Buffer {
  const header = Buffer.alloc(512);
  const written = Buffer.from(name, "utf8");
  if (written.length > 100) {
    throw new Error(`\`${name}\` is longer than a tar header holds, so it cannot be served`);
  }
  written.copy(header, 0);
  header.write("0000644\0", 100, "ascii"); // mode
  header.write("0000000\0", 108, "ascii"); // uid
  header.write("0000000\0", 116, "ascii"); // gid
  header.write(`${size.toString(8).padStart(11, "0")}\0`, 124, "ascii");
  header.write("00000000000\0", 136, "ascii"); // mtime, fixed: see [`tarball`]
  header.write("        ", 148, "ascii"); // checksum, computed below
  header.write("0", 156, "ascii"); // a regular file
  header.write("ustar\0", 257, "ascii");
  header.write("00", 263, "ascii");
  let checksum = 0;
  for (const byte of header) checksum += byte;
  header.write(`${checksum.toString(8).padStart(6, "0")}\0 `, 148, "ascii");
  return header;
}

/** One file's bytes, padded to the 512-byte block a tar entry is. */
function pad(bytes: Buffer): Buffer {
  const remainder = bytes.length % 512;
  if (remainder === 0) return bytes;
  return Buffer.concat([bytes, Buffer.alloc(512 - remainder)]);
}

/**
 * The content hash of a materialised tree, computed the way
 * `compose_core::codegen::artifact` computes it.
 *
 * Exported because §4 step 2 makes a worker "verify the hash it computed against
 * the hash it asked for before unpacking anything", and both ends have to agree
 * about the answer to the byte. `src/artifact.ts` is the one entry outside the
 * digest, for the reason that module gives: a file carrying the hash of a tree it
 * is part of has no fixed point.
 */
export function contentHash(files: ReadonlyMap<string, Uint8Array>): string {
  const listing: string[] = [];
  for (const name of [...files.keys()].sort()) {
    if (name === "src/artifact.ts") continue;
    const bytes = files.get(name);
    if (bytes === undefined) continue;
    listing.push(`${name}\0${createHash("sha256").update(bytes).digest("hex")}`);
  }
  return `sha256:${createHash("sha256").update(listing.join("\n")).digest("hex")}`;
}

function message(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
