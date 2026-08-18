//
// What a compiled node does when it runs (PRD 5.5, grammar 8, 9).
//
// `./graph.ts` is the composition: one descriptor per node, the guards, the
// budgets, the wiring. This module is everything those descriptors share — the
// activity loop, the provider surfaces, the subprocess and HTTP wrappers, and
// the router. It is byte-identical in every project this compiler release
// builds, which is what makes a golden diff about the *composition* rather than
// about the runtime it happens to sit beside.
//
// # Temporal discipline (PRD 5.5)
//
// The compiled graph is the workflow — deterministic and replayable — and nodes
// are activities: effectful, retryable, and the only place a clock, a socket or
// a child process appears. Everything in this module that touches one is behind
// [`runActivity`], which owns the per-node `retry` / `timeout` / `on_error`
// contract of grammar 9 and reports failures as `NodeFailure`s naming the node.
//
// # Why the policies are not LangGraph's
//
// LangGraph 1.4 has `retryPolicy` and `timeout` options on `addNode`, and they
// are deliberately not used. Its timeout "applies to a single attempt … the
// timer resets for each retry attempt", while grammar 9.2 bounds **one node
// execution, all retry attempts included**, and grammar 9.1 says outright that
// "retry attempts consume the node's `timeout` budget". Those are different
// contracts, and the one this compiler is held to is the grammar's — so the
// budget is a deadline this module carries across attempts.
//
// # Where a fan-out and a subgraph run
//
// A `map` dispatches its instances **inside the map node's own task**
// ([`runMap`]), and a `flow:` node instantiates its subflow as a separate run of
// a separate compiled graph ([`runSubflow`]). Grammar 7.6's codegen note leaves
// the shaping to M1 and fixes P1 and P2 instead; the reasons this is the shaping
// that satisfies grammar 8.6 and 10.1 — and what a `Send` into the parent graph
// cannot express — are the ledger in `codegen::graph`, which is where a reader
// signs a difference off.
//
// # Why the model is called with `fetch`
//
// No provider SDK, and no `withStructuredOutput`. Three reasons, in the order
// they bind:
//
//  1. **The schema a model is constrained by has to be the schema its answer is
//     parsed with.** PRD 5.2 makes structured output load-bearing for routing,
//     and `codegen::schema`'s *What a provider is handed* records what the
//     LangChain path does to it: converting the emitted Zod to JSON Schema drops
//     every check Zod models as a refinement — six of the ten `format:`s, both
//     length bounds, `unique_items` — so the model would be told less than the
//     parse then demands of it. This module sends the JSON Schema the compiler
//     *publishes* (`codegen::schema`'s JSON column), which the conformance
//     corpus proves accepts exactly what the emitted Zod accepts.
//  2. **The wire is a contract this project tests.** `crates/mock-provider` is
//     strict about the request a compiled graph sends; going through an SDK
//     would put its defaults and its extra keys between codegen and that gate.
//  3. **One fewer pinned dependency** (PRD 5.12), and no SDK release able to
//     move the runtime's semantics without moving the compiler's.
//
// `fetch` is global from Node 18 and the project's floor is 22.18.

import { spawn } from "node:child_process";
import process from "node:process";

import { Command, GraphRecursionError, isInterrupted } from "@langchain/langgraph";

import { CelError, bind, evaluate, evaluateGuard, toJson } from "./cel.ts";
import type { CelValue, Roots, Shape } from "./cel.ts";

// ---------------------------------------------------------------------------
// Failures
// ---------------------------------------------------------------------------

/** A node error: what grammar 9's `on_error` decides the fate of. */
export class NodeFailure extends Error {
  readonly flow: string;
  readonly node: string;
  readonly attempts: number;

  constructor(flow: string, node: string, attempts: number, detail: string, cause?: unknown) {
    super(`${flow} node \`${node}\` failed: ${detail}`);
    this.name = "NodeFailure";
    this.flow = flow;
    this.node = node;
    this.attempts = attempts;
    this.cause = cause;
  }
}

/** A node whose `timeout:` budget ran out (grammar 9.2). */
export class NodeTimeout extends Error {
  constructor(node: string, budgetMs: number, attempts: number) {
    super(
      `\`${node}\` timed out: its ${budgetMs}ms budget was spent after ${attempts} attempt(s) (grammar 9.2)`,
    );
    this.name = "NodeTimeout";
  }
}

/** Grammar 7.3 rule 7: a completed node with no edge to take. */
export class NoViableRoute extends Error {
  /**
   * What each outgoing edge answered, in declaration order.
   *
   * The reason the error carries them: they are the routing decision PRD 5.3
   * asks a trace to hold, and the run that this error ends is exactly the run
   * whose trace never got to record them. [`runNode`] puts them on the entry it
   * hands the failure, so "which guard answered what" survives an abort.
   */
  readonly decisions: readonly EdgeDecision[];

  constructor(flow: string, node: string, decisions: readonly EdgeDecision[]) {
    super(
      `${flow}: no viable route out of \`${node}\` — every outgoing edge was untaken (grammar 7.3 rule 7)`,
    );
    this.name = "NoViableRoute";
    this.decisions = decisions;
  }
}

/** A construct this compiler release parses, validates, and does not yet run. */
export class Unimplemented extends Error {
  constructor(what: string, bullet: string) {
    super(`${what} is not executed by this compiler release: ${bullet}`);
    this.name = "Unimplemented";
  }
}

/**
 * One dispatched `map` instance that did not complete (grammar 8.6 rule 10).
 *
 * Raised out of the map node when `on_item_error` resolves the item as `fail` —
 * either because that is the policy or because a parameterized `retry:` ran out
 * of attempts, which grammar 8.6 rule 10 says resolves as `fail` does. It is a
 * failure of the **map node**, so the node's own `on_error:` is what absorbs it,
 * and the item it names is the source-item index rather than a completion
 * ordinal: two runs over one array fail about the same item.
 */
export class ItemFailure extends Error {
  /** The source-item index, which is what identifies the item (PRD 5.6). */
  readonly index: number;
  /** The dispatch target the item went to. */
  readonly target: string;
  /** How many attempts the item's own policy made. */
  readonly attempts: number;
  /**
   * What the **whole** fan-out did, in source-item order (PRD 5.3, 5.6).
   *
   * Carried on the failure for the reason [`carryEntry`] carries a trace entry:
   * a map node that throws returns no answer, so the records it made have
   * nowhere else to go — and this is the path where they matter most. The items
   * that completed already had their effects, including any detached delivery
   * this dispatch issued, and the failure alone says nothing about them.
   */
  readonly dispatches: readonly DispatchRecord[];

  constructor(
    node: string,
    index: number,
    target: string,
    attempts: number,
    cause: unknown,
    dispatches: readonly DispatchRecord[] = [],
  ) {
    super(
      `\`${node}\` item ${index} was dispatched to \`${target}\` and failed after ${attempts} attempt(s): ${describe(cause)}`,
    );
    this.name = "ItemFailure";
    this.index = index;
    this.target = target;
    this.attempts = attempts;
    this.cause = cause;
    this.dispatches = dispatches;
  }
}

/**
 * A subflow instance that did not reach quiescence, or reached it holding no
 * value for one of its `outputs:` (grammar 7.5, 7.6.3, 10.1).
 *
 * The subgraph boundary is where a failure would otherwise lose its context: the
 * caller sees a node that failed, and what actually happened is a node three
 * levels down. So the message names the instance and the error keeps the
 * instance's own trace, which the `flow:` node then hands to its trace entry.
 */
export class SubflowFailure extends Error {
  /** The flow that was instantiated. */
  readonly flow: string;
  /** Every routing decision the instance made before it stopped (PRD 5.3). */
  readonly trace: readonly TraceEntry[];

  constructor(flow: string, what: string, trace: readonly TraceEntry[], cause: unknown) {
    super(`the instance of \`${flow}\` ${what}: ${describe(cause)}`);
    this.name = "SubflowFailure";
    this.flow = flow;
    this.trace = trace;
    this.cause = cause;
  }
}

/**
 * A result that does not match the schema its own contract declares (PRD 5.2).
 *
 * Structured output is load-bearing for routing, so the parse is where an answer
 * is held to the contract — and what a reader needs from a failed one is *which*
 * field and *what was there*. The emitted Zod says the first and, for two shapes
 * that matter most, not the second: a discriminated union reports "Invalid
 * discriminator value. Expected 'auto_fixable' | 'needs_human'" without ever
 * naming the tag it was handed, and a closed object reports an unrecognized key
 * without its value. Reading the document at each issue's own path is what makes
 * the message actionable rather than merely correct (PRD G3).
 */
export class ResultMismatch extends Error {
  /** What was being parsed, in diagnostic voice. */
  readonly subject: string;

  constructor(subject: string, value: unknown, issues: readonly ResultIssue[]) {
    const described = issues
      .map((issue) => {
        const path = issue.path.map(String).join(".");
        const found = valueAt(value, issue.path);
        const at = path === "" ? "" : `${path}: `;
        return found === undefined ? `${at}${issue.message}` : `${at}${issue.message} (found ${excerpt(found)})`;
      })
      .join("; ");
    super(`${subject}: ${described}`);
    this.name = "ResultMismatch";
    this.subject = subject;
  }
}

/** One issue a schema reported, in the shape both Zod and this module read. */
interface ResultIssue {
  readonly path: readonly PropertyKey[];
  readonly message: string;
}

/** What a schema this module parses with has to be able to do. */
export interface ResultSchema<T> {
  safeParse(value: unknown): {
    readonly success: boolean;
    readonly data?: T;
    readonly error?: { readonly issues: readonly ResultIssue[] };
  };
}

/** The value a document holds at one issue's path, if it holds one. */
function valueAt(value: unknown, path: readonly PropertyKey[]): unknown {
  let held = value;
  for (const step of path) {
    if (held === null || typeof held !== "object") return undefined;
    held = (held as Record<PropertyKey, unknown>)[step];
  }
  return held;
}

/** A value as a message quotes it: JSON, and short enough to read. */
function excerpt(value: unknown): string {
  const rendered = JSON.stringify(value) ?? String(value);
  return rendered.length <= 120 ? rendered : `${rendered.slice(0, 117)}...`;
}

/**
 * Parse one result with the schema its contract declares, or say what was wrong
 * with it (PRD 5.2, and see [`ResultMismatch`]).
 */
export function parseResult<T>(schema: ResultSchema<T>, value: unknown, subject: string): T {
  const parsed = schema.safeParse(value);
  if (parsed.success) return parsed.data as T;
  throw new ResultMismatch(subject, value, parsed.error?.issues ?? []);
}

/** A provider answered something other than a completion. */
export class ProviderFailure extends Error {
  readonly status: number;
  readonly body: string;

  constructor(model: string, status: number, body: string) {
    super(`\`${model}\` answered ${status}: ${body.slice(0, 400)}`);
    this.name = "ProviderFailure";
    this.status = status;
    this.body = body;
  }
}

/**
 * A request that never got an answer at all (PRD 5.9's `timeout`).
 *
 * Told apart from [`ProviderFailure`] because the two are different failover
 * conditions and from a `SyntaxError` because that one is not a condition: a
 * body that is not JSON is a response generated code must **reject**, and
 * failing over to the next route member would be asking a second provider about
 * the first one's malformed answer. So the socket is wrapped where it is used
 * ([`send`]) rather than classified by guessing at whatever `fetch` rejected
 * with, which differs between the two supported runtimes.
 */
export class ProviderUnreachable extends Error {
  constructor(model: string, cause: unknown) {
    super(`\`${model}\` answered nothing: ${describe(cause)}`);
    this.name = "ProviderUnreachable";
    this.cause = cause;
  }
}

// ---------------------------------------------------------------------------
// The environment, read where it is used
// ---------------------------------------------------------------------------

/**
 * One `${ENV}` reference, resolved at the moment a node needs it.
 *
 * `./index.ts` checks presence at process start (PRD 5.9) and this reads the
 * value. The read is late on purpose: importing `./graph.ts` builds descriptors
 * and constructs graphs, which a tool with no credentials — `tsc`, a graph
 * drawing, the construction gate — must be able to do.
 */
export function environmentValue(name: string, site: string): string {
  const value = process.env[name];
  if (value === undefined) {
    throw new Error(`\`${name}\` is not set (referenced by ${site})`);
  }
  return value;
}

/** One part of an interpolable string (grammar 4.3 class 2). */
export type Interpolation = string | { readonly env: string; readonly site: string };

/** Substitute a class-2 string's references (grammar 4.3). */
export function interpolate(parts: readonly Interpolation[]): string {
  return parts
    .map((part) => (typeof part === "string" ? part : environmentValue(part.env, part.site)))
    .join("");
}

// ---------------------------------------------------------------------------
// Policy: grammar 9
// ---------------------------------------------------------------------------

/** A resolved `retry:` block (grammar 9.1). */
export interface RetryPolicy {
  readonly max: number;
  readonly backoffMs: number;
  readonly multiplier: number;
  readonly maxBackoffMs?: number;
  readonly jitter: boolean;
}

/** What `on_error:` does once retries are exhausted (grammar 9.2). */
export type ErrorStrategy = "fail" | "skip" | { readonly fallback: string };

/** One node's resolved policy — levels 2, 3 and 4 of grammar 9.3's chain. */
export interface NodePolicy {
  readonly retry?: RetryPolicy;
  readonly timeoutMs?: number;
  readonly onError: ErrorStrategy;
}

/**
 * Level **1** of grammar 9.3's chain: the `policy:` of the `flow:` node that
 * instantiated the enclosing flow instance.
 *
 * It is carried in the instance's own `$run` rather than compiled into a node,
 * because it is a property of the *instantiation site* and one flow may be
 * instantiated from several of them — `flow.review_loop` under
 * `policy: { timeout: 30s }` here and with nothing there is the same emitted
 * node either way. `on_error` takes no `fallback` at this level: its target is a
 * node id of the flow the declaring node sits in, and this level names no flow
 * (§9.2, Decision D103).
 */
export interface InstancePolicy {
  readonly retry?: RetryPolicy;
  readonly timeoutMs?: number;
  readonly onError?: "fail" | "skip";
}

/**
 * Grammar 9.3 resolved for one node execution: its compiled levels 2–4, with
 * level 1 laid over them where the instance carries one.
 *
 * Level 1 wins per field, which is what "highest precedence first" says — and a
 * `human` node takes neither `timeout` nor `retry` from it, at this level as at
 * every other (Decision D102), which is what `exempt` marks.
 */
export function effectivePolicy(
  compiled: NodePolicy,
  exempt: boolean,
  override: InstancePolicy | undefined,
): NodePolicy {
  if (override === undefined) return compiled;
  return {
    ...(exempt ? {} : { retry: override.retry ?? compiled.retry }),
    ...(exempt
      ? {}
      : { timeoutMs: override.timeoutMs ?? compiled.timeoutMs }),
    onError: override.onError ?? compiled.onError,
  };
}

/**
 * A flow instance's run identity (grammar 4.1's `execution` root).
 *
 * `item_index` is the source-item index of the **innermost** enclosing `map`
 * dispatch. It is absent everywhere else, which is a property of the *site*
 * rather than of the composition — the same flow is a dispatch target at one
 * instantiation and a root instance at another — so an expression that reads it
 * outside a dispatch fails the execution rather than the build (Decision D115).
 */
export interface ExecutionIdentity {
  readonly id: string;
  readonly session_key: string;
  readonly item_index?: number;
}

/** What one node execution needs from the world. */
export interface RunContext {
  /** The flow instance's execution identity (grammar 4.1's `execution`). */
  readonly execution: ExecutionIdentity;
  /**
   * Aborted when the node's `timeout:` budget runs out.
   *
   * With one deliberate exception: a **detached** `map` dispatch runs under a
   * signal of its own, which nothing aborts. Its outcome is never observed
   * (Decision D94), so the node's deadline has nothing to say about it — and a
   * delivery the node's bound had merely delayed past that deadline would
   * otherwise be dropped on the floor rather than sent. See [`runMap`].
   */
  readonly signal: AbortSignal;
  /** How this attempt is addressed, for a message. */
  readonly node: string;
  /**
   * The idempotency key this effect carries to its receiver (grammar 9.4).
   *
   * Present at exactly one kind of call: a **detached** `map` dispatch to a
   * `tool.*` — the sink of PRD 5.6, whose outcome is never observed and whose
   * delivery is therefore at-least-once. Every other call leaves it absent,
   * because grammar 9.4 fixes the carriers at two ("a detached `map` dispatch
   * and a store write") and a key on a call that is not one of them would be a
   * key a receiver dedupes on for effects that are *meant* to repeat.
   *
   * **The name is normative.** Grammar 9.4's delivery surface says a
   * `function:`-bound target "receives it as the `idempotency_key` field of its
   * invocation context", and the invocation context of a host function is this
   * object ([`HostFunction`]) — so this field *is* that surface and is spelled
   * the way the spec spells it rather than the way the rest of this file spells
   * a name. The other two carriers are named there too, and see [`runHttp`] and
   * [`runExec`] for the slot each puts it in.
   */
  readonly idempotency_key?: string;
  /**
   * Where a store op writes what it did, for this node's trace entry (PRD 5.8).
   *
   * The array is the **node execution's**, created by [`runNode`] and handed to
   * every attempt, because both surfaces of a store reach a store from inside an
   * activity and neither can return a record on its own: a store-op node answers
   * with the op's result, and an agent's synthesized store tool answers the
   * model. Reads land here because PRD 5.8 makes them recorded effects — "replay
   * consumes history, not the live store" — and writes land here because the
   * idempotency key they carried, and whether the backend had already seen it,
   * are the whole of what at-least-once delivery means at this store.
   *
   * Mutable behind a `readonly` field on purpose: the field is the channel, and
   * what flows through it is appended by whoever runs an op.
   */
  readonly storeRecords?: StoreRecord[];
}

/**
 * The context a **detached** dispatch runs its sink under: this node's, plus
 * the key grammar 8.6 rule 7 says the dispatch receives.
 *
 * Emitted at the dispatch site rather than applied to every scoped context in
 * [`runMap`], because which dispatches carry a key is a property of the *route*
 * and its target, and a compiled `graph.ts` is where a reader should be able to
 * see which of its deliveries is keyed.
 */
export function delivering(context: RunContext, site: DispatchSite): RunContext {
  return { ...context, idempotency_key: site.idempotencyKey };
}

const sleep = (ms: number, signal: AbortSignal): Promise<void> =>
  new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(signal.reason instanceof Error ? signal.reason : new Error("aborted"));
      return;
    }
    const timer = setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal.reason instanceof Error ? signal.reason : new Error("aborted"));
    };
    signal.addEventListener("abort", onAbort, { once: true });
  });

/**
 * A promise that never resolves and rejects with `signal`'s reason when it
 * aborts.
 *
 * The half of a deadline an `AbortSignal` alone cannot supply: something to
 * *race*. See [`runActivity`].
 */
function untilAborted(signal: AbortSignal): Promise<never> {
  return new Promise<never>((_, reject) => {
    const fail = () =>
      reject(signal.reason instanceof Error ? signal.reason : new Error("aborted"));
    if (signal.aborted) {
      fail();
      return;
    }
    signal.addEventListener("abort", fail, { once: true });
  });
}

/** The delay before attempt `attempt` (1 = the first retry), per grammar 9.1. */
export function backoffFor(policy: RetryPolicy, attempt: number): number {
  const raw = policy.backoffMs * policy.multiplier ** (attempt - 1);
  const capped = policy.maxBackoffMs === undefined ? raw : Math.min(raw, policy.maxBackoffMs);
  // Full jitter: uniform over [0, capped]. The one nondeterminism in a run, and
  // it moves *when* an attempt happens, never what it answers.
  return policy.jitter ? Math.random() * capped : capped;
}

/**
 * Run one activity under its node's policy: attempts, backoff, and one deadline
 * across all of them (grammar 9.1, 9.2).
 *
 * Answers the activity's value, or throws — `on_error` is the router's to
 * apply, because `skip` and `fallback` are routing outcomes rather than values
 * (grammar 9.2, Decision D97).
 *
 * # The deadline is raced, not merely signalled
 *
 * `context.signal` is the **cooperative** half of the budget, and the activities
 * that observe it stop on time: `fetch` rejects when its signal aborts, and a
 * spawned child is killed and its promise rejected. It cannot be the whole
 * mechanism, because one activity is arbitrary caller code — a `function:`
 * binding (grammar 6.1), whether it is the node itself or a tool called inside
 * an agent's loop — and a host implementation that never looks at
 * `context.signal` would otherwise run to completion and write its result long
 * after the budget it was given. Grammar 9.2 bounds **one node execution** with
 * no exemption for a kind, so the deadline is raced against the activity: the
 * node fails on time whatever the activity does about the signal.
 *
 * What racing does not do is *stop* the work. An abandoned host function keeps
 * running to whatever it was going to do, and JavaScript offers no way to
 * unschedule it; what the node stops doing is waiting for it, and its value is
 * discarded when it arrives. That is the whole of what a timeout can mean here,
 * and the generated `README.md` says so where a host reads about registering
 * one.
 *
 * **One activity is an exception**, and it is the one where "keeps running" is a
 * whole graph rather than a call: a `flow:` node's instance, and a joined `map`
 * dispatch to a `flow.*`. An instance is a run of its own, so `context.signal`
 * is handed to it ([`Instantiation.signal`]) and LangGraph stops scheduling its
 * supersteps — leaving one abandoned activity inside it, exactly as above,
 * rather than every node the instance had left to run.
 */
export async function runActivity<T>(
  flow: string,
  node: string,
  policy: NodePolicy,
  execution: RunContext["execution"],
  activity: (context: RunContext) => Promise<T>,
  storeRecords?: StoreRecord[],
): Promise<{ value: T; attempts: number }> {
  const attempts = 1 + (policy.retry?.max ?? 0);
  const controller = new AbortController();
  const budget = policy.timeoutMs;
  let expired = false;
  // What the node *did*, not what its policy allowed: a budget that ran out
  // during the second of three attempts made two, and a trace that reported
  // three would describe a run that did not happen. Declared out here because
  // the abort reason is read too — it becomes the `cause` of the failure below,
  // which a run prints under the message, and two different counts in one report
  // is worse than either.
  let made = 0;
  const timer =
    budget === undefined
      ? undefined
      : setTimeout(() => {
          expired = true;
          controller.abort(new NodeTimeout(node, budget, made));
        }, budget);
  // One rejection for the whole call rather than one per attempt: its listener
  // is registered before any activity starts, so it is the *first* reaction to
  // the abort and the reason a race sees is the `NodeTimeout` rather than
  // whatever the activity made of the signal. `Promise.race` attaches a handler
  // to it on every attempt, so a rejection nobody is waiting on is still a
  // handled one.
  const expiry = budget === undefined ? undefined : untilAborted(controller.signal);

  try {
    let last: unknown;
    for (let attempt = 1; attempt <= attempts; attempt += 1) {
      if (expired) break;
      made = attempt;
      try {
        const running = activity({
          execution,
          signal: controller.signal,
          node,
          ...(storeRecords === undefined ? {} : { storeRecords }),
        });
        // The loser of the race rejects with nobody awaiting it — an activity
        // that observes the abort, after the deadline has already answered for
        // the node — and in Node an unhandled rejection ends the process. This
        // is the handler that keeps an abandoned activity from taking the run
        // down with it.
        if (expiry !== undefined) running.catch(() => {});
        const value =
          expiry === undefined ? await running : await Promise.race([running, expiry]);
        return { value, attempts: attempt };
      } catch (error) {
        last = error;
        if (expired) break;
        if (attempt === attempts) break;
        const delay = backoffFor(policy.retry!, attempt);
        try {
          await sleep(delay, controller.signal);
        } catch {
          break;
        }
      }
    }
    if (expired) {
      throw new NodeFailure(
        flow,
        node,
        made,
        new NodeTimeout(node, budget ?? 0, made).message,
        last,
      );
    }
    throw new NodeFailure(flow, node, made, describe(last), last);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

function describe(error: unknown): string {
  if (error instanceof Error) return `${error.name}: ${error.message}`;
  return String(error);
}

// ---------------------------------------------------------------------------
// Providers and models
// ---------------------------------------------------------------------------

/**
 * Every `kind:` grammar 12.1 admits — including the two this runtime cannot
 * call.
 *
 * Four of them speak an HTTP surface. `bedrock` and `vertex` are reached
 * through a cloud SDK, which is why grammar 12.1's rows give them neither
 * `base_url:` nor `headers:`: there is no bare endpoint to point at. They are
 * here because the type has to describe every binding the compiler emits — a
 * composition declaring one is valid, and `build` writes its project — and
 * [`callModel`] is where the gap is reported, once, in the run that reached it.
 */
export type ProviderKind =
  | "anthropic"
  | "openai"
  | "openai_compatible"
  | "azure_openai"
  | "bedrock"
  | "vertex";

/** A resolved `provider.*` (grammar 12.1). Values are read when a node runs. */
export interface ProviderBinding {
  readonly address: string;
  readonly kind: ProviderKind;
  readonly apiKey?: string;
  readonly baseUrl?: string;
  readonly apiVersion?: string;
  readonly organization?: string;
  readonly headers?: Readonly<Record<string, string>>;
}

/** A resolved `model.*` in its direct form (grammar 12.2). */
export interface ModelBinding {
  readonly address: string;
  readonly id: string;
  readonly provider: ProviderBinding;
  readonly settings: Readonly<Record<string, unknown>>;
}

/**
 * A `model.*` in its **route** form: an ordered failover ladder (grammar 12.2,
 * PRD 5.9).
 *
 * Members are direct bindings — grammar 12.2 forbids a nested route, which is
 * what keeps "served by `model.fast`, fallback #1" a flat, unambiguous ordinal
 * (Decision D39). `routeOn` is non-empty for the same reason `route_on: []` is a
 * compile error: a route that never fails over is a direct model.
 */
export interface ModelRoute {
  readonly address: string;
  readonly route: readonly ModelBinding[];
  readonly routeOn: readonly RouteCondition[];
}

/** What an agent's `model:` resolves to: one binding, or a ladder. */
export type ModelSelection = ModelBinding | ModelRoute;

/** Grammar 12.2's `route_on:` vocabulary — infrastructure conditions only. */
export type RouteCondition = "rate_limit" | "overloaded" | "timeout" | "server_error";

/** Whether this selection is the route form. */
function isRoute(selection: ModelSelection): selection is ModelRoute {
  return Array.isArray((selection as ModelRoute).route);
}

/** The ladder a selection is tried in: a route's members, or the one binding. */
function ladder(selection: ModelSelection): readonly ModelBinding[] {
  return isRoute(selection) ? selection.route : [selection];
}

/**
 * One member's refusal, as the trace records it.
 *
 * The condition is what `route_on:` is written in, so a reader can see both that
 * the failover happened and that the composition asked for it.
 */
export interface Failover {
  /** The member that refused. */
  readonly model: string;
  /** Which of grammar 12.2's conditions it refused with. */
  readonly condition: RouteCondition;
  /** What it said, for a reader who has to fix it. */
  readonly detail: string;
}

/**
 * What one model call did, as the trace records it (PRD 5.9).
 *
 * PRD 5.9 asks for failover to be "deterministic runtime behavior recorded in
 * the trace (`served by model.fast, fallback #1`)", and this is that record:
 * the `model.*` the agent named, the member that answered, its ordinal in the
 * route — `0` for the first member, so a `fallback` of `1` reads as "fallback
 * #1" — and every member that refused on the way, with the condition it refused
 * with.
 *
 * A direct binding produces one too, with `model === servedBy`, `fallback: 0`
 * and no failovers: a trace that recorded only the interesting calls would leave
 * a reader unable to tell a call that did not fail over apart from a call
 * nothing recorded at all.
 */
export interface ModelCall {
  readonly model: string;
  readonly servedBy: string;
  readonly fallback: number;
  readonly failovers: readonly Failover[];
}

/**
 * Which of grammar 12.2's conditions a failure is, or `undefined` when it is
 * none of them.
 *
 * The classification is the whole of what `route_on:` selects over, so it is
 * stated once, here:
 *
 * | what happened | condition |
 * |---|---|
 * | HTTP 429 | `rate_limit` |
 * | HTTP 503, HTTP 529 | `overloaded` — the two spellings of an overloaded provider (Anthropic answers 529, the OpenAI surface 503) |
 * | any other 5xx | `server_error` |
 * | no answer at all: a dropped connection, a refused socket, a name that did not resolve | `timeout` |
 * | anything else — a 4xx that is not 429, a body that is not JSON | **not a condition** |
 *
 * A failure that is not a condition, and a condition the route did not declare,
 * both fail the node rather than moving to the next member: `route_on:` is a
 * declaration, and a runtime that failed over on everything would make it mean
 * nothing.
 */
export function classify(error: unknown): RouteCondition | undefined {
  if (error instanceof ProviderUnreachable) return "timeout";
  if (!(error instanceof ProviderFailure)) return undefined;
  if (error.status === 429) return "rate_limit";
  if (error.status === 503 || error.status === 529) return "overloaded";
  if (error.status >= 500 && error.status <= 599) return "server_error";
  return undefined;
}

/** A JSON Schema, as `codegen::schema`'s JSON column publishes it. */
export type JsonSchema = Readonly<Record<string, unknown>>;

/** One tool as a provider is told about it. */
export interface ToolSpec {
  readonly name: string;
  readonly description: string;
  readonly schema: JsonSchema;
}

/** One turn of the conversation, in a shape both surfaces can render. */
export type Turn =
  | { readonly role: "user"; readonly text: string }
  | {
      readonly role: "assistant";
      readonly text?: string;
      readonly toolCalls?: readonly { id: string; name: string; args: unknown }[];
      /**
       * The content blocks the model itself sent, when this turn is one being
       * replayed to the surface that produced it (see [`ModelAnswer.content`]).
       * Present, they are the turn: `text` and `toolCalls` are a *reading* of an
       * answer and a replay must be the answer.
       */
      readonly blocks?: readonly unknown[];
    }
  | {
      readonly role: "tool";
      readonly results: readonly { id: string; name: string; content: string }[];
    };

/** What a model answered. */
export interface ModelAnswer {
  readonly text: string | null;
  readonly toolCalls: readonly { id: string; name: string; args: unknown }[];
  readonly structured: unknown | null;
  readonly stopReason: string | null;
  /**
   * The answer's own content blocks, on a surface that sends them.
   *
   * A tool loop replays its assistant turns, and a turn rebuilt from `text` and
   * `toolCalls` is not the turn the model sent: the Messages API also answers
   * with `thinking` blocks — which a model with `settings: { thinking: … }`
   * sends as a matter of course, and which it requires back unaltered beside the
   * `tool_use` blocks they preceded. So the loop replays these when they are
   * here, and only falls back to the reading when they are not (Chat
   * Completions, whose answer is a message rather than a block list).
   */
  readonly content?: readonly unknown[];
  /**
   * The reason the model gave for declining, on a surface that states one.
   *
   * Chat Completions answers a refusal with `content: null` and a `refusal`
   * string, which is otherwise indistinguishable from an answer cut short by
   * `max_tokens` — both arrive as no structured output. Carrying it lets the
   * node error say which happened.
   */
  readonly refusal?: string | null;
}

/**
 * The `max_tokens` an Anthropic request carries when the model declares none.
 *
 * The Messages API requires the key (`WIRE-NOTES` (8)), and grammar 12.2 puts
 * the knob in `settings:` where an author sets it — so this is the value a
 * composition that did not is given, not a policy.
 */
const ANTHROPIC_MAX_TOKENS = 4096;

/**
 * Whether OpenAI's structured-output decoder can be asked to close this schema.
 *
 * `strict: true` is a promise the *service* checks: every object closed with
 * `additionalProperties: false`, and every declared property listed in
 * `required`, all the way down (`WIRE-NOTES` (13)). A schema that breaks either
 * is answered 400 rather than decoded loosely, so asking for it here would make
 * a legal composition unrunnable.
 *
 * Answering `false` is not free, and what it costs is written down rather than
 * discovered: under `strict: false` the decoder is not constrained by the schema
 * at all, so the model can answer something the emitted Zod then refuses — the
 * one property PRD 9.16 makes structured output load-bearing for. The
 * compiler's `codegen::runtime` module records that as
 * `strict-is-refused-by-an-optional-property`, with the alternatives and why
 * each is worse. The reachable way in is `optional:` on an object nested in an
 * agent's `output:`; the contract then lives in the parse, which runs over the
 * same schema either way.
 */
function strictable(schema: unknown): boolean {
  if (typeof schema !== "object" || schema === null) return true;
  const object = schema as Record<string, unknown>;
  if (object["type"] === "object") {
    if (object["additionalProperties"] !== false) return false;
    const properties = (object["properties"] ?? {}) as Record<string, unknown>;
    const required = (object["required"] ?? []) as unknown[];
    for (const name of Object.keys(properties)) {
      if (!required.includes(name)) return false;
    }
  }
  for (const value of Object.values(object)) {
    if (Array.isArray(value)) {
      if (!value.every(strictable)) return false;
    } else if (!strictable(value)) {
      return false;
    }
  }
  return true;
}

function baseUrl(provider: ProviderBinding): string {
  if (provider.baseUrl !== undefined) return provider.baseUrl.replace(/\/+$/, "");
  if (provider.kind === "anthropic") return "https://api.anthropic.com";
  if (provider.kind === "openai") return "https://api.openai.com";
  // `openai_compatible` and `azure_openai` require `base_url:` (grammar 12.1),
  // so reaching this is a binding that lost one between the spec and the
  // process — not an author who forgot to declare it.
  throw new Error(
    `\`${provider.address}\` is \`kind: ${provider.kind}\` and reached the wire with no \`base_url:\``,
  );
}

/**
 * One header set out of the layers that compose it, later layers winning.
 *
 * Header names are **case-insensitive** (grammar 6.1, 12.1) and `fetch` is not:
 * it builds its `Headers` by *appending* each key of the object it is handed, so
 * `Content-Type` beside `content-type` reaches the server as one header carrying
 * both values joined by a comma — two media types in the field an API dispatches
 * on. Folding the name is what makes a declared header replace the one this
 * runtime would have sent rather than join it; the spelling that won is the one
 * that goes out, because a server is entitled to read the name it was sent.
 */
function headerSet(
  ...layers: readonly (Readonly<Record<string, string>> | undefined)[]
): Record<string, string> {
  const folded = new Map<string, [string, string]>();
  for (const layer of layers) {
    for (const [name, value] of Object.entries(layer ?? {})) {
      folded.set(name.toLowerCase(), [name, value]);
    }
  }
  return Object.fromEntries(folded.values());
}

async function send(
  model: ModelBinding,
  url: string,
  headers: Record<string, string>,
  body: unknown,
  signal: AbortSignal,
): Promise<Record<string, unknown>> {
  // The type is spelled from `fetch` itself rather than as `Response`: the
  // global type comes from the runtime's own library types, and naming it here
  // would tie this module to one of them.
  let response: Awaited<ReturnType<typeof fetch>>;
  let text: string;
  try {
    response = await fetch(url, {
      method: "POST",
      headers: headerSet({ "content-type": "application/json" }, headers, model.provider.headers),
      body: JSON.stringify(body),
      signal,
    });
    text = await response.text();
  } catch (error) {
    // The node's own deadline reaches here as an abort, and it is not the
    // provider's failure to answer: it is the budget grammar 9.2 gave the node,
    // and failing over would spend what is left of it on a second provider.
    if (signal.aborted) throw error;
    throw new ProviderUnreachable(model.address, error);
  }
  if (!response.ok) {
    throw new ProviderFailure(model.address, response.status, text);
  }
  return JSON.parse(text) as Record<string, unknown>;
}

/** What one model call answered, and which member of its route answered it. */
export interface ModelResult {
  readonly answer: ModelAnswer;
  readonly served: ModelCall;
}

/**
 * One model call: the failover ladder, then the surface the answering member's
 * provider kind reaches (PRD 5.9, grammar 12.2).
 *
 * A direct binding is a ladder of one, so there is a single path through here
 * and a single shape of record out of it. Each member is tried in **declaration
 * order** — the order is the composition's, and a route that reordered itself
 * would not be deterministic — and a member that refuses moves the call on
 * exactly when two things hold: the failure classifies as one of grammar 12.2's
 * conditions ([`classify`]), and that condition is in this route's `route_on:`.
 * Anything else is raised, which is what makes `route_on:` a declaration rather
 * than a description.
 *
 * The **last** member is not special-cased: when it refuses, its own error is
 * what the node sees, with the earlier refusals in the message so a reader is
 * not left wondering why one 429 ended a two-member route.
 */
export async function callModel(
  selection: ModelSelection,
  request: {
    readonly system: string;
    readonly turns: readonly Turn[];
    readonly tools: readonly ToolSpec[];
    readonly pinned?: ToolSpec;
  },
  signal: AbortSignal,
): Promise<ModelResult> {
  const members = ladder(selection);
  const routeOn: readonly RouteCondition[] = isRoute(selection) ? selection.routeOn : [];
  const failovers: Failover[] = [];

  for (let ordinal = 0; ordinal < members.length; ordinal += 1) {
    const model = members[ordinal]!;
    try {
      const answer = await callDirect(model, request, signal);
      return {
        answer,
        served: {
          model: selection.address,
          servedBy: model.address,
          fallback: ordinal,
          failovers: [...failovers],
        },
      };
    } catch (error) {
      const condition = classify(error);
      const last = ordinal + 1 === members.length;
      if (condition === undefined || !routeOn.includes(condition) || last) {
        throw failoverContext(selection, failovers, error);
      }
      failovers.push({ model: model.address, condition, detail: describe(error) });
    }
  }
  // Unreachable: grammar 12.2 requires at least two members on a route and a
  // direct binding is a ladder of one, so the loop always answers or throws.
  throw new Error(`\`${selection.address}\` has no route member to call`);
}

/**
 * The error a spent route raises: the failure that ended it, with what the
 * members before it did.
 *
 * The refusals are on the message rather than only in the trace because this is
 * what a node failure prints, and "`model.smart` answered 429" alone would
 * describe a route as though it were a binding.
 */
function failoverContext(
  selection: ModelSelection,
  failovers: readonly Failover[],
  error: unknown,
): unknown {
  if (failovers.length === 0) return error;
  const tried = failovers
    .map((one) => `\`${one.model}\` (${one.condition})`)
    .join(", ");
  return new Error(
    `\`${selection.address}\` spent its route: ${tried} failed over, and the next member also refused: ${describe(error)}`,
    { cause: error },
  );
}

/** One call to one direct binding, on whichever surface its kind reaches. */
async function callDirect(
  model: ModelBinding,
  request: {
    readonly system: string;
    readonly turns: readonly Turn[];
    readonly tools: readonly ToolSpec[];
    readonly pinned?: ToolSpec;
  },
  signal: AbortSignal,
): Promise<ModelAnswer> {
  // Grammar 12.1's two SDK-reached kinds. There is no request to compose for
  // them — no endpoint, no header set, and a signing scheme that belongs to a
  // cloud SDK — so the gap is reported here rather than as a malformed call.
  // `base_url:` is not the fix: those rows do not take one.
  if (model.provider.kind === "bedrock" || model.provider.kind === "vertex") {
    throw new Error(
      `\`${model.provider.address}\` is \`kind: ${model.provider.kind}\`, which is reached through a cloud SDK rather than an HTTP endpoint (grammar 12.1) and which this compiler release does not call: bind \`${model.address}\` to an \`anthropic\`, \`openai\`, \`openai_compatible\` or \`azure_openai\` provider`,
    );
  }
  return model.provider.kind === "anthropic"
    ? await callMessages(model, request, signal)
    : await callChatCompletions(model, request, signal);
}

async function callMessages(
  model: ModelBinding,
  request: {
    readonly system: string;
    readonly turns: readonly Turn[];
    readonly tools: readonly ToolSpec[];
    readonly pinned?: ToolSpec;
  },
  signal: AbortSignal,
): Promise<ModelAnswer> {
  const settings = { ...model.settings };
  const maxTokens = settings["max_tokens"] ?? ANTHROPIC_MAX_TOKENS;
  delete settings["max_tokens"];

  const messages = request.turns.map((turn) => {
    if (turn.role === "user") return { role: "user", content: turn.text };
    if (turn.role === "tool") {
      return {
        role: "user",
        content: turn.results.map((result) => ({
          type: "tool_result",
          tool_use_id: result.id,
          content: result.content,
        })),
      };
    }
    // A turn the model sent goes back exactly as it came — thinking blocks and
    // all, which the Messages API requires unaltered beside the `tool_use`
    // blocks they preceded. Only a turn this runtime *composed* (the shared
    // history channel of grammar 10.4) is rendered from its parts.
    if (turn.blocks !== undefined) return { role: "assistant", content: [...turn.blocks] };
    const content: unknown[] = [];
    if (turn.text !== undefined && turn.text !== "") content.push({ type: "text", text: turn.text });
    for (const call of turn.toolCalls ?? []) {
      content.push({ type: "tool_use", id: call.id, name: call.name, input: call.args });
    }
    return { role: "assistant", content };
  });

  const offered = [...request.tools, ...(request.pinned === undefined ? [] : [request.pinned])];
  const body: Record<string, unknown> = {
    model: model.id,
    max_tokens: maxTokens,
    system: request.system,
    messages,
    ...settings,
  };
  if (offered.length > 0) {
    body["tools"] = offered.map((tool) => ({
      name: tool.name,
      description: tool.description,
      input_schema: tool.schema,
    }));
  }
  if (request.pinned !== undefined) {
    body["tool_choice"] = { type: "tool", name: request.pinned.name };
  }

  const answer = await send(
    model,
    `${baseUrl(model.provider)}/v1/messages`,
    {
      "x-api-key": model.provider.apiKey ?? "",
      "anthropic-version": "2023-06-01",
    },
    body,
    signal,
  );

  const blocks = (answer["content"] ?? []) as { type: string; [key: string]: unknown }[];
  const texts = blocks.filter((block) => block.type === "text").map((block) => block["text"]);
  const uses = blocks.filter((block) => block.type === "tool_use");
  const pinnedUse =
    request.pinned === undefined
      ? undefined
      : uses.find((use) => use["name"] === request.pinned!.name);
  return {
    text: texts.length > 0 ? texts.join("") : null,
    toolCalls: uses
      .filter((use) => use !== pinnedUse)
      .map((use) => ({
        id: String(use["id"]),
        name: String(use["name"]),
        args: use["input"],
      })),
    structured: pinnedUse === undefined ? null : (pinnedUse["input"] ?? null),
    stopReason: (answer["stop_reason"] as string | null) ?? null,
    content: blocks,
  };
}

async function callChatCompletions(
  model: ModelBinding,
  request: {
    readonly system: string;
    readonly turns: readonly Turn[];
    readonly tools: readonly ToolSpec[];
    readonly pinned?: ToolSpec;
  },
  signal: AbortSignal,
): Promise<ModelAnswer> {
  const messages: Record<string, unknown>[] = [{ role: "system", content: request.system }];
  for (const turn of request.turns) {
    if (turn.role === "user") {
      messages.push({ role: "user", content: turn.text });
      continue;
    }
    if (turn.role === "tool") {
      for (const result of turn.results) {
        messages.push({ role: "tool", tool_call_id: result.id, content: result.content });
      }
      continue;
    }
    const message: Record<string, unknown> = { role: "assistant", content: turn.text ?? null };
    if (turn.toolCalls !== undefined && turn.toolCalls.length > 0) {
      message["tool_calls"] = turn.toolCalls.map((call) => ({
        id: call.id,
        type: "function",
        function: { name: call.name, arguments: JSON.stringify(call.args) },
      }));
    }
    messages.push(message);
  }

  const body: Record<string, unknown> = { model: model.id, messages, ...model.settings };
  if (request.tools.length > 0) {
    body["tools"] = request.tools.map((tool) => ({
      type: "function",
      function: { name: tool.name, description: tool.description, parameters: tool.schema },
    }));
  }
  if (request.pinned !== undefined) {
    // `response_format` rather than a forced function: the schema shapes the
    // content and leaves the request's own `tools` callable, which is what an
    // agent whose loop has just ended still has on offer (WIRE-NOTES (3)).
    body["response_format"] = {
      type: "json_schema",
      json_schema: {
        name: request.pinned.name,
        strict: strictable(request.pinned.schema),
        schema: request.pinned.schema,
      },
    };
  }

  const provider = model.provider;
  const { headers, query } = openAiRequest(provider);
  const path = provider.kind === "azure_openai" ? "/openai/v1/chat/completions" : "/v1/chat/completions";

  const answer = await send(model, `${baseUrl(provider)}${path}${query}`, headers, body, signal);
  const choice = ((answer["choices"] ?? []) as Record<string, unknown>[])[0] ?? {};
  const message = (choice["message"] ?? {}) as Record<string, unknown>;
  const content = (message["content"] ?? null) as string | null;
  const calls = ((message["tool_calls"] ?? []) as Record<string, unknown>[]).map((call) => {
    const fn = (call["function"] ?? {}) as Record<string, unknown>;
    return {
      id: String(call["id"]),
      name: String(fn["name"]),
      args: JSON.parse(String(fn["arguments"] ?? "{}")) as unknown,
    };
  });
  return {
    text: content,
    toolCalls: request.pinned === undefined ? calls : [],
    structured:
      request.pinned === undefined || content === null ? null : (JSON.parse(content) as unknown),
    stopReason: (choice["finish_reason"] as string | null) ?? null,
    // A refusal is `content: null` beside a stated reason (WIRE-NOTES (3)). The
    // absence of content is what every other branch here sees; the reason is the
    // only thing that tells a refusal from a `max_tokens` cut.
    refusal: (message["refusal"] as string | null) ?? null,
  };
}

/**
 * How a request reaches an OpenAI-shaped surface: its auth header, and the
 * query an Azure connection needs.
 *
 * Shared by the two such surfaces this runtime speaks — Chat Completions and
 * Embeddings — because the difference between them is the route and the body,
 * never how a connection authenticates.
 */
function openAiRequest(provider: ProviderBinding): {
  headers: Record<string, string>;
  query: string;
} {
  const headers: Record<string, string> = {};
  if (provider.kind === "azure_openai") {
    headers["api-key"] = provider.apiKey ?? "";
  } else {
    headers["authorization"] = `Bearer ${provider.apiKey ?? ""}`;
    if (provider.organization !== undefined) {
      headers["openai-organization"] = provider.organization;
    }
  }
  const query =
    provider.kind === "azure_openai" && provider.apiVersion !== undefined
      ? `?api-version=${encodeURIComponent(provider.apiVersion)}`
      : "";
  return { headers, query };
}

// ---------------------------------------------------------------------------
// Embeddings: what turns a vector store's text into a vector (grammar 11.2)
// ---------------------------------------------------------------------------

/**
 * A vector store's `embed:` block, resolved (grammar 11.2, Decision D116).
 *
 * `provider` is the connection that **computes** the vectors and is a
 * `provider.*` like any other; where the vectors *live* is the store's
 * `backend:`, which is a different question and the one that forks per target.
 */
export interface EmbedBinding {
  /** The store this embeds for, for a message. */
  readonly store: string;
  /** `model:` — a provider-native embedding model id (Decision D36). */
  readonly model: string;
  /** `provider:` — the connection that serves the embeddings. */
  readonly provider: ProviderBinding;
  /** `dimensions:` — asserted against what the provider answers with. */
  readonly dimensions?: number;
}

/**
 * Embed one batch of texts, in order (grammar 11.2).
 *
 * The wire is OpenAI's `/v1/embeddings`, which is the surface every provider
 * kind the capability table marks as embedding-capable speaks — `anthropic` is
 * marked as not, and the validator refuses an `embed.provider` naming one, so
 * reaching here with one is a binding that lost its kind between the spec and
 * the process.
 *
 * `dimensions:` is **checked** rather than merely forwarded: grammar 11.2 says
 * it is "asserted against the backend's index", and a store whose declared width
 * silently disagreed with what the provider answered would fill an index with
 * vectors no later search could compare against.
 */
export async function callEmbeddings(
  embed: EmbedBinding,
  texts: readonly string[],
  signal: AbortSignal,
): Promise<number[][]> {
  const provider = embed.provider;
  if (provider.kind === "anthropic" || provider.kind === "bedrock" || provider.kind === "vertex") {
    throw new Error(
      `\`${embed.store}\` embeds through \`${provider.address}\`, whose \`${provider.kind}\` plugin this compiler release cannot reach for embeddings: name a provider of kind \`openai\`, \`openai_compatible\` or \`azure_openai\` (grammar 11.2)`,
    );
  }
  const { headers, query } = openAiRequest(provider);
  const path = provider.kind === "azure_openai" ? "/openai/v1/embeddings" : "/v1/embeddings";
  // `send` reports a refusal as a `ProviderFailure` naming its subject, which
  // here is the store rather than a model: an embeddings call belongs to a
  // store's `embed:` block and nothing about a `model.*` is involved.
  const answer = await send(
    { address: embed.store, id: embed.model, provider, settings: {} },
    `${baseUrl(provider)}${path}${query}`,
    headers,
    { model: embed.model, input: [...texts] },
    signal,
  );
  const data = (answer["data"] ?? []) as Record<string, unknown>[];
  if (data.length !== texts.length) {
    throw new Error(
      `\`${embed.store}\` asked \`${provider.address}\` to embed ${texts.length} text(s) and was answered ${data.length}`,
    );
  }
  // The API is documented to answer in request order, and it also carries an
  // `index` on every row; sorting by it is what makes the promise this code
  // relies on the *response's* rather than the documentation's.
  const ordered = [...data].sort(
    (left, right) => Number(left["index"] ?? 0) - Number(right["index"] ?? 0),
  );
  return ordered.map((row, position) => {
    const vector = row["embedding"];
    if (!Array.isArray(vector) || vector.some((value) => typeof value !== "number")) {
      throw new Error(
        `\`${embed.store}\` was answered an embedding that is not a list of numbers (row ${position})`,
      );
    }
    if (embed.dimensions !== undefined && vector.length !== embed.dimensions) {
      throw new Error(
        `\`${embed.store}\` declares \`dimensions: ${embed.dimensions}\` and \`${provider.address}\` answered a vector of ${vector.length}`,
      );
    }
    return vector as number[];
  });
}

// ---------------------------------------------------------------------------
// Agents: the intra-agent tool loop (grammar 5, PRD §9.14)
// ---------------------------------------------------------------------------

/** One tool an agent may call, and how the graph runs it. */
export interface AgentTool extends ToolSpec {
  readonly address: string;
  invoke(args: unknown, context: RunContext): Promise<unknown>;
}

/** Everything a compiled `agent:` node needs (grammar 5, 8.1). */
export interface AgentBinding {
  readonly address: string;
  readonly prompt: string;
  /** Its `model:` — a direct binding or a failover route (grammar 12.2). */
  readonly model: ModelSelection;
  readonly output: ToolSpec;
  readonly tools: readonly AgentTool[];
  readonly maxToolIterations: number;
}

/**
 * One agent node: the tool loop, then the pinned structured-output call.
 *
 * Two shapes, and which one runs is decided by the agent's own `tools:` list:
 *
 *  * **No tools** — one call, the output schema offered as the single tool and
 *    pinned by name. A pinned choice is a promise that the pinned tool is what
 *    gets called, which is exactly the ask PRD 5.2 makes.
 *  * **With tools** — the loop first, on calls that offer the agent's tools and
 *    pin **nothing** (a pinned choice would make the loop unreachable), and the
 *    pinned call is what ends it. The loop is bounded by `max_tool_iterations`
 *    (Decision D51, PRD §9.14): the bound counts model calls in the loop, and
 *    spending it is a node error rather than a silent stop, because a model that
 *    only ever asks for tools has not answered.
 *
 * The final call still offers the agent's tools beside the pinned one: the
 * history it carries holds `tool_use`/`tool_result` blocks, and both surfaces
 * refuse a request that carries those without declaring the tools they name.
 *
 * Every loop answer is replayed by [`replayed`], which is where an answer that
 * carried nothing at all stops the node instead of becoming an empty turn the
 * next request could not legally carry.
 */
export async function callAgent(
  agent: AgentBinding,
  input: unknown,
  history: readonly Turn[],
  context: RunContext,
): Promise<{ output: unknown; history: MessageLike[]; models: readonly ModelCall[] }> {
  const rendered = typeof input === "string" ? input : JSON.stringify(input);
  const turn: Turn = { role: "user", text: rendered };
  const turns: Turn[] = [...history, turn];
  // Every call this node makes, in the order it made them (PRD 5.9). A tool
  // loop makes several, and which member of a route served each one is a
  // separate fact about each.
  const models: ModelCall[] = [];

  if (agent.tools.length > 0) {
    let iterations = 0;
    for (;;) {
      if (iterations >= agent.maxToolIterations) {
        throw new Error(
          `\`${agent.address}\` reached its \`max_tool_iterations\` bound of ${agent.maxToolIterations} without answering (Decision D51)`,
        );
      }
      iterations += 1;
      const { answer, served } = await callModel(
        agent.model,
        { system: agent.prompt, turns, tools: agent.tools },
        context.signal,
      );
      models.push(served);
      turns.push(replayed(agent, answer));
      if (answer.toolCalls.length === 0) break;

      const results: { id: string; name: string; content: string }[] = [];
      for (const call of answer.toolCalls) {
        const tool = agent.tools.find((candidate) => candidate.name === call.name);
        if (tool === undefined) {
          throw new Error(
            `\`${agent.address}\` was answered with a call to \`${call.name}\`, which is not one of its tools`,
          );
        }
        const result = await tool.invoke(call.args, context);
        results.push({ id: call.id, name: call.name, content: JSON.stringify(result) });
      }
      turns.push({ role: "tool", results });
    }
  }

  const { answer: final, served: finalServed } = await callModel(
    agent.model,
    { system: agent.prompt, turns, tools: agent.tools, pinned: agent.output },
    context.signal,
  );
  models.push(finalServed);
  if (final.structured === null) {
    // Why there is no answer, where the surface said: a stated refusal first,
    // and otherwise the stop reason, which is what tells a `max_tokens` cut from
    // a model that simply sent nothing. Without either, "no structured output"
    // is true and says nothing a reader can act on.
    const why =
      final.refusal !== undefined && final.refusal !== null
        ? `: the model declined — ${final.refusal}`
        : final.stopReason === null
          ? ""
          : ` (\`stop_reason: ${final.stopReason}\`)`;
    throw new Error(
      `\`${agent.address}\` asked for \`${agent.output.name}\` and the answer carried no structured output${why}`,
    );
  }
  return {
    output: final.structured,
    history: [
      { role: "user", content: rendered },
      { role: "assistant", content: JSON.stringify(final.structured) },
    ],
    models,
  };
}

/**
 * The assistant turn one loop answer goes back into the conversation as.
 *
 * Two things happen here, and the second is why it is a function rather than an
 * object literal at the call site.
 *
 * The turn is the model's **own** content where the surface sent blocks, rather
 * than a turn rebuilt from the `text` and `toolCalls` this runtime read out of
 * them. Rebuilding drops whatever it does not read — a `thinking` block, which a
 * model with `settings: { thinking: … }` sends with every answer and which the
 * Messages API requires back unaltered beside the `tool_use` blocks it preceded.
 *
 * And an answer that carried **nothing** ends the node here, with a message
 * about the answer. A `max_tokens` cut can end a turn with an empty content
 * list, and the turn built from one is `{"role": "assistant", "content": []}` —
 * a message the Messages API refuses (`content: List should have at least 1
 * item`), as does `crates/mock-provider`, which holds this project to that same
 * wire. Sending it anyway would report the model's empty answer as a provider
 * 400 one call later, about the wrong request.
 */
function replayed(agent: AgentBinding, answer: ModelAnswer): Turn {
  const blocks = answer.content;
  const empty =
    blocks === undefined
      ? (answer.text ?? "") === "" && answer.toolCalls.length === 0
      : blocks.length === 0;
  if (empty) {
    const why = answer.stopReason === null ? "" : ` (\`stop_reason: ${answer.stopReason}\`)`;
    throw new Error(
      `\`${agent.address}\` was answered with no content${why}, so its tool loop has no turn to send back`,
    );
  }
  return {
    role: "assistant",
    text: answer.text ?? "",
    ...(answer.toolCalls.length > 0 ? { toolCalls: answer.toolCalls } : {}),
    ...(blocks === undefined ? {} : { blocks }),
  };
}

/**
 * What one agent node contributes to the shared history channel: its rendered
 * input, and its structured answer (grammar 10.4, PRD 5.7 tier 3).
 *
 * The intra-agent tool loop's own turns are **not** among them, and the reason
 * is a property of the surface rather than a preference: an assistant turn
 * carrying `tool_use` blocks is only well formed when the very next turn answers
 * every one of them, so a history that kept the loop's turns and was then
 * appended to by the *next* node would hand a provider a conversation it
 * refuses. The loop is the agent's own business — the trace records what it
 * called (PRD 5.3) — and what crosses into the shared channel is the exchange:
 * one user turn in, one assistant turn out, strictly alternating however many
 * agent nodes a flow instance runs.
 */
export type MessageLike = { readonly role: "user" | "assistant"; readonly content: string };

/** The shared history channel as an agent call reads it. */
export function historyTurns(messages: readonly unknown[]): Turn[] {
  return messages.map((message) => {
    const held = message as { role?: string; content?: unknown; getType?: () => string };
    const role = typeof held.getType === "function" ? held.getType() : (held.role ?? "user");
    const text = typeof held.content === "string" ? held.content : JSON.stringify(held.content);
    return role === "ai" || role === "assistant"
      ? ({ role: "assistant", text } as const)
      : ({ role: "user", text } as const);
  });
}

// ---------------------------------------------------------------------------
// `exec` and `http` (grammar 6.1, 8.2, 8.3)
// ---------------------------------------------------------------------------

/** How a result is read out of a process or a response (grammar 8.2, 8.3). */
export interface Decoding {
  /** Envelope fields the surface binds directly, by name. */
  readonly envelope: readonly string[];
  /** Fields decoded from the payload. */
  readonly decoded: readonly string[];
  /**
   * The single string property that takes the raw stream whole — the
   * `tool.*`-surface exception of grammar 6.1, absent on inline nodes (D91).
   *
   * What "whole" means is the surface's, not this descriptor's: see [`decode`].
   */
  readonly raw?: string;
  /** No result at all: nothing is decoded (grammar 6.1's `output: {}`). */
  readonly empty: boolean;
}

/** A resolved `exec:` block. */
export interface ExecBinding {
  readonly command: readonly Interpolation[];
  readonly args: readonly (readonly Interpolation[])[];
  readonly cwd?: readonly Interpolation[];
  readonly env: readonly { readonly name: string; readonly value: readonly Interpolation[] }[];
  readonly expectExit: readonly number[];
  readonly decoding: Decoding;
}

/** The environment-variable spelling of an input field (grammar 6.1). */
export function environmentName(field: string): string {
  return field.toUpperCase();
}

/**
 * The variable a subprocess sink reads its idempotency key out of — the name
 * grammar 9.4's delivery surface fixes for an `exec:`-bound target.
 *
 * The environment rather than the argument vector: `args:` is the author's exact
 * command line and a runtime that appended to it would be changing the program's
 * arguments — while the environment is the out-of-band channel a process already
 * has.
 *
 * The name is a plain one, so [`runExec`] clears it out of the inherited
 * environment first: a target receives it when this runtime delivered it, and
 * not because the process happened to be started with a variable by that name.
 *
 * [`environmentName`] spells an input field by upper-casing it, so a target that
 * declared an input field called `idempotency_key` would name this same
 * variable. **No compiled graph reaches that**: grammar 9.4 says the key is
 * "delivery metadata, never part of the target's declared input schema", and the
 * validator refuses a detached dispatch to an `exec:` sink declaring one rather
 * than letting either value be discarded in silence — the rule Decision D66
 * already applies to the binding's own `env:` (`check::maps`). What is left for
 * this function to settle is the order a *direct* call resolves it in, and it is
 * 9.4's: the delivery is written **after** the input fields, and **before** the
 * binding's own `env:` — the layer an author configures their wire from, which
 * sits over the delivery exactly as a declared header sits over the emitted
 * media type.
 */
export const IDEMPOTENCY_ENV = "IDEMPOTENCY_KEY";

/**
 * The header an HTTP sink reads its idempotency key out of — the name
 * grammar 9.4's delivery surface fixes for an `http:`-bound target, and the one
 * sinks already dedupe on. Header names are case-insensitive and [`headerSet`]
 * folds them, so this is the spelling that reaches the wire rather than a
 * second name for one header.
 */
export const IDEMPOTENCY_HEADER = "Idempotency-Key";

/** Run one `exec:` binding (grammar 6.1, 8.2). */
export async function runExec(
  binding: ExecBinding,
  input: unknown,
  context: RunContext,
): Promise<unknown> {
  const environment: Record<string, string> = { ...(process.env as Record<string, string>) };
  // The delivery slot starts empty, whatever the process was started with. The
  // name grammar 9.4 fixes is a plain one, and a variable that happened to be in
  // this process's environment would otherwise reach every `exec:` target in the
  // composition as a key a sink dedupes on — silently dropping repeat calls that
  // were meant to repeat. A target receives this variable when, and only when,
  // this runtime delivered it (or the binding's own `env:` set it, below).
  delete environment[IDEMPOTENCY_ENV];
  let stdin: string | undefined;
  if (typeof input === "string") {
    stdin = input;
  } else if (input !== null && typeof input === "object") {
    for (const [field, value] of Object.entries(input as Record<string, unknown>)) {
      environment[environmentName(field)] =
        typeof value === "string" ? value : JSON.stringify(value);
    }
  }
  // The key a detached delivery carries, where this call is one (grammar 8.6
  // rule 7, 9.4). Under the declared `env:` for the same reason a declared
  // header sits under the runtime's media type: what the binding wrote out is
  // the author's statement about this process's environment, and an automatic
  // addition that overwrote it would break a sink using its own scheme. Over the
  // input fields, which no compiled graph puts here — the validator refuses a
  // composition whose detached sink declares the slot (see [`IDEMPOTENCY_ENV`])
  // — because a delivery a sink cannot dedupe on is the failure the key exists
  // to prevent.
  if (context.idempotency_key !== undefined) {
    environment[IDEMPOTENCY_ENV] = context.idempotency_key;
  }
  for (const entry of binding.env) {
    environment[entry.name] = interpolate(entry.value);
  }

  const command = interpolate(binding.command);
  const args = binding.args.map((argument) => interpolate(argument));
  const result = await new Promise<{ code: number; stdout: string; stderr: string }>(
    (resolve, reject) => {
      const child = spawn(command, args, {
        cwd: binding.cwd === undefined ? undefined : interpolate(binding.cwd),
        env: environment,
        signal: context.signal,
      });
      let stdout = "";
      let stderr = "";
      child.stdout.on("data", (chunk: Buffer) => {
        stdout += chunk.toString();
      });
      child.stderr.on("data", (chunk: Buffer) => {
        stderr += chunk.toString();
      });
      child.on("error", reject);
      child.on("close", (code) => resolve({ code: code ?? -1, stdout, stderr }));
      // A command that never reads its input closes the pipe while this write is
      // still in flight, and Node reports that as an `error` **event on the
      // stream** rather than as a rejected promise. Unhandled, an EventEmitter
      // `error` aborts the whole process — past `retry`, `timeout`, `skip` and
      // `fallback` alike (grammar 9), past `runFlow`'s own `catch`, so the run
      // would not even leave the trace PRD 5.3 promises, and under `serve` it
      // would take every concurrent execution down with it. EPIPE is the child
      // declining the input rather than a failure — `printf` declines it by
      // design — so the exit code and the streams stay the result; a destroyed
      // stream is this run being aborted, which the abort path already reports;
      // anything else fails the node through the same promise a spawn error
      // does.
      child.stdin.on("error", (error: NodeJS.ErrnoException) => {
        if (error.code !== "EPIPE" && error.code !== "ERR_STREAM_DESTROYED") reject(error);
      });
      if (stdin !== undefined) {
        child.stdin.end(stdin);
      } else {
        child.stdin.end();
      }
    },
  );

  if (!binding.expectExit.includes(result.code)) {
    throw new Error(
      `\`${command}\` exited ${result.code}, which is outside \`expect_exit: [${binding.expectExit.join(", ")}]\`${result.stderr === "" ? "" : `: ${result.stderr.trim()}`}`,
    );
  }

  return decode(
    binding.decoding,
    result.stdout,
    {
      exit_code: result.code,
      stdout: result.stdout,
      stderr: result.stderr,
    },
    "trimmed",
  );
}

/** A resolved `http:` block. */
export interface HttpBinding {
  readonly method: string;
  readonly url: readonly Interpolation[];
  readonly headers: readonly { readonly name: string; readonly value: readonly Interpolation[] }[];
  readonly expectStatus: readonly number[] | "2xx";
  readonly decoding: Decoding;
}

/** Run one `http:` binding (grammar 6.1, 8.3). */
export async function runHttp(
  binding: HttpBinding,
  request: { readonly query?: Record<string, unknown>; readonly body?: unknown },
  context: RunContext,
): Promise<unknown> {
  const url = new URL(interpolate(binding.url));
  for (const [name, value] of Object.entries(request.query ?? {})) {
    url.searchParams.set(name, typeof value === "string" ? value : JSON.stringify(value));
  }
  const declared: Record<string, string> = {};
  for (const header of binding.headers) {
    declared[header.name] = interpolate(header.value);
  }
  const sendsBody = request.body !== undefined && !["GET", "HEAD"].includes(binding.method);
  // The body this runtime composes is JSON, so `application/json` is the media
  // type a binding that said nothing gets — and a binding that *did* say
  // something replaces it rather than adding to it. See [`headerSet`].
  //
  // The idempotency key rides in the same layering, one layer down from the
  // binding's own headers (grammar 8.6 rule 7, 9.4). A **header** rather than a
  // body field: the body of a `tool.*` request is the object its declared
  // `input:` parsed, so a field injected into it would be a property the tool's
  // own contract does not declare — and could collide with one it does. A
  // header is out of band, and `Idempotency-Key` is the name sinks already
  // dedupe on.
  const headers = headerSet(
    sendsBody ? { "content-type": "application/json" } : undefined,
    context.idempotency_key === undefined
      ? undefined
      : { [IDEMPOTENCY_HEADER]: context.idempotency_key },
    declared,
  );

  const response = await fetch(url, {
    method: binding.method,
    headers,
    body: sendsBody ? JSON.stringify(request.body) : undefined,
    signal: context.signal,
  });
  const text = await response.text();
  const accepted =
    binding.expectStatus === "2xx"
      ? response.status >= 200 && response.status < 300
      : binding.expectStatus.includes(response.status);
  if (!accepted) {
    throw new Error(
      `\`${url}\` answered ${response.status}, which is outside ${
        binding.expectStatus === "2xx" ? "the 2xx range" : `\`expect_status: [${binding.expectStatus.join(", ")}]\``
      }: ${text.slice(0, 200)}`,
    );
  }
  return decode(binding.decoding, text, { status: response.status, body: text }, "verbatim");
}

/**
 * Read a result out of a raw stream and an envelope (grammar 8.2, 8.3, 6.1).
 *
 * `binds` is how the single string-typed property of a `tool.*` takes the raw
 * stream, and the two surfaces differ there by one word of grammar 6.1: the
 * `exec` binding says **"trimmed raw stdout"**, the `http` binding says **"the
 * raw response text"**. The difference is the streams rather than the rule —
 * trailing whitespace is a shell artefact on stdout, where a command that ends
 * its output with a newline has said nothing by it, and is payload in a response
 * body, where every byte is what the server chose to send.
 */
function decode(
  decoding: Decoding,
  raw: string,
  envelope: Readonly<Record<string, unknown>>,
  binds: "trimmed" | "verbatim",
): unknown {
  if (decoding.empty) return {};
  if (decoding.raw !== undefined) {
    return { [decoding.raw]: binds === "trimmed" ? raw.trim() : raw };
  }

  const result: Record<string, unknown> = {};
  for (const field of decoding.envelope) {
    result[field] = envelope[field];
  }
  if (decoding.decoded.length === 0) return result;

  let payload: unknown;
  try {
    payload = JSON.parse(raw) as unknown;
  } catch (error) {
    throw new Error(`the result is not JSON: ${describe(error)}`);
  }
  if (payload === null || typeof payload !== "object") {
    throw new Error(`the result decoded to a ${typeof payload}, not an object`);
  }
  for (const field of decoding.decoded) {
    result[field] = (payload as Record<string, unknown>)[field];
  }
  return result;
}

// ---------------------------------------------------------------------------
// The host function registry (grammar 6.1's escape hatch)
// ---------------------------------------------------------------------------

/**
 * What a host-registered function is: arguments in, a result out.
 *
 * `context` is the invocation context grammar 9.4's delivery surface names: a
 * host function reached as the sink of a **detached** dispatch finds that
 * dispatch's key in `context.idempotency_key`, and finds it absent on every
 * other call. The generated `README.md` says so where a host reads about
 * registering one.
 */
export type HostFunction = (args: unknown, context: RunContext) => unknown | Promise<unknown>;

const REGISTRY = new Map<string, HostFunction>();

/**
 * Register the implementation of a `function:` tool binding (grammar 6.1).
 *
 * The escape hatch, and the one construct that breaks spec portability — a
 * composition using it needs this call to have happened before the graph runs.
 * See the generated `README.md`.
 */
export function registerFunction(name: string, implementation: HostFunction): void {
  REGISTRY.set(name, implementation);
}

/** Call a host-registered function, or say which registration is missing. */
export async function callFunction(
  name: string,
  args: unknown,
  context: RunContext,
): Promise<unknown> {
  const implementation = REGISTRY.get(name);
  if (implementation === undefined) {
    throw new Error(
      `no host function is registered as \`${name}\`: call \`registerFunction(${JSON.stringify(name)}, …)\` before running the graph`,
    );
  }
  return await implementation(args, context);
}

// ---------------------------------------------------------------------------
// The router (grammar 7.3, 7.4, 7.6)
// ---------------------------------------------------------------------------

/** One outgoing edge, in the declaration order grammar 7.3 evaluates them in. */
export interface EdgeDescriptor {
  /** The target node id, or `"__end__"`. */
  readonly to: string;
  /** The `when:` guard, as CEL source. */
  readonly when?: string;
  /** Whether the guard reads the source node's own output (Decision D97). */
  readonly readsOutput?: boolean;
  /** `else: true`. */
  readonly otherwise?: boolean;
  /** The `max_iterations` budget, and the counter it spends (grammar 7.4). */
  readonly budget?: { readonly key: string; readonly max: number };
}

/** What one edge did, as the trace records it (PRD 5.3). */
export interface EdgeDecision {
  readonly to: string;
  readonly when?: string;
  readonly else?: true;
  readonly value?: boolean;
  readonly budget?: { readonly key: string; readonly used: number; readonly max: number };
  readonly taken: boolean;
  readonly reason?: string;
}

/** One node's whole routing decision. */
export interface RoutingDecision {
  readonly edges: EdgeDecision[];
  readonly targets: string[];
  readonly counters: Record<string, number>;
}

/**
 * Evaluate a node's outgoing edges (grammar 7.3, 7.4, Decision D97).
 *
 * Declaration order, multicast, `else:` suppressed by a **taken** guarded
 * sibling, and an exhausted budget untakeable whatever its guard says. A
 * `skip`ped node changes exactly one thing: a guard that references its output
 * is `false` without being evaluated, while a guard over `input`/`state`/
 * `execution` is evaluated normally.
 */
export function route(
  flow: string,
  node: string,
  edges: readonly EdgeDescriptor[],
  roots: Roots,
  counters: Readonly<Record<string, number>>,
  skipped: boolean,
): RoutingDecision {
  const decisions: EdgeDecision[] = [];
  const targets: string[] = [];
  const spent: Record<string, number> = {};
  let guardedTaken = false;

  for (const edge of edges) {
    if (edge.otherwise === true) continue;
    if (edge.when === undefined) {
      decisions.push({ to: edge.to, taken: true, reason: "unconditional" });
      if (!targets.includes(edge.to)) targets.push(edge.to);
      continue;
    }

    let value: boolean;
    if (skipped && edge.readsOutput === true) {
      value = false;
    } else {
      try {
        value = evaluateGuard(edge.when, roots);
      } catch (error) {
        throw new Error(
          `${flow} node \`${node}\`: the guard \`${edge.when}\` could not be evaluated: ${describe(error)}`,
        );
      }
    }

    if (!value) {
      decisions.push({ to: edge.to, when: edge.when, value, taken: false });
      continue;
    }
    if (edge.budget !== undefined) {
      const used = counters[edge.budget.key] ?? 0;
      if (used >= edge.budget.max) {
        decisions.push({
          to: edge.to,
          when: edge.when,
          value,
          budget: { key: edge.budget.key, used, max: edge.budget.max },
          taken: false,
          reason: "the `max_iterations` budget is spent",
        });
        continue;
      }
      spent[edge.budget.key] = used + 1;
      decisions.push({
        to: edge.to,
        when: edge.when,
        value,
        budget: { key: edge.budget.key, used: used + 1, max: edge.budget.max },
        taken: true,
      });
    } else {
      decisions.push({ to: edge.to, when: edge.when, value, taken: true });
    }
    guardedTaken = true;
    if (!targets.includes(edge.to)) targets.push(edge.to);
  }

  for (const edge of edges) {
    if (edge.otherwise !== true) continue;
    if (guardedTaken) {
      decisions.push({
        to: edge.to,
        else: true,
        taken: false,
        reason: "a guarded sibling was taken",
      });
      continue;
    }
    decisions.push({ to: edge.to, else: true, taken: true });
    if (!targets.includes(edge.to)) targets.push(edge.to);
  }

  if (targets.length === 0) {
    // Ordered the way a returned decision is (below), because the trace entry
    // this ends up on is read the same way whether the run survived it or not.
    throw new NoViableRoute(flow, node, ordered(decisions, edges));
  }
  return { edges: ordered(decisions, edges), targets, counters: spent };
}

/**
 * The decisions in declaration order.
 *
 * Declaration order is what the trace records; the targets are ordered by it
 * too, so a multicast reads the way the file does.
 */
function ordered(
  decisions: EdgeDecision[],
  edges: readonly EdgeDescriptor[],
): EdgeDecision[] {
  decisions.sort(
    (left, right) =>
      edges.findIndex((edge) => edge.to === left.to && edge.when === left.when) -
      edges.findIndex((edge) => edge.to === right.to && edge.when === right.when),
  );
  return decisions;
}

// ---------------------------------------------------------------------------
// The run channel: what the compiler keeps beside a composition's own state
// ---------------------------------------------------------------------------

/**
 * One entry of the routing trace (PRD 5.3: routing decisions are data).
 *
 * A run that **fails** carries one final entry too: the node it aborted at,
 * with `outcome: "failed"` and an `error` saying what stopped it. That entry
 * records no `writes`, because the superstep a run dies in lands none of them —
 * every task's update in that step is discarded — and it carries `routing` only
 * when the routing decision is what failed ([`NoViableRoute`]), where the guard
 * values are the whole explanation. See [`abortedEntry`].
 */
export interface TraceEntry {
  readonly step: number;
  readonly flow: string;
  readonly node: string;
  readonly traversal: number;
  readonly outcome: "completed" | "skipped" | "failed";
  readonly attempts: number;
  readonly writes?: readonly string[];
  readonly routing?: RoutingDecision;
  /**
   * What a `map` node dispatched, one entry per source item in **index** order
   * (grammar 8.6, PRD 5.6).
   *
   * Cardinality and destination are data in this design, so they are trace data
   * too: which route each item took, whether the instance completed, was skipped
   * by `on_item_error`, failed under it, or was resolved at dispatch by
   * `detach: true`. Without it a fan-out is the one construct whose whole
   * decision — how many, and to where — leaves no record at all.
   *
   * A map node that **failed** carries it too, whether the failure ended the run
   * or its own `on_error:` absorbed it: the items that ran already had their
   * effects, and that is the reading of a fan-out an operator most needs. See
   * [`ItemFailure.dispatches`], which is how it gets out of a node that returned
   * no answer, and [`plannedDispatches`], which is how it gets out of a node
   * whose own `timeout:` fired before it could raise one. In that last case the
   * entry holds the dispatches that had **resolved** — every detached delivery,
   * and every joined instance that had settled — rather than one per source
   * item: an instance the deadline caught mid-flight has no outcome to record,
   * and the entry's `error` names the budget that ended it.
   */
  readonly dispatches?: readonly DispatchRecord[];
  /**
   * The trace of the subflow instance a `flow:` node ran (grammar 8.5).
   *
   * A subgraph's routing decisions are its own instance's, and a flat trace
   * would either lose them or pretend they were the caller's. Nesting them keeps
   * PRD 5.3's record complete across a module boundary without moving anyone's
   * step numbers.
   */
  readonly inner?: readonly TraceEntry[];
  /**
   * Every store op this node performed, in the order it performed them
   * (PRD 5.8).
   *
   * Both consumption surfaces land here: a `store:` node's own op, and every op
   * an agent's synthesized store tools ran inside its tool loop — which PRD 5.8
   * asks to be "recorded as tool calls", and a tool call this graph made is a
   * thing the trace has to hold rather than only the provider transcript.
   */
  readonly stores?: readonly StoreRecord[];
  /**
   * Every model call this node made, and which member of its route served it
   * (PRD 5.9).
   *
   * "Served by `model.fast`, fallback #1" is PRD 5.9's own phrasing of what a
   * trace must carry, so failover is data rather than behaviour a reader has to
   * infer from a provider's logs. See [`ModelCall`].
   */
  readonly models?: readonly ModelCall[];
  readonly error?: string;
  readonly fallback?: string;
}

/**
 * One store op, as the trace records it (PRD 5.8).
 *
 * Reads and writes are both here and are told apart by `effect`, because the
 * two are recorded for different reasons. A **read** is recorded because PRD 5.8
 * makes store ops effects whose reads replay from history rather than from the
 * live store: the answer is kept so a replay has something to consume. A
 * **write** is recorded because it is at-least-once — it carries the
 * idempotency key of grammar 9.4, and `deduped` says whether the backend had
 * already applied that key, which is the difference between "this run wrote it"
 * and "an earlier attempt of this same effect did".
 */
export interface StoreRecord {
  /** The store's typed address (grammar 2.2). */
  readonly store: string;
  /** The op, spelled as grammar 11.4 spells it. */
  readonly op: string;
  /** Which half of the replay discipline this record belongs to. */
  readonly effect: "read" | "write";
  /** Which consumption surface ran it (PRD 5.8's two modes). */
  readonly via: "node" | "tool";
  /** The store's lifetime, and with it which partition was addressed. */
  readonly scope: "execution" | "session" | "global";
  /** The key the op addressed, on the ops that address one. */
  readonly key?: string;
  /** What a read answered — the history a replay consumes. */
  readonly answer?: unknown;
  /** A write's idempotency key (grammar 9.4). */
  readonly idempotencyKey?: string;
  /** Whether the backend had already applied that key (at-least-once). */
  readonly deduped?: boolean;
}

/** What one dispatched `map` instance did (grammar 8.6, PRD 5.6). */
export interface DispatchRecord {
  /** The source-item index — what orders every write this instance made. */
  readonly index: number;
  /**
   * The route's variant tag, `"$default"` for the catch-all, absent on the
   * homogeneous form.
   *
   * The catch-all's name carries a sigil the identifier grammar cannot produce
   * (§2.1: `lower , { lower | digit | "_" }`) because a union may declare a
   * variant tagged `default` *and* a `default:` catch-all beside it — legal
   * together, and resolved correctly by [`selectRoute`], which searches the
   * named routes first. Spelling both `"default"` here would make the one
   * distinction a reader has between them the target, and two routes may share
   * a target.
   */
  readonly route?: string;
  /** The component the item was dispatched to. */
  readonly target: string;
  /**
   * What became of the item, which is what `on_item_error` decided (grammar 8.6
   * rule 10): `skipped` is the policy dropping it and the fan-out carrying on,
   * `failed` is `fail` — or a `retry:` out of attempts, which "resolves as
   * `fail` does" — taking the map node down with it.
   *
   * `detached` is *resolved at dispatch*: the join counted it the moment the
   * dispatch was issued and never learned its outcome (Decision D94), so no
   * strategy ever applied to it.
   */
  readonly outcome: "completed" | "skipped" | "failed" | "detached";
  /**
   * How many attempts the item's policy **made** — not how many it allowed, so
   * a node deadline that ended a backoff early is reported as the attempts that
   * really happened. `0` for a detached dispatch, which has no observed outcome
   * for `on_item_error` to have acted on.
   */
  readonly attempts: number;
  /**
   * The key this dispatch's effect site derives (grammar 9.4).
   *
   * A **detached** delivery carries it to its sink on the surface grammar 9.4
   * fixes per binding kind — the `Idempotency-Key` header of an `http:`
   * binding, the `IDEMPOTENCY_KEY` variable in an `exec:` binding's
   * environment, the `idempotency_key` field of the context a `function:`
   * binding is invoked with — which is what makes at-least-once delivery a
   * defensible trade rather than a lost message (PRD 5.6). It is recorded for
   * every dispatch because it is the same derivation either way, and because it
   * is the one place the flattened instance path of a nested fan-out is
   * observable at all.
   */
  readonly idempotencyKey: string;
  /** The instance's own trace, when the target was a `flow.*`. */
  readonly inner?: readonly TraceEntry[];
  /** Why the item did not complete, when `on_item_error` skipped it. */
  readonly error?: string;
}

/**
 * Where a node hands its own trace entry to the failure that ends the run.
 *
 * A node that throws returns no `Command`, so the entry it had reached has
 * nowhere to be written: LangGraph discards the whole superstep. Carrying it on
 * the error is what gets it to [`runFlow`], which appends it to the trace it
 * recovered. `Symbol.for` rather than a field name so that nothing a
 * composition can spell collides with it, and non-enumerable so an error that
 * carries one still logs and serializes the way the same error without one does.
 */
const ABORTED = Symbol.for("agent-compose.abortedEntry");

/** Carry `entry` on `error`, and answer with the error unchanged otherwise. */
export function carryEntry<E>(error: E, entry: TraceEntry): E {
  if (typeof error === "object" && error !== null && Object.isExtensible(error)) {
    Object.defineProperty(error, ABORTED, { value: entry, enumerable: false, configurable: true });
  }
  return error;
}

/**
 * What a failed `map` node dispatched, recovered from the failure it raised.
 *
 * The `cause` chain is followed for the same reason [`abortedEntry`] follows it:
 * `runActivity` wraps whatever the activity threw in a [`NodeFailure`], so the
 * [`ItemFailure`] carrying the records is never the outermost error by the time
 * a node's `on_error:` is deciding what to do with it.
 */
function dispatchesOf(error: unknown): readonly DispatchRecord[] | undefined {
  for (let held: unknown = error; typeof held === "object" && held !== null; ) {
    if (held instanceof ItemFailure) return held.dispatches;
    held = (held as { cause?: unknown }).cause;
  }
  return undefined;
}

/**
 * The trace of the subflow instance this error came out of, if it came out of
 * one (grammar 8.5, PRD 5.3).
 *
 * The one account of what happened inside a module boundary, and the caller's
 * own failure says nothing about it — so it is recovered wherever a failed
 * instance is recorded: a `flow:` node's trace entry ([`runNode`]) and a
 * dispatched instance's [`DispatchRecord`] ([`runMap`]). The `cause` chain is
 * followed for the reason [`dispatchesOf`] follows it: `runActivity` wraps
 * whatever the activity threw, so the [`SubflowFailure`] is rarely the outermost
 * error by the time anyone asks.
 */
function traceOf(error: unknown): readonly TraceEntry[] | undefined {
  for (let held: unknown = error; typeof held === "object" && held !== null; ) {
    if (held instanceof SubflowFailure) return held.trace;
    held = (held as { cause?: unknown }).cause;
  }
  return undefined;
}

/**
 * What a `map` node dispatched, read off the **plan** rather than off an answer
 * or a failure (grammar 8.6, PRD 5.3, 5.6).
 *
 * The last resort of the three, and the only one there is when a node's own
 * `timeout:` fired: the deadline is raced ([`runActivity`]), so [`runMap`]'s
 * promise is abandoned where it stands and neither its answer nor an
 * [`ItemFailure`] ever arrives. The plan is [`runNode`]'s own object — it built
 * it, outside the policy — and the map writes each dispatch into it as that
 * dispatch resolves, so what survives here is every detached delivery (resolved
 * at dispatch, D94) and every joined instance that had settled. An instance
 * still in flight when the budget ran out has no outcome yet and so no record;
 * the entry's own `error`, which names the node and its budget, is what accounts
 * for it.
 *
 * Shaped rather than typed, because `runNode` holds a node's input as `unknown`:
 * a map node's is a [`MapPlan`] and nothing else's is.
 */
function plannedDispatches(input: unknown): readonly DispatchRecord[] | undefined {
  if (typeof input !== "object" || input === null) return undefined;
  const plan = input as Partial<MapPlan>;
  if (!Array.isArray(plan.instances) || !Array.isArray(plan.records)) return undefined;
  if (plan.records.length === 0) return undefined;
  return [...plan.records].sort((left, right) => left.index - right.index);
}

/**
 * The entry of the node this error aborted, if it came from one.
 *
 * The `cause` chain is followed, because an error can be restated on the way out
 * — `SuperstepCeiling` puts the original underneath itself — and the entry
 * belongs to whichever error in that chain a node actually raised.
 */
export function abortedEntry(error: unknown): TraceEntry | undefined {
  for (let held: unknown = error; typeof held === "object" && held !== null; ) {
    const entry = (held as Record<symbol, unknown>)[ABORTED];
    if (entry !== undefined) return entry as TraceEntry;
    held = (held as { cause?: unknown }).cause;
  }
  return undefined;
}

/**
 * A run that produced no answer, and the trace it made before it stopped.
 *
 * PRD 5.3 asks that routing decisions appear in traces as data, and a failed run
 * is where a trace is wanted most: it is the record of which guards answered
 * what, which budgets were spent and how many attempts each node made on the way
 * to whatever went wrong. The graph's state — `$run` included — does not survive
 * an `invoke` that throws, so [`runFlow`] streams the run instead, keeps the last
 * state each superstep produced, and raises this in place of the error the graph
 * threw. The original is the `cause`, so a report that prints the chain says
 * exactly what it said before.
 *
 * `what` is which of a run's two ways of failing this is, because both of them
 * have a trace to carry: a run that never reached quiescence, and a run that
 * reached it and could not materialize an output (grammar 7.6.3, 10.1). One
 * error type for both is what lets every caller — `agent-compose run`, the
 * generated `serve` app, an ejected project — read `.trace` without asking which
 * kind of failure it has.
 */
export class FlowFailure extends Error {
  /** The flow that was running. */
  readonly flow: string;
  /** What landed, plus the entry of the node the run aborted at. */
  readonly trace: readonly TraceEntry[];

  constructor(flow: string, what: string, trace: readonly TraceEntry[], cause: unknown) {
    super(`\`${flow}\` ${what}: ${describe(cause)}`);
    this.name = "FlowFailure";
    this.flow = flow;
    this.trace = trace;
    this.cause = cause;
  }
}

/**
 * A run stopped by the superstep ceiling rather than by anything it declared.
 *
 * The ceiling is the compiler's safety net (see `CompiledFlow.recursionLimit`),
 * and LangGraph announces reaching it in its own terms — a `recursionLimit`
 * config key and a link to its troubleshooting page, neither of which is a thing
 * this composition has. What a reader needs instead is that the net caught a
 * loop the composition did not bound itself: grammar 7.4 clause 2 admits a cycle
 * bounded only by a CEL exit condition, PRD 5.4 accepts it as a bound that is
 * *not* a static termination proof, and a guard that never goes false therefore
 * loops until something outside the composition stops it.
 */
export class SuperstepCeiling extends Error {
  /** The ceiling the run was given. */
  readonly ceiling: number;

  constructor(ceiling: number, cause: unknown) {
    super(
      `the run reached its ceiling of ${ceiling} supersteps. The ceiling is the compiler's ` +
        "safety net rather than one of the composition's own bounds: it is sized from the " +
        "`max_iterations` budgets the flow declares, plus an allowance for every cycle " +
        "bounded only by a CEL exit condition (grammar 7.4 clause 2) — which declares no " +
        "number of passes at all, so a guard that never goes false loops until the net " +
        "catches it. Bound the loop where it is, with `max_iterations:` on a `when:`-guarded " +
        "edge inside it, or raise the net for one run with " +
        "`runFlow(flow, inputs, { recursionLimit })`",
    );
    this.name = "SuperstepCeiling";
    this.ceiling = ceiling;
    this.cause = cause;
  }
}

/**
 * LangGraph's recursion error, restated in the composition's own terms, and the
 * error unchanged otherwise.
 *
 * The pinned import is the detection: a release that renamed the class fails
 * `tsc` on this module rather than silently passing its message through.
 */
export function restateCeiling(ceiling: number, error: unknown): unknown {
  return error instanceof GraphRecursionError ? new SuperstepCeiling(ceiling, error) : error;
}

/**
 * The trace a failed run carries: every entry that landed, then the aborting
 * node's own.
 *
 * `state` is the last one a superstep produced, which is the last one whose
 * writes were committed — so its `$run` holds exactly the entries of the steps
 * that completed. The aborting node's entry is appended rather than merged
 * because it never reached the reducer, and it belongs last for the same reason:
 * it is the step the recovered ones stopped short of.
 */
export function failedTrace(state: unknown, error: unknown): readonly TraceEntry[] {
  const held = (state as { $run?: RunChannel } | undefined)?.$run;
  const landed = held?.trace ?? [];
  const aborted = abortedEntry(error);
  return aborted === undefined ? landed : [...landed, aborted];
}

/**
 * The compiler's own state channel.
 *
 * A composition's `state:` channels are the author's (grammar 10); this is what
 * the *runtime* needs beside them and cannot put anywhere else: the flow
 * instance's input object and execution identity, which every CEL surface reads
 * as `input` and `execution`; the per-bounded-edge counters grammar 7.4 requires
 * in graph state; the per-node traversal ordinals of grammar 9.4; the step
 * number of grammar 7.6; and the routing trace PRD 5.3 asks for.
 *
 * It is named `$run` because grammar 2.1's identifier cannot spell it, so no
 * composition can collide with it.
 */
export interface RunChannel {
  readonly step: number;
  readonly input: Readonly<Record<string, unknown>>;
  readonly execution: ExecutionIdentity;
  readonly iterations: Readonly<Record<string, number>>;
  readonly traversals: Readonly<Record<string, number>>;
  /**
   * The result of every node a `map.over` in this flow reads (grammar 8.6
   * rule 11, 4.2).
   *
   * `over: plan.output.tasks` is the one place a node's result is read from
   * *another* node's task — Decision D42 keeps every other surface off node
   * outputs — and the producer ran in an earlier step, so its answer has to be
   * somewhere. Only the nodes a map actually reads are kept, because a node
   * output is arbitrarily large and this channel is copied into every superstep.
   * A node that was **skipped** writes its key back as `undefined`, which is how
   * grammar 8.6 rule 11's "dispatches zero instances" tells a producer that did
   * not run this pass from a stale value it left on the last one.
   */
  readonly outputs: Readonly<Record<string, unknown>>;
  /**
   * The frames of every node crossed from the **root** flow instance down to
   * this one, outermost first (grammar 9.4).
   *
   * Empty in the instance an invocation started; a `flow:` node appends
   * `<node>/<traversal>` and a `map` dispatch `<node>/<traversal>/<index>`. It is
   * what makes the idempotency key of a detached dispatch or a store write name
   * *one* effect site rather than a node id two instances share.
   */
  readonly path: readonly string[];
  /** Level 1 of grammar 9.3, if a `flow:` node instantiated this instance. */
  readonly policy?: InstancePolicy;
  readonly trace: readonly TraceEntry[];
}

/** The empty run channel a flow instance starts at. */
export function emptyRun(): RunChannel {
  return {
    step: 0,
    input: {},
    execution: { id: "", session_key: "" },
    iterations: {},
    traversals: {},
    outputs: {},
    path: [],
    trace: [],
  };
}

/**
 * Fold one node's contribution into the run channel.
 *
 * Concurrent nodes in one step each supply a partial update, and every field
 * merges in a way that does not depend on which of them the reducer sees first:
 * `step` takes the larger, the three key-wise maps merge key-wise (a node only
 * ever writes its own keys), and the trace is concatenated and then ordered by
 * `(step, node)` — the canonical write order of grammar 7.6.4, read for the one
 * channel whose order is otherwise the scheduler's.
 *
 * `path` and `policy` are the instance's own identity rather than anything a
 * node contributes: they are set once, in the state a flow instance is started
 * from, and a partial update that carries neither leaves them alone.
 */
export function mergeRun(left: RunChannel, right: Partial<RunChannel>): RunChannel {
  const trace = [...left.trace, ...(right.trace ?? [])];
  trace.sort((a, b) => a.step - b.step || (a.node < b.node ? -1 : a.node > b.node ? 1 : 0));
  const policy = right.policy ?? left.policy;
  return {
    step: Math.max(left.step, right.step ?? 0),
    input: right.input ?? left.input,
    execution: right.execution ?? left.execution,
    iterations: { ...left.iterations, ...(right.iterations ?? {}) },
    traversals: { ...left.traversals, ...(right.traversals ?? {}) },
    // Spread rather than `??`, so a skipped producer's `{ [node]: undefined }`
    // *replaces* the value it left on an earlier pass instead of being read as
    // "nothing to say" (grammar 8.6 rule 11).
    outputs: { ...left.outputs, ...(right.outputs ?? {}) },
    path: right.path ?? left.path,
    ...(policy === undefined ? {} : { policy }),
    trace,
  };
}

// ---------------------------------------------------------------------------
// Reading and writing a composition's own channels (grammar 8.0, 10)
// ---------------------------------------------------------------------------

/** How a write reaches its channel, which decides what one write supplies. */
export type Reduce = "set" | "append" | "merge";

/** One field of a node's output, and where it goes (grammar 8.0's write map). */
export interface WriteDescriptor {
  readonly field: string;
  readonly channel: string;
  readonly reduce: Reduce;
}

/**
 * The writes a **`map`** node makes to one channel, already in the canonical
 * order of grammar 7.6.4 clause 2 — source-item index, never completion order.
 *
 * A step's writers reach a channel one at a time and LangGraph's reducers are
 * called once per writer, so a map's *N* instances cannot each be a writer: a
 * fan-out of three appends would have to be three calls, and the order of those
 * calls is the scheduler's rather than the source array's. PRD 5.6 calls
 * unordered reduces a silent break of replay, so what the map node writes is one
 * value carrying all *N* contributions in index order — the "index-tagged and
 * reordered before the join" of PRD 5.6, arrived at as one write rather than as
 * a sorting reducer. Every reducer this compiler emits unpacks it
 * ([`appendReduce`], [`mergeReduce`], [`setReduce`]), so the map occupies its
 * own place in clause 1's node-id ordering while its instances are ordered
 * inside it by clause 2.
 *
 * It is a **class** rather than a tagged object because a composition's data is
 * JSON: no `default:`, no model answer, and no `exec` result can be an instance
 * of one, so `instanceof` cannot be spoofed by a value that merely looks like a
 * batch.
 */
export class OrderedWrites {
  /** The contributions, in ascending source-item index. */
  readonly values: readonly unknown[];

  constructor(values: readonly unknown[]) {
    this.values = values;
  }
}

/** What a channel's reducer may be handed: one write, or a `map`'s batch. */
export type Written<T> = T | OrderedWrites;

/** Whether this update is a `map` node's ordered batch rather than one write. */
export function isOrdered(value: unknown): value is OrderedWrites {
  return value instanceof OrderedWrites;
}

/**
 * What a `map`'s contributions to one channel go to LangGraph **as**: a batch
 * for the policies that unpack one, and the folded value for the one that
 * cannot be handed a batch at all.
 *
 * A channel that starts **unset** never calls its reducer for the first write.
 * LangGraph's `BinaryOperatorAggregate.update` keeps the first value verbatim
 * while it holds nothing — `if (this.value === void 0) { this.value = first;
 * newValues = newValues.slice(1); }` — so a batch arriving there would *become*
 * the channel's value: an `OrderedWrites` object where a declared type says
 * `string`, read back by every `state.<channel>` expression, every downstream
 * write, and the flow's own `outputs:`.
 *
 * Exactly one policy can be in that position. `append` and `merge` start at the
 * identity element their policy names (`[]`, `{}` — see `codegen::state`), so
 * their channel always holds a value and their reducer is always called;
 * `reduce: last_wins` with no `default:` starts unset, because grammar 10.1 and
 * Decision D78 say a channel with no declared default does and reading one
 * before its first write is an execution failure rather than a silent default.
 * So the batch is folded here for `set`, and the fold is exactly what
 * [`setReduce`] would have returned — the highest-indexed item's write
 * (grammar 8.6 rule 5, 10.2). The value a run ends with is the same under
 * either shaping; what changes is that it reaches the channel as the value.
 *
 * The half of that argument this function cannot see is the channel table, which
 * `codegen::state` emits: the identity elements are what make "always called"
 * true for `append` and `merge`, and a policy that stopped declaring one would
 * turn an unfolded batch into a channel's value with nothing here to notice.
 * `every_policy_a_map_batch_reaches_unfolded_starts_at_a_value`
 * (`codegen::state`) is the pairing asserted where the initial value is decided.
 */
export function orderedUpdate(batch: ChannelWrite): unknown {
  if (batch.reduce !== "set") return new OrderedWrites(batch.values);
  return batch.values[batch.values.length - 1];
}

/** `reduce: append` — one element per write, in canonical order (D58). */
export function appendReduce<T>(left: readonly T[], right: Written<T>): T[] {
  return isOrdered(right) ? [...left, ...(right.values as readonly T[])] : [...left, right];
}

/** `reduce: merge` — shallow key-wise, so the last writer in order wins a key. */
export function mergeReduce<T>(left: T, right: Written<T>): T {
  if (!isOrdered(right)) return { ...left, ...right };
  let value = left;
  for (const one of right.values) value = { ...value, ...(one as T) };
  return value;
}

/** `reduce: last_wins`, and a defaulted unreduced channel: the last write wins. */
export function setReduce<T>(left: T, right: Written<T>): T {
  if (!isOrdered(right)) return right;
  // A map contributes a channel entry only when it has at least one write for
  // it, so the batch is never empty — and `left` is what an empty one would
  // have to leave behind anyway.
  return right.values.length === 0 ? left : (right.values[right.values.length - 1] as T);
}

/** Read a state channel by name, failing the way Decision D78 says (grammar 10.1). */
export function channelValue(
  state: Readonly<Record<string, unknown>>,
  channel: string,
  reader: string,
): unknown {
  const value = state[channel];
  if (value === undefined) {
    throw new Error(
      `the state channel \`${channel}\` is unset and ${reader} reads it (grammar 10.1, Decision D78)`,
    );
  }
  return value;
}

/** Read one field of the enclosing flow's input object (grammar 8.0 step 3). */
export function flowInput(run: RunChannel, field: string, reader: string): unknown {
  const value = run.input[field];
  if (value === undefined) {
    throw new Error(`the flow input \`${field}\` is absent and ${reader} reads it`);
  }
  return value;
}

/** The value a channel holds once this write has landed, for a guard to read. */
export function applyWrite(previous: unknown, value: unknown, reduce: Reduce): unknown {
  if (reduce === "append") return appendReduce((previous as unknown[]) ?? [], value);
  if (reduce === "merge") {
    return mergeReduce((previous as Record<string, unknown>) ?? {}, value as Record<string, unknown>);
  }
  return setReduce(previous, value);
}

// ---------------------------------------------------------------------------
// Subgraph instantiation (grammar 7.5, 8.5) and fan-out (grammar 8.6)
// ---------------------------------------------------------------------------

/**
 * Anything that can be run to quiescence: a compiled flow, or one of them
 * reached as a subflow.
 *
 * `signal` is LangGraph's own cancellation key (`RunnableConfig.signal`, which
 * the Pregel runner races against each superstep), and it is here because a
 * subgraph is the one activity this runtime can really *stop*: an aborted run
 * stops scheduling tasks instead of carrying on to quiescence. Who supplies it
 * is [`Instantiation.signal`].
 */
export interface Quiescible<S> {
  stream(
    initial: Record<string, unknown>,
    options: { recursionLimit: number; signal?: AbortSignal },
  ): Promise<AsyncIterable<S>>;
}

/**
 * Run a compiled graph until it quiesces, keeping the last state each superstep
 * produced.
 *
 * A stream rather than an invocation because of what a **failure** must leave
 * behind: LangGraph's `invoke` is this loop with the last value kept, and an
 * error thrown out of it discards the state it was keeping — the routing trace
 * of PRD 5.3 with it. Taking the supersteps one at a time keeps every one that
 * did complete, which is what a failure is then reported with. Both callers need
 * that, which is why it is here rather than written twice.
 *
 * `signal` is the caller's deadline, when the caller has one: a `flow:` node and
 * a joined `map` dispatch each run their instance inside a node budget, and a
 * top-level run has no clock above it to pass. See [`Instantiation.signal`].
 */
export async function quiesce<S extends { $run: RunChannel }>(
  graph: Quiescible<S>,
  initial: Record<string, unknown>,
  recursionLimit: number,
  signal?: AbortSignal,
): Promise<{ state?: S; error?: unknown }> {
  let state: S | undefined;
  try {
    const supersteps = await graph.stream(initial, { recursionLimit, signal });
    for await (const superstep of supersteps) {
      // An interrupt is announced as a chunk of its own rather than as a state,
      // and reading it as one would lose the run's.
      if (!isInterrupted(superstep)) state = superstep;
    }
  } catch (error) {
    return { state, error: restateCeiling(recursionLimit, error) };
  }
  return { state };
}

/**
 * The instance path of the flow instance a `flow:` node is about to start
 * (grammar 9.4).
 *
 * The frame is `<node id>/<traversal ordinal>`, where the ordinal is how many
 * times this node has already begun executing in *its own* instance — so the
 * second traversal of a bounded cycle instantiates a different effect site than
 * the first, and a node-level `retry:` re-running the same attempt does not.
 */
export function instancePath(view: NodeView, node: string): readonly string[] {
  return [...view.run.path, `${node}/${view.run.traversals[node] ?? 0}`];
}

/**
 * Grammar 9.3 level 1 for the instance a `flow:` node starts: what already
 * reached this instance, then what this site declares (Decision D79).
 *
 * **Outermost wins, per field.** A caller's hardening of a module it does not
 * own cannot be undone by that module's own instantiation of a deeper one, so an
 * inherited value is never replaced — only an absent one is filled in.
 */
export function instancePolicy(
  inherited: InstancePolicy | undefined,
  declared: InstancePolicy | undefined,
): InstancePolicy | undefined {
  if (inherited === undefined) return declared;
  if (declared === undefined) return inherited;
  const retry = inherited.retry ?? declared.retry;
  const timeoutMs = inherited.timeoutMs ?? declared.timeoutMs;
  const onError = inherited.onError ?? declared.onError;
  return {
    ...(retry === undefined ? {} : { retry }),
    ...(timeoutMs === undefined ? {} : { timeoutMs }),
    ...(onError === undefined ? {} : { onError }),
  };
}

/** One compiled flow, as a `flow:` node or a `map` dispatch reaches it. */
export interface SubflowBinding extends Quiescible<GraphStateLike> {
  /** Its typed address (grammar 2.2). */
  readonly address: string;
  /** The fields its `outputs:` declares, each read from the channel of that name. */
  readonly outputs: readonly string[];
  /** The superstep ceiling one instance of it takes. */
  readonly recursionLimit: number;
}

/** What one instantiation supplies to the instance it starts (grammar 7.5). */
export interface Instantiation {
  /**
   * The subflow's `inputs:`, bound by the instantiating node.
   *
   * **Total, and the only thing that crosses**: a subgraph receives parent state
   * exclusively through these (PRD 5.7, Decision D68). The instance's channels
   * start at their own `default:`s, which is what makes a flow's behaviour
   * independent of the caller that ran it (grammar 10.1).
   */
  readonly inputs: Record<string, unknown>;
  /** The run identity the instance carries (grammar 4.1). */
  readonly execution: ExecutionIdentity;
  /** Its instance path, for anything nested inside it (grammar 9.4). */
  readonly path: readonly string[];
  /**
   * The instantiating node's deadline, which crosses the boundary with it
   * (grammar 9.2).
   *
   * A subgraph is the one activity a `timeout:` can really **stop**. Every other
   * kind is either cooperative (`fetch` takes a signal, `spawn` is killed) or
   * uncancellable (a host function), so [`runActivity`] races the budget and the
   * node fails on time whatever the activity does — but an instance nothing
   * aborted would run on to quiescence *after* the node that started it had
   * already failed: issuing every effect its remaining nodes were going to
   * issue, and holding the [`Admission`] permit a later execution of the same
   * `map` node has to wait for. LangGraph stops scheduling supersteps when this
   * aborts, so what is left in flight inside is one abandoned activity rather
   * than the whole rest of the instance.
   *
   * A **detached** dispatch supplies it too, and it is the delivery's own signal
   * — the one nothing aborts (Decision D94, and see [`runMap`]) — so an instance
   * reached that way runs to quiescence exactly as its sink does.
   */
  readonly signal?: AbortSignal;
  /** Level 1 of grammar 9.3 for every node inside it. */
  readonly policy?: InstancePolicy;
  /**
   * The caller's conversation history, on a `context: inherit` instantiation
   * (grammar 8.5, 10.4).
   *
   * Absent is `isolated`, which is the default and what a `map` dispatch always
   * gets: the instance starts on a fresh history, and what it says there is
   * discarded when it finishes (Decision D105).
   */
  readonly history?: readonly unknown[];
}

/**
 * Instantiate one subflow and materialize its `outputs:` at quiescence
 * (grammar 7.5, 7.6.3, 8.5).
 *
 * The instance is a **separate run of a separate compiled graph**, which is what
 * gives it its own channel values: grammar 10.1 makes the channel set
 * composition-global in shape and per-instance in value, and a subgraph added to
 * the caller's graph would share the caller's values instead. Only two things
 * cross the boundary, in each direction: the `inputs:` this instantiation bound,
 * and the `outputs:` read back out of the instance's own channels.
 *
 * What crosses beside them is not data but a **clock**: the instantiating node's
 * deadline ([`Instantiation.signal`]), so a `timeout:` on a `flow:` node — or on
 * the `map` node a joined dispatch belongs to — ends the instance rather than
 * only the node's wait for it.
 */
export async function runSubflow(
  binding: SubflowBinding,
  instance: Instantiation,
): Promise<NodeAnswer> {
  const seeded = instance.history ?? [];
  const initial: Record<string, unknown> = {
    $run: {
      ...emptyRun(),
      input: instance.inputs,
      execution: instance.execution,
      path: instance.path,
      ...(instance.policy === undefined ? {} : { policy: instance.policy }),
    },
    ...(instance.history === undefined ? {} : { messages: [...seeded] }),
  };

  const { state, error } = await quiesce(
    binding,
    initial,
    binding.recursionLimit,
    instance.signal,
  );
  const trace = state?.$run.trace ?? [];
  if (error !== undefined) {
    throw new SubflowFailure(binding.address, "did not run to quiescence", trace, error);
  }
  if (state === undefined) {
    // Unreachable: `streamMode: "values"` emits the state the instance started
    // from before any node has run.
    throw new SubflowFailure(
      binding.address,
      "produced no state",
      trace,
      new Error("the instance emitted no superstep"),
    );
  }

  const held = state as unknown as Record<string, unknown>;
  const output: Record<string, unknown> = {};
  try {
    for (const field of binding.outputs) {
      output[field] = channelValue(
        held,
        field,
        `\`${binding.address}\`'s output field \`${field}\``,
      );
    }
  } catch (cause) {
    throw new SubflowFailure(binding.address, "reached quiescence without an output", trace, cause);
  }

  // On `context: inherit`, what the instance *added* is what goes back to the
  // caller's channel: the seeded turns are already there, and appending them
  // again would double the conversation (grammar 8.5, 10.4).
  const grown = held["messages"];
  const added =
    instance.history === undefined || !Array.isArray(grown) ? [] : grown.slice(seeded.length);
  return {
    output,
    ...(added.length === 0 ? {} : { history: added }),
    inner: trace,
  };
}

/**
 * A counting semaphore: `max_concurrency`, enforced (grammar 8.6 rule 1, D28).
 *
 * LangGraph's own `maxConcurrency` is a **run-level** config key — the Pregel
 * runner reads it once per superstep and applies it to every task in that step —
 * so it cannot say "this map at 8, its `auto_fixable` route at 4, and the
 * `announce` node running beside them unbounded", which is exactly what D28
 * declares. The bound is normative, so it is counted here.
 *
 * It is an **admission** bound: a permit is taken before a dispatch starts and
 * held until it settles, so what the number bounds is how many of this map's
 * instances are in flight at once — every one of them, detached included
 * (grammar 8.6's key table). Which gate a dispatch takes its permit from is
 * [`Admission`]'s: the bound is over a node, not over a call.
 *
 * **A joined waiter is served before a detached one**, which is grammar 8.6 rule
 * 7's "nothing it does can *delay* the enclosing flow instance" enforced at the
 * one place a detached dispatch could: the queue. A joined instance waiting
 * behind a queued delivery waits out that delivery's whole duration, and the
 * queue spans executions — [`Admission`] belongs to the node, so a bounded
 * cycle's second traversal, or the node's own `retry:`, would otherwise line its
 * instances up behind deliveries the previous traversal left in flight. Ordering
 * the queue costs the delivery nothing: it still starts under a permit (the key
 * table), just never ahead of work the join is waiting for. [`runMap`] keeps the
 * other half — a delivery is not *issued* until this call's joined instances are
 * admitted, because a permit already taken cannot be reordered.
 *
 * `permits > 0` implies both queues are empty, because [`release`] hands a
 * permit straight to a waiter rather than returning it to the count — so the
 * fast path cannot jump a detached dispatch over a waiting joined one.
 */
class Gate {
  private permits: number;
  /** Joined waiters: the dispatches the map node's join is waiting on. */
  private readonly waiting: (() => void)[] = [];
  /** Detached waiters, served only once no joined dispatch wants a permit. */
  private readonly deferred: (() => void)[] = [];

  constructor(permits: number) {
    this.permits = Math.max(1, permits);
  }

  async acquire(kind: DispatchKind = "joined"): Promise<void> {
    if (this.permits > 0) {
      this.permits -= 1;
      return;
    }
    await new Promise<void>((resolve) => {
      (kind === "detached" ? this.deferred : this.waiting).push(resolve);
    });
  }

  release(): void {
    const next = this.waiting.shift() ?? this.deferred.shift();
    if (next === undefined) {
      this.permits += 1;
    } else {
      next();
    }
  }
}

/** Which half of grammar 8.6 rule 7 a dispatch queues under (see [`Gate`]). */
type DispatchKind = "joined" | "detached";

/**
 * The permits one `map` **node** admits its dispatches against (grammar 8.6's
 * key table, Decision D28).
 *
 * A gate built inside [`runMap`] would bound one *call*, and a dispatch can
 * outlive the call that issued it: `detach: true` is resolved at dispatch (D94)
 * and its delivery is still in flight when the map node returns. So a node-level
 * `retry:` and a second traversal of a bounded cycle each start a fresh call
 * while the previous one's deliveries hold permits nothing counts any more — and
 * a map declaring 1 reaches 2 in flight, which is the number an author wrote it
 * down to prevent. The gates therefore belong to the node, and every execution
 * of it draws on the same ones.
 *
 * **Which node.** The enclosing flow *instance*'s, not the flow's: the key is
 * the execution id, the instance path, and the node id ([`MapPlan.admission`]).
 * Two concurrent instances of one dispatched subflow are two separate runs of
 * that flow, each bounding its own map node, and a bound shared between them
 * would serialize siblings an author asked to run side by side — and would
 * deadlock outright on a flow that dispatches itself, where the outer instance
 * holds a permit the inner one is waiting for.
 *
 * **Why it is counted.** The entry is retired when the last dispatch counted
 * against it settles, so the table holds one entry per map node that is *doing
 * something* rather than one per node the process ever ran. A delivery that
 * never settles keeps its node's entry — and its permit — which is the same
 * unbounded wait an abandoned host function is, and is bounded for the run by
 * the node's own `timeout:` rather than by anything here.
 *
 * Spanning executions is also the one way a detached delivery can still be in
 * front of a joined instance, so it is the reason [`Gate`] orders its queue: the
 * call that *issued* the delivery cannot queue behind it ([`runMap`] admits its
 * joined instances first), but a second traversal of a bounded cycle knows
 * nothing of it. A delivery this table is still holding a permit for is served
 * after every joined waiter, so the only one that can delay a later execution is
 * one that already *started* — which is the abandoned-host-function wait above,
 * and is what that execution's own `timeout:` is racing.
 */
interface Admission {
  /** `max_concurrency:` — the node-wide bound (grammar 8.6 rule 1). */
  readonly node: Gate;
  /** A route's own bound, keyed by its position in the descriptor. */
  readonly routes: Map<string, Gate>;
  /** Dispatches counted against it: in flight, or still queued for a permit. */
  outstanding: number;
}

/** Every map node with a dispatch outstanding, by [`MapPlan.admission`]. */
const admissions = new Map<string, Admission>();

/** One fresh set of permits, for the bounds this descriptor declares. */
function permitsFor(map: MapDescriptor): Admission {
  return { node: new Gate(map.maxConcurrency), routes: new Map(), outstanding: 0 };
}

/**
 * The permits this dispatch of `map` is admitted against, counting its
 * instances in.
 *
 * A plan of **zero** instances registers nothing: an entry with no dispatch to
 * retire it is an entry that never goes away, and a map that dispatched nothing
 * has nothing to bound.
 */
function admit(map: MapDescriptor, plan: MapPlan): Admission {
  if (plan.instances.length === 0) return permitsFor(map);
  let held = admissions.get(plan.admission);
  if (held === undefined) {
    held = permitsFor(map);
    admissions.set(plan.admission, held);
  }
  held.outstanding += plan.instances.length;
  return held;
}

/** One dispatch has settled; the node's permits go away with the last of them. */
function retire(key: string, admission: Admission): void {
  admission.outstanding -= 1;
  if (admission.outstanding <= 0 && admissions.get(key) === admission) {
    admissions.delete(key);
  }
}

/** `on_item_error:` — per item, and map-wide (grammar 8.6 rule 10, D73). */
export type ItemPolicy = "fail" | "skip" | { readonly retry: RetryPolicy };

/** Where one dispatch sits, for everything nested inside it (grammar 9.4). */
export interface DispatchSite {
  /** The instance's run identity, carrying this dispatch's `item_index`. */
  readonly execution: ExecutionIdentity;
  /** Its instance path (grammar 9.4). */
  readonly path: readonly string[];
  /**
   * The key a **detached** delivery carries, derived from the execution id and
   * this dispatch's flattened instance path (grammar 9.4, PRD 5.6).
   *
   * Derived for every dispatch rather than only the detached ones: it costs a
   * `join` and it is what a `store` write inside a dispatched `flow.*` will need
   * from the same site. What *delivers* it is [`delivering`], at the one kind of
   * dispatch grammar 9.4 names as a carrier.
   */
  readonly idempotencyKey: string;
}

/** One dispatch target of a `map` (grammar 8.6). */
export interface MapRoute {
  /**
   * Its variant tag, `"$default"` for the catch-all, absent on `node:`.
   *
   * Read for the trace and for nothing else: which route an item takes is
   * [`selectRoute`]'s, and the catch-all is a field of its own on
   * [`MapDescriptor`] rather than an entry in `routes`. See
   * [`DispatchRecord.route`] for why the catch-all's spelling is not one an
   * author could have written.
   */
  readonly tag?: string;
  /** The component it dispatches to. */
  readonly target: string;
  /** Its own bound, at most the map's (grammar 8.6 rule 1). */
  readonly maxConcurrency: number;
  /** `detach: true` — resolved at dispatch (grammar 8.6 rule 7, D94). */
  readonly detach: boolean;
  /**
   * The shape the item is read through: **this route's variant payload**, not
   * the union (grammar 8.6 rule 4).
   */
  readonly itemShape: Shape;
  /** Build one instance's input from the item (grammar 8.6 rule 12). */
  input(roots: Roots): unknown;
  /** Run one instance. */
  run(input: unknown, context: RunContext, site: DispatchSite): Promise<NodeAnswer>;
  /** Where its result's fields go — reduced channels only (rule 5). */
  readonly writes: readonly WriteDescriptor[];
}

/** Everything `./graph.ts` says about one `map` node (grammar 8.6). */
export interface MapDescriptor {
  /** The flow-local node id, which every frame and message is written from. */
  readonly node: string;
  /** `as:` — what the item is called in the per-item expressions. */
  readonly as: string;
  /** `over:` — the path expression, and the node whose result it reads. */
  readonly source: {
    readonly path: string;
    readonly producer?: string;
    readonly shape: Shape;
  };
  /** `max_concurrency:` — the node-wide admission bound (D28, and see [`Gate`]). */
  readonly maxConcurrency: number;
  /** `on_item_error:` — map-wide, and read from the `map:` block alone (rule 10). */
  readonly onItemError: ItemPolicy;
  /** `route_by:` — the literal discriminator field, on the routed form (rule 8). */
  readonly routeBy?: string;
  /** The named routes in declaration order, or the single homogeneous target. */
  readonly routes: readonly MapRoute[];
  /** `default:` — the catch-all (D30). */
  readonly fallback?: MapRoute;
}

/** One instance a `map` will dispatch, decided before any of them runs. */
interface PlannedInstance {
  readonly index: number;
  readonly route: MapRoute;
  readonly input: unknown;
  readonly site: DispatchSite;
}

/** What a `map` node's input phase answers: every dispatch, already bound. */
export interface MapPlan {
  readonly instances: readonly PlannedInstance[];
  /**
   * Which map node this is a dispatch of: the execution id, the enclosing
   * instance's path, and the node id.
   *
   * What [`Admission`] is keyed by, and the reason it is derived here rather
   * than in [`runMap`]: the plan is built once per node execution and reused
   * across the node's own `retry:` attempts, and a later traversal of a bounded
   * cycle builds a new plan with the same key — which is exactly the span the
   * bound has to cover.
   */
  readonly admission: string;
  /**
   * What this node's dispatches resolved to, as they resolve.
   *
   * [`runMap`] answers with the whole account when it returns and carries it on
   * an [`ItemFailure`] when an item fails it, and neither reaches [`runNode`]
   * when the node's own `timeout:` fires: the deadline is *raced*, so the map's
   * promise — and everything riding on it — is abandoned where it stands. The
   * plan is the one thing both sides hold, so the records are written here too
   * and `runNode` reads them off a node it never got an answer from.
   *
   * Reset at the start of every call, because a node retry re-executes the whole
   * fan-out and the trace reports the attempt it made, not the sum of them.
   * Every value that escapes is a copy: this array is cleared in place, and a
   * trace entry an earlier traversal already holds must not empty out under it.
   */
  readonly records: DispatchRecord[];
}

/**
 * Decide a `map`'s whole dispatch **before** anything runs: how many instances,
 * which route each takes, and what each is passed (grammar 8.6 rules 4, 11, 12).
 *
 * This is the map's *input phase*, and it is separate from [`runMap`] for the
 * reason `runNode` builds every node's input outside its policy: reading an
 * absent value fails the **execution** (grammar 4.1, Decisions D110, D78), and
 * neither `on_item_error` nor the map node's `on_error:` may absorb that. A
 * per-item binding that reads a channel nothing has written is a broken
 * composition, not a failed item.
 */
export function mapPlan(map: MapDescriptor, view: NodeView): MapPlan {
  const traversal = view.run.traversals[map.node] ?? 0;
  const items = mapSource(map, view);
  const instances: PlannedInstance[] = items.map((item, index) => {
    const route = selectRoute(map, item);
    const execution: ExecutionIdentity = { ...view.run.execution, item_index: index };
    const frame = `${map.node}/${traversal}/${index}`;
    const path = [...view.run.path, frame];
    const site: DispatchSite = {
      execution,
      path,
      idempotencyKey: [view.run.execution.id, ...path].join("/"),
    };
    const roots: Roots = {
      ...view.roots,
      execution: bindRoot(execution, EXECUTION_SHAPE),
      [map.as]: bindRoot(item, route.itemShape),
    };
    return { index, route, input: route.input(roots), site };
  });
  return {
    instances,
    admission: [view.run.execution.id, ...view.run.path, map.node].join("/"),
    records: [],
  };
}

/**
 * The array `over` resolves to (grammar 4.2, 8.6 rule 11).
 *
 * A producer that was **skipped** on this pass has no result, and the map then
 * dispatches zero instances — grammar 8.6 rule 11's last sentence, which rule 6
 * makes a completion rather than a failure.
 */
function mapSource(map: MapDescriptor, view: NodeView): unknown[] {
  let roots = view.roots;
  const producer = map.source.producer;
  if (producer !== undefined) {
    const produced = view.run.outputs[producer];
    if (produced === undefined) return [];
    roots = {
      ...roots,
      [producer]: bindRoot({ output: produced }, { properties: { output: map.source.shape } }),
    };
  }
  const value = toJson(evaluate(map.source.path, roots));
  if (!Array.isArray(value)) {
    // Unreachable over an artifact `build` accepted: `over` is resolved against
    // the declared schemas and must land on an array (grammar 8.6 rule 1).
    throw new Error(
      `\`${map.node}\`'s \`over: ${map.source.path}\` did not resolve to an array (grammar 8.6 rule 1)`,
    );
  }
  return value;
}

/**
 * How one route is addressed inside its node's [`Admission`].
 *
 * By **position** in the descriptor rather than by `tag:`, because the tag is
 * the author's spelling of a variant and the catch-all has one of its own: two
 * routes whose tags collided would share a bound neither declared. The
 * descriptor a compiled `graph.ts` passes is a module constant, so a route's
 * position is the same on every traversal.
 */
function routeKey(map: MapDescriptor, route: MapRoute): string {
  const at = map.routes.indexOf(route);
  return at < 0 ? "*" : String(at);
}

/** Which route one item takes (grammar 8.6 rules 2, 4). */
function selectRoute(map: MapDescriptor, item: unknown): MapRoute {
  if (map.routeBy === undefined) {
    return map.routes[0]!;
  }
  const tag = (item as Record<string, unknown> | null)?.[map.routeBy];
  const named = map.routes.find((route) => route.tag === tag);
  if (named !== undefined) return named;
  if (map.fallback !== undefined) return map.fallback;
  // Unreachable: exhaustiveness is a compile error, and the item's own schema
  // admits no other tag (grammar 8.6 rule 4). An item that got here anyway is
  // one the parse should have refused, so it says so rather than being dropped.
  throw new Error(
    `\`${map.node}\` has no route for \`${map.routeBy}: ${JSON.stringify(tag)}\` and no \`default:\` (grammar 8.6 rule 4)`,
  );
}

/**
 * Dispatch a `map`, join, and answer with the writes its instances made — in
 * source-item order (grammar 8.6 rules 5, 6, 10, PRD 5.6).
 *
 * **The join is the node.** The map node completes when every instance has
 * completed, been resolved by `on_item_error`, or been detached, which is
 * grammar 7.6's fan-out barrier; because the node's own outgoing edges are
 * evaluated in its own task after it completes (P1), the downstream edge fires
 * exactly once, after the join, whatever order the instances finished in. A
 * dispatch of **zero** instances completes immediately and writes nothing, and
 * its edges fire as if every instance had finished (rule 6).
 *
 * **A detached dispatch is admitted, but never ahead of the join.** It takes a
 * node permit before it starts, because `max_concurrency` bounds every in-flight
 * dispatch (grammar 8.6's key table) — a map declaring 2 against a rate-limited
 * provider would otherwise reach 2 + the detached route's bound. What the join
 * never does is wait on its *outcome* (D94): the delivery is counted resolved
 * the moment it is issued, and the map node returns while it is still in flight.
 *
 * Those two are a bound and a queue, and a queue is the one place the second can
 * take the first back. A delivery that held a permit a joined instance was
 * waiting for would *delay* the enclosing flow instance, which rule 7 says
 * outright that a detached dispatch cannot do: at `max_concurrency: 1` a
 * detached item ahead of a joined one made the map node wait out the sink, and a
 * sink that never answered never released the permit at all — the join hung for
 * ever on a delivery it is defined not to wait for. So a detached dispatch
 * queues **behind every joined instance**, in both of the ways it could get
 * ahead of one:
 *
 *   * it is not *issued* until every joined instance of this call has been
 *     admitted. A permit is taken synchronously by whichever dispatch asks
 *     first, and the loop below issues them in source-item order, so a detached
 *     item at index 0 would otherwise hold the permit before the joined item at
 *     index 1 had asked — no queue discipline can reorder a permit already
 *     taken, so it is not taken;
 *   * once issued it waits as a `"detached"` [`Gate`] waiter, which is served
 *     only when no joined dispatch wants a permit. That covers the executions
 *     this call cannot see: [`Admission`] belongs to the node, so a bounded
 *     cycle's second traversal — or the node's own `retry:` — would otherwise
 *     line its joined instances up behind deliveries this one left queued.
 *
 * Both halves of the key table then hold at once: a detached dispatch waits for
 * a permit to *start*, and the join waits for none of it. Nothing is lost to the
 * ordering either — a delivery that is merely *later* is still delivered, which
 * is the distinction rule 7 and PRD 5.6 trade dedupe-on-a-key to keep. What
 * survives is narrower and honest: a delivery that has already *started* and
 * never settles still holds its permit, exactly as an abandoned host function
 * holds a slot, and only a *later* execution of this node can meet it there
 * ([`Admission`]).
 *
 * **A detached delivery is off the node's clock.** It runs under a signal of its
 * own, never `context.signal`. That signal is the map node's `timeout:`
 * (grammar 9.2), and a delivery still queued for a permit when the budget ran
 * out would otherwise *start* against an already-aborted signal — [`runHttp`]
 * hands it to `fetch` and [`runExec`] to `spawn`, so it would throw before
 * anything reached the wire, be swallowed by the dispatch's own catch, and leave
 * a record saying `detached` for a message that was never sent. That is the lost
 * message grammar 8.6 rule 7 and PRD 5.6 trade dedupe-on-a-key to avoid, and the
 * budget has nothing to say about it: a deadline bounds what the node *waits*
 * for, and D94 is the statement that the node waits for none of this.
 *
 * **Order comes from the index, never from completion.** Results are collected
 * per instance and folded into one [`ChannelWrite`] per channel afterwards, in
 * ascending source-item index — so two runs over one array leave every channel
 * holding the same value even when the provider answers them in the opposite
 * order (grammar 7.6.4 clause 2, 10.2).
 */
export async function runMap(
  map: MapDescriptor,
  plan: MapPlan,
  context: RunContext,
): Promise<NodeAnswer> {
  const admission = admit(map, plan);
  const node = admission.node;
  const gateOf = (route: MapRoute): Gate => {
    const key = routeKey(map, route);
    let gate = admission.routes.get(key);
    if (gate === undefined) {
      gate = new Gate(Math.min(route.maxConcurrency, map.maxConcurrency));
      admission.routes.set(key, gate);
    }
    return gate;
  };

  // The plan's own account, which is the copy a node that never returns an
  // answer is read from — so it starts this execution empty. Everything that
  // leaves here is a copy of it, because it is cleared in place.
  const records = plan.records;
  records.length = 0;
  const landed: {
    index: number;
    route: MapRoute;
    output: unknown;
    models?: readonly ModelCall[];
  }[] = [];
  const failed: { index: number; target: string; attempts: number; error: unknown }[] = [];
  const joined: Promise<void>[] = [];

  // What a detached dispatch waits behind before it asks for a permit at all:
  // every joined instance of this call, admitted (grammar 8.6 rule 7, and see
  // *A detached dispatch is admitted, but never ahead of the join* above). The
  // count is taken before anything runs, because the loop below issues the
  // dispatches in source-item order and a detached item at index 0 would
  // otherwise take the permit before the joined item at index 1 exists.
  //
  // A plan of no joined instances releases it immediately: there is nothing for
  // a delivery to get ahead of, and a barrier nothing resolves would strand
  // every delivery a fire-and-forget map exists to make.
  let unadmitted = plan.instances.reduce(
    (count, dispatch) => count + (dispatch.route.detach ? 0 : 1),
    0,
  );
  let admitted = (): void => {};
  const joinedAdmitted: Promise<void> =
    unadmitted === 0
      ? Promise.resolve()
      : new Promise<void>((resolve) => {
          admitted = () => {
            unadmitted -= 1;
            if (unadmitted === 0) resolve();
          };
        });

  for (const instance of plan.instances) {
    const { index, route, site } = instance;
    const named = route.tag === undefined ? {} : { route: route.tag };
    const scoped: RunContext = { ...context, execution: site.execution };

    if (route.detach) {
      // Resolved at dispatch (Decision D94): the record is written now, the
      // delivery is issued now, and the join never learns what became of it.
      records.push({
        index,
        ...named,
        target: route.target,
        outcome: "detached",
        attempts: 0,
        idempotencyKey: site.idempotencyKey,
      });
      const gate = gateOf(route);
      // Its own signal, never the node's: see *A detached delivery is off the
      // node's clock* above. One per delivery rather than one per call, so a
      // fan-out of many sinks does not pile listeners onto a shared signal.
      const delivery: RunContext = { ...scoped, signal: new AbortController().signal };
      void (async () => {
        // `max_concurrency` is an **admission** bound over every in-flight
        // dispatch, detached included (grammar 8.6's key table, D28): a detached
        // delivery waits for a node permit to *start*, exactly as a joined
        // instance does. It waits for it **behind the join**, though — rule 7's
        // "nothing it does can delay the enclosing flow instance" is a statement
        // about the permit queue as much as about the outcome. Both gates are
        // taken after the barrier and as a `"detached"` waiter, so a joined
        // instance of this call never queues behind this delivery and a later
        // execution of this node never queues behind it either.
        await joinedAdmitted;
        await gate.acquire("detached");
        await node.acquire("detached");
        try {
          await route.run(instance.input, delivery, site);
        } finally {
          node.release();
          gate.release();
          retire(plan.admission, admission);
        }
      })().catch(() => {
        // Nothing it does can fail the enclosing flow instance, which is what an
        // author asks for by writing the key (grammar 8.6 rule 7). Swallowing it
        // here is also what keeps an unhandled rejection from ending the process
        // long after the map node completed.
      });
      continue;
    }

    const gate = gateOf(route);
    joined.push(
      (async () => {
        // The route's gate first and the node's second — the same order every
        // dispatch takes them in, detached or not, so there is nothing to
        // deadlock on. A route whose own bound is spent queues on its own gate
        // instead of holding a node permit another route could have used.
        await gate.acquire();
        await node.acquire();
        // Both permits held: this instance can no longer be got in front of, so
        // it is what a detached dispatch was waiting to be behind. Reported here
        // rather than on completion — the barrier orders the *queue*, and making
        // a delivery wait for the join to finish would hold back a message the
        // map node is defined not to wait for (grammar 8.6 rule 7, D94).
        admitted();
        let attempts = 0;
        try {
          const answer = await attemptItem(map, instance, scoped);
          attempts = answer.attempts;
          landed.push({
            index,
            route,
            output: answer.value.output,
            // Which member of a route served each of this instance's model
            // calls is a fact about *this* run (PRD 5.9), and a dispatched
            // instance has no trace entry of its own — so the map node's entry
            // is where it belongs, in source-item order like everything else a
            // fan-out reports.
            ...(answer.value.models === undefined ? {} : { models: answer.value.models }),
          });
          records.push({
            index,
            ...named,
            target: route.target,
            outcome: "completed",
            attempts,
            idempotencyKey: site.idempotencyKey,
            ...(answer.value.inner === undefined ? {} : { inner: answer.value.inner }),
          });
        } catch (error) {
          attempts = error instanceof ItemAttempts ? error.attempts : 1;
          const cause = error instanceof ItemAttempts ? error.cause : error;
          failed.push({ index, target: route.target, attempts, error: cause });
          // A dispatched `flow.*` that failed still made a trace, exactly as one
          // that completed did, and under `on_item_error: skip` the run carries
          // on and this record is the *only* account of what happened inside the
          // boundary — every guard, every budget, every attempt (grammar 8.5,
          // PRD 5.3). The attempt reported is the last one the item made, which
          // is the attempt `attempts` counts and the error `error` describes.
          const held = traceOf(cause);
          records.push({
            index,
            ...named,
            target: route.target,
            // What the item's own policy did with it: `skip` dropped it and the
            // fan-out carried on, while `fail` — and a `retry:` that ran out of
            // attempts, which "resolves as `fail` does" (grammar 8.6 rule 10) —
            // failed it. Reporting a failure as `skipped` would tell a reader
            // the map absorbed an item it did not absorb.
            outcome: map.onItemError === "skip" ? "skipped" : "failed",
            attempts,
            idempotencyKey: site.idempotencyKey,
            ...(held === undefined ? {} : { inner: held }),
            error: describe(cause),
          });
        } finally {
          node.release();
          gate.release();
          retire(plan.admission, admission);
        }
      })(),
    );
  }

  await Promise.all(joined);
  // A copy, and the sort is on the copy: `plan.records` is cleared in place at
  // the start of the next execution of this node, and a trace entry already
  // holding it would empty out under a reader.
  const dispatched = [...records].sort((left, right) => left.index - right.index);

  // Exhausted retries resolve as `fail` does (grammar 8.6 rule 10), and the item
  // reported is the **lowest-indexed** failure rather than the first in time:
  // two runs over one array must fail about the same item.
  if (map.onItemError !== "skip" && failed.length > 0) {
    failed.sort((left, right) => left.index - right.index);
    const first = failed[0]!;
    // The records go out **on the failure**, because this is where the account
    // of a fan-out matters most: the items that did run had their effects, and
    // the node's own `on_error:` is about to absorb the one that did not
    // (grammar 8.6 rule 10's "retry each item, and if one still fails, skip the
    // fan-out"). `runNode` reads them back off the error onto the trace entry it
    // builds, so a skipped or fallen-back map still says what it dispatched.
    throw new ItemFailure(
      map.node,
      first.index,
      first.target,
      first.attempts,
      first.error,
      dispatched,
    );
  }

  const models = [...landed]
    .sort((left, right) => left.index - right.index)
    .flatMap((one) => one.models ?? []);
  return {
    output: {},
    channels: orderedChannels(landed),
    dispatches: dispatched,
    ...(models.length === 0 ? {} : { models }),
  };
}

/** A failed item, carrying how many attempts its policy made. */
class ItemAttempts extends Error {
  readonly attempts: number;

  constructor(attempts: number, cause: unknown) {
    super(describe(cause));
    this.name = "ItemAttempts";
    this.attempts = attempts;
    this.cause = cause;
  }
}

/**
 * One item, under `on_item_error` (grammar 8.6 rule 10).
 *
 * A retry re-executes the **whole dispatched instance** from its entry as a
 * fresh attempt, and nothing the instance path is built from changes — the item
 * keeps its index — so every attempt derives the same idempotency key
 * (grammar 9.4). The node's own deadline is the one clock: `context.signal` is
 * the map node's, so an item's attempts are spent inside the map's `timeout:`
 * rather than beside it.
 */
async function attemptItem(
  map: MapDescriptor,
  instance: PlannedInstance,
  context: RunContext,
): Promise<{ value: NodeAnswer; attempts: number }> {
  const retry = typeof map.onItemError === "object" ? map.onItemError.retry : undefined;
  const allowed = 1 + (retry?.max ?? 0);
  // What the item *did*, not what its policy allowed — the same distinction
  // `runActivity` draws for a node. The map node's deadline can end the loop
  // mid-backoff, and a record saying five attempts were made against a provider
  // the item reached once describes a run that did not happen.
  let made = 0;
  let last: unknown;
  for (let attempt = 1; attempt <= allowed; attempt += 1) {
    made = attempt;
    try {
      return { value: await instance.route.run(instance.input, context, instance.site), attempts: attempt };
    } catch (error) {
      last = error;
      if (attempt === allowed) break;
      try {
        await sleep(backoffFor(retry!, attempt), context.signal);
      } catch {
        break;
      }
    }
  }
  throw new ItemAttempts(made, last);
}

/**
 * The instances' results, folded into one write per channel in **source-item**
 * order (grammar 7.6.4 clause 2).
 *
 * The channels themselves are ordered by name so that the update object a map
 * hands LangGraph has the same key order on every run — the last thing between
 * this and a replay that reproduces the live run exactly.
 */
function orderedChannels(
  landed: readonly { index: number; route: MapRoute; output: unknown }[],
): ChannelWrite[] {
  const held = new Map<string, { reduce: Reduce; values: unknown[] }>();
  for (const one of [...landed].sort((left, right) => left.index - right.index)) {
    const fields = (one.output ?? {}) as Record<string, unknown>;
    for (const write of one.route.writes) {
      const value = fields[write.field];
      // A field the result does not carry performs no write (Decision D110).
      if (value === undefined) continue;
      let channel = held.get(write.channel);
      if (channel === undefined) {
        channel = { reduce: write.reduce, values: [] };
        held.set(write.channel, channel);
      }
      channel.values.push(value);
    }
  }
  return [...held.entries()]
    .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0))
    .map(([channel, { reduce, values }]) => ({ channel, reduce, values }));
}

// ---------------------------------------------------------------------------
// One node execution, end to end
// ---------------------------------------------------------------------------

/** The graph state a node reads: the composition's channels, plus `$run`. */
export type GraphStateLike = Readonly<Record<string, unknown>> & { readonly $run: RunChannel };

/** What a node sees before it runs. */
export interface NodeView {
  /** Every channel, as of the start of this step. */
  readonly state: Readonly<Record<string, unknown>>;
  /** The compiler's own channel. */
  readonly run: RunChannel;
  /**
   * The roots a node's *configuration* reads: `input`, `state` and `execution`
   * (grammar 4.1), already bound through their declared shapes.
   *
   * Carried on the view because two constructs need them after the input
   * builder has run: a `map`'s per-item bindings extend them with the item and
   * with the dispatch's own `execution.item_index`, and `over` extends them with
   * the producing node's result.
   */
  readonly roots: Roots;
}

/** Everything `./graph.ts` says about one node (grammar 7.1, 8, 9). */
export interface NodeDescriptor {
  readonly flow: string;
  readonly node: string;
  readonly policy: NodePolicy;
  /** The shapes the roots of this node's expressions are read through. */
  readonly shapes: {
    readonly input: Shape;
    readonly state: Shape;
    readonly output: Shape;
  };
  /**
   * Build the input the activity is given (grammar 8.0).
   *
   * `roots` carries `input`, `state` and `execution` — the three a node's
   * configuration may read (Decision D42: node outputs are readable only from
   * edge guards and `map.over`) — and `view` is what a name-based read resolves
   * against, where no expression stands between the two declarations.
   */
  input(roots: Roots, view: NodeView): unknown;
  /**
   * Run the activity and parse its answer with the emitted schema.
   *
   * The parse is inside, so a nonconforming answer is a node error the node's
   * own `retry:` policy can ask again about (PRD 5.2: the answer is rejected
   * before any edge is evaluated).
   */
  run(input: unknown, context: RunContext, view: NodeView): Promise<NodeAnswer>;
  readonly writes: readonly WriteDescriptor[];
  readonly edges: readonly EdgeDescriptor[];
  /**
   * Whether a `map.over` in this flow reads this node's result, so the result is
   * kept in `$run.outputs` for the map's own step to read (grammar 8.6 rule 11).
   */
  readonly retains?: boolean;
  /**
   * Whether this node takes no `timeout` and no `retry` from grammar 9.3 level 1
   * — a `human` node, and only a `human` node (Decision D102).
   */
  readonly exempt?: boolean;
}

/** What one node's activity produced. */
export interface NodeAnswer {
  /** Its result object, which `writes` maps into channels (grammar 8.0). */
  readonly output: unknown;
  /**
   * What it contributes to the shared conversation history (grammar 10.4):
   * the exchange an `agent:` node composed, or the turns a `context: inherit`
   * subflow instance added to the caller's channel.
   */
  readonly history?: readonly unknown[];
  /**
   * Channel writes the activity made **itself**, already in canonical order — a
   * `map` node's, whose writers are its instances rather than its own output
   * (grammar 7.6.4 clause 2, 8.6 rule 5).
   */
  readonly channels?: readonly ChannelWrite[];
  /** What a `map` dispatched, for the trace (PRD 5.3, 5.6). */
  readonly dispatches?: readonly DispatchRecord[];
  /** The trace of the subflow instance a `flow:` node ran (grammar 8.5). */
  readonly inner?: readonly TraceEntry[];
  /** Which member of its route served each model call (PRD 5.9). */
  readonly models?: readonly ModelCall[];
}

/** One channel a `map` node writes, with every contribution in index order. */
export interface ChannelWrite {
  readonly channel: string;
  readonly reduce: Reduce;
  /** The instances' contributions, ascending by source-item index. */
  readonly values: readonly unknown[];
}

/** The `execution` root, bound for an expression (grammar 4.1). */
const EXECUTION_SHAPE = {
  properties: { id: "string", session_key: "string", item_index: "int" },
} as const;

/**
 * Run one node: its input, its activity, its writes, and its routing decision —
 * in one LangGraph task, which is what makes grammar 7.6's P1 (a node's edges
 * are evaluated only after it has completed) structural rather than asserted.
 *
 * The answer is a `Command`: LangGraph's one primitive that carries a state
 * update **and** the control transfer together, so the counter a bounded edge
 * spends (grammar 7.4) and the branch it spends it on land in the same write.
 *
 * **What a guard sees.** The state as of the start of this step, with this
 * node's own writes applied through their channels' reduce policies — so a
 * guard here does not see a concurrent sibling's writes. Grammar 7.6's P1 is
 * per node ("a node's outgoing edges are evaluated only after **that node** has
 * completed"), and this is that reading: routing is part of the node's own task.
 * Reachable only from a fork whose branches write a channel the other branch's
 * guard reads, which
 * `a_guard_sees_its_own_writes_and_not_a_concurrent_siblings` pins.
 *
 * **Which errors `on_error` governs.** The node's own, and only those. Grammar 9
 * is a policy over an *activity* — what the model, the process or the request
 * did — while reading an unset channel or an absent property fails the
 * **execution** (grammar 10.1, Decisions D78, D101, D110). Those are different
 * outcomes, and `skip` and `fallback` would swallow the second into the first,
 * so the input is built **outside** the policy-governed `try`: an expression
 * that cannot be evaluated propagates, and only what the activity did reaches
 * the `catch`.
 *
 * **What a failure leaves behind.** Every way out of here that throws carries
 * this node's own trace entry on the error ([`carryEntry`]), because a thrown
 * task returns no `Command` and LangGraph discards the superstep it was in —
 * so without that, the one node a reader of a failed run most wants to see is
 * the one node the trace could never hold (PRD 5.3).
 */
export async function runNode(
  descriptor: NodeDescriptor,
  state: GraphStateLike,
): Promise<Command> {
  const run = state.$run;
  const step = run.step + 1;
  const traversal = run.traversals[descriptor.node] ?? 0;

  // Grammar 9.3 level 1, laid over the levels the compiler resolved: the
  // instantiating `flow:` node's `policy:`, which this instance carries.
  const policy = effectivePolicy(descriptor.policy, descriptor.exempt === true, run.policy);

  // The roots a node's *configuration* reads: no `<node>.output` among them,
  // because a node never reads another's output (Decision D42).
  const configuration: Roots = {
    input: bindRoot(run.input, descriptor.shapes.input),
    state: bindRoot(declared(state, descriptor.shapes.state), descriptor.shapes.state),
    execution: bindRoot(run.execution, EXECUTION_SHAPE),
  };
  const view: NodeView = { state, run, roots: configuration };

  const base: Partial<RunChannel> = {
    step,
    traversals: { [descriptor.node]: traversal + 1 },
  };

  let output: unknown;
  let history: readonly unknown[] | undefined;
  let channels: readonly ChannelWrite[] | undefined;
  let dispatches: readonly DispatchRecord[] | undefined;
  let inner: readonly TraceEntry[] | undefined;
  let models: readonly ModelCall[] | undefined;
  let attempts = 0;
  let skipped = false;
  let failure: NodeFailure | undefined;
  // Every store op this node performs, across every attempt its policy makes:
  // an effect that happened is an effect that happened, and a record that kept
  // only the last attempt's would describe a run the store did not see
  // (PRD 5.8). Owned here rather than by `runActivity` so a node that fails
  // still reports what it wrote before it did.
  const storeRecords: StoreRecord[] = [];

  /** This node's entry, for a failure that leaves nothing else behind. */
  const aborted = (error: unknown, made: number, routing?: RoutingDecision): TraceEntry => {
    // A subflow that failed still made a trace, and it is the only account of
    // what happened inside the boundary (grammar 8.5, PRD 5.3).
    const held = inner ?? traceOf(error);
    const dispatched = dispatches ?? dispatchesOf(error) ?? plannedDispatches(input);
    return {
      step,
      flow: descriptor.flow,
      node: descriptor.node,
      traversal,
      outcome: "failed",
      attempts: made,
      ...(routing === undefined ? {} : { routing }),
      ...(dispatched === undefined ? {} : { dispatches: dispatched }),
      ...(held === undefined ? {} : { inner: held }),
      ...(storeRecords.length === 0 ? {} : { stores: [...storeRecords] }),
      ...(models === undefined ? {} : { models }),
      error: describe(error),
    };
  };

  // Outside the `try` on purpose — see *Which errors `on_error` governs* above.
  // A `map`'s whole dispatch is decided here too: which items, which routes, and
  // what each instance is passed, so a per-item binding that reads an absent
  // value fails the execution rather than one item (grammar 8.6, D110).
  let input: unknown;
  try {
    input = descriptor.input(configuration, view);
  } catch (error) {
    // No attempt was made: the node never ran, so `0` is what it made.
    throw carryEntry(error, aborted(error, 0));
  }

  try {
    const answer = await runActivity(
      descriptor.flow,
      descriptor.node,
      policy,
      run.execution,
      (context) => descriptor.run(input, context, view),
      storeRecords,
    );
    attempts = answer.attempts;
    output = answer.value.output;
    history = answer.value.history;
    channels = answer.value.channels;
    dispatches = answer.value.dispatches;
    inner = answer.value.inner;
    models = answer.value.models;
  } catch (error) {
    const strategy = policy.onError;
    failure = error instanceof NodeFailure ? error : undefined;
    if (failure?.cause instanceof SubflowFailure) inner = failure.cause.trace;
    // A `map` that failed still dispatched: the items that completed had their
    // effects and the detached ones were delivered, and the records are the only
    // account of them (PRD 5.3, 5.6). They ride out on the `ItemFailure`, which
    // `runActivity` wrapped, so the chain is walked the way `inner` above is —
    // and when the failure is the node's own **deadline** there is no
    // `ItemFailure` to walk, because the deadline is raced and the map's promise
    // was abandoned holding it. The plan is where the records are then read
    // from, and is the reason a timed-out fan-out still says what it dispatched.
    dispatches = dispatchesOf(error) ?? plannedDispatches(input);
    // `runActivity` wraps everything the activity threw in a `NodeFailure`
    // carrying the attempts it *made*, so the fallback is for an error that
    // reached here without one being made at all — and `0` is what that is.
    attempts = failure?.attempts ?? 0;
    if (strategy === "fail") throw carryEntry(error, aborted(error, attempts));
    if (typeof strategy === "object") {
      // The node's own outgoing edges are not evaluated: the fallback target is
      // scheduled instead (grammar 9.2, Decision D21).
      const entry: TraceEntry = {
        step,
        flow: descriptor.flow,
        node: descriptor.node,
        traversal,
        outcome: "failed",
        attempts,
        error: describe(error),
        ...(dispatches === undefined ? {} : { dispatches }),
        ...(inner === undefined ? {} : { inner }),
        ...(storeRecords.length === 0 ? {} : { stores: [...storeRecords] }),
        ...(models === undefined ? {} : { models }),
        fallback: strategy.fallback,
      };
      return new Command({
        update: {
          $run: { ...base, ...retained(descriptor, undefined, true), trace: [entry] },
        },
        goto: [strategy.fallback],
      });
    }
    skipped = true;
  }

  const update: Record<string, unknown> = {};
  const localState = declared(state, descriptor.shapes.state);
  const written: string[] = [];
  if (!skipped) {
    const fields = (output ?? {}) as Record<string, unknown>;
    for (const write of descriptor.writes) {
      const value = fields[write.field];
      // A field the result does not carry performs no write (Decision D110).
      if (value === undefined) continue;
      update[write.channel] = value;
      localState[write.channel] = applyWrite(state[write.channel], value, write.reduce);
      written.push(write.channel);
    }
    // A `map`'s writers are its instances, so what it landed arrives already
    // ordered by source-item index and goes to the channel as one batch its
    // reducer unpacks — or as the folded value where the channel could not be
    // handed a batch (grammar 7.6.4 clause 2, and see `orderedUpdate`).
    for (const batch of channels ?? []) {
      // A channel with no contribution at all is not a write: `orderedChannels`
      // opens an entry only for a field an instance really produced, so this
      // guards the invariant rather than naming a case.
      if (batch.values.length === 0) continue;
      update[batch.channel] = orderedUpdate(batch);
      localState[batch.channel] = batch.values.reduce(
        (held, value) => applyWrite(held, value, batch.reduce),
        state[batch.channel],
      );
      written.push(batch.channel);
    }
    if (history !== undefined && history.length > 0) {
      update["messages"] = history;
    }
  }

  let routing: RoutingDecision;
  try {
    const roots: Roots = {
      ...configuration,
      state: bindRoot(localState, descriptor.shapes.state),
      [descriptor.node]: skipped
        ? undefined
        : bindRoot({ output }, { properties: { output: descriptor.shapes.output } }),
    };
    routing = route(
      descriptor.flow,
      descriptor.node,
      descriptor.edges,
      roots,
      run.iterations,
      skipped,
    );
  } catch (error) {
    // A route that could not be decided is still a routing decision, and the
    // guard values are the whole of it — so a `NoViableRoute` hands them on
    // (PRD 5.3). `targets` is empty because that is what went wrong.
    throw carryEntry(
      error,
      aborted(
        error,
        attempts,
        error instanceof NoViableRoute
          ? { edges: [...error.decisions], targets: [], counters: {} }
          : undefined,
      ),
    );
  }

  const entry: TraceEntry = {
    step,
    flow: descriptor.flow,
    node: descriptor.node,
    traversal,
    outcome: skipped ? "skipped" : "completed",
    attempts,
    writes: written,
    routing,
    ...(dispatches === undefined ? {} : { dispatches }),
    ...(inner === undefined ? {} : { inner }),
    ...(storeRecords.length === 0 ? {} : { stores: [...storeRecords] }),
    ...(models === undefined ? {} : { models }),
    ...(failure === undefined ? {} : { error: failure.message }),
  };
  update["$run"] = {
    ...base,
    ...retained(descriptor, output, skipped),
    iterations: routing.counters,
    trace: [entry],
  };
  return new Command({ update, goto: routing.targets });
}

/**
 * What a node keeps in `$run.outputs` for a `map.over` in a later step to read
 * (grammar 8.6 rule 11, and see [`RunChannel.outputs`]).
 *
 * A node no map reads keeps nothing. A node that was **skipped** — or that took
 * a `fallback:`, which produced no result either — writes its key back holding
 * `undefined`, so a map dispatches zero instances rather than fanning out over
 * the array the producer left on an earlier traversal of the same cycle.
 */
function retained(
  descriptor: NodeDescriptor,
  output: unknown,
  skipped: boolean,
): Partial<RunChannel> {
  if (descriptor.retains !== true) return {};
  return { outputs: { [descriptor.node]: skipped ? undefined : output } };
}

function bindRoot(value: unknown, shape: Shape): CelValue {
  const bound = bind(value, shape);
  if (bound === undefined) {
    throw new CelError("a root is absent");
  }
  return bound;
}

/**
 * The channels the `state` root exposes: exactly the ones the composition
 * declares (grammar 4.1, 10.1).
 *
 * The graph's state object holds more than that — `messages`, the implicit
 * conversation history of grammar 10.4, and `$run`, this compiler's own channel
 * — and neither is a name an expression may read: the validator types `state`
 * from the `state:` section alone, so `state.messages` is an undefined channel
 * at compile time and would be a readable object at run time. Narrowing here is
 * what keeps the runtime scope equal to the declared one.
 *
 * It is also what keeps a node's cost flat in the length of the run. `bind`
 * copies what it is given, and `$run` carries the whole routing trace (PRD 5.3),
 * so binding the raw state object copied every earlier step's trace entries into
 * CEL values at every node — quadratic in the trace, for a root nothing can
 * name.
 */
function declared(
  state: Readonly<Record<string, unknown>>,
  shape: Shape,
): Record<string, unknown> {
  const properties = typeof shape === "object" && "properties" in shape ? shape.properties : {};
  const narrowed: Record<string, unknown> = {};
  for (const channel of Object.keys(properties)) {
    // An unset channel stays absent rather than arriving as `undefined`: which
    // channels a value carries is what `has()` reads and what a failed read
    // reports (grammar 10.1, Decision D78).
    if (state[channel] !== undefined) narrowed[channel] = state[channel];
  }
  return narrowed;
}

// ---------------------------------------------------------------------------
// Evaluating an expression at a node
// ---------------------------------------------------------------------------

export type { CelValue, Roots, Shape };
export { CelError, bind, evaluate, evaluateGuard, toJson };
