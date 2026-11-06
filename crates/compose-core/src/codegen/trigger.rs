//! `src/triggers.ts`: the composition's declared trigger table (grammar 13).
//!
//! [`super::serve`] emits the app; this emits what it serves. One entry per
//! declared **`http`** trigger, carrying everything grammar 13.3 makes a
//! property of the route rather than of the flow: its `path:` and `method:`, its
//! `respond:` mode and the sync budget that goes with it, and the CEL that turns
//! one request payload into the flow's inputs, its session identity, and its
//! completion webhook.
//!
//! # The two tables
//!
//! Beside the routes there is a second, much smaller table: one entry per
//! declared **`manual`** trigger. A `manual` trigger is not a route and enables
//! nothing — every flow is runnable from the CLI whether or not a trigger names
//! it (Decision D64), so `src/cli.ts` reads the flow registry to decide *what*
//! it may run. What a declared `manual` trigger does carry is the one key
//! grammar 13.2 makes a property of the CLI entry rather than of the flow: a
//! `session_key:` **remap**, over a payload whose single member is the
//! `--session` argument. Undeclared, the CLI entry keys off `--session` itself
//! (`"payload.session"`, the key's own default); declared, that expression is
//! what turns the argument into the session identity a `scope: session` store
//! partitions by (grammar 11.3). Emitting the table is what keeps a declared
//! remap from being a key the compiler accepts and nothing evaluates.
//!
//! # The tables `serve` runs rather than mounts
//!
//! `schedule` and `event` triggers execute under `serve` (PRD resolved q78):
//! the scheduler `./sources.ts` runs evaluates every `scheduleTriggers` entry's
//! `cron:` in its `timezone:`, and the consumer it runs for each `kind: command`
//! source admits every line to every `eventTriggers` entry bound to it. Neither
//! is a route. Beside them, `eventSources` is the active target's
//! `event_sources:` — every entry, with its kind, so `serve` can refuse a kind
//! this release does not implement **by name** at start (the q63 posture), and
//! for the one it does, the program, its arguments, its `checkpoint:` and its
//! resolved `restart:`.
//!
//! # The payload is bound through a declared shape
//!
//! Grammar 13.3 fixes what a request payload holds — `body`, `query`, `headers`,
//! `path`, `method` — and the validator types every trigger expression against
//! exactly that. The emitted shape is the same one, so an expression the
//! validator accepted binds here through the same declaration: `payload.query.x`
//! is a `string` on both sides, and a number in a decoded body is whatever the
//! body says it is.

use crate::ast::deploy::EventSourceKind;
use crate::ast::trigger::{HmacAlgorithm, Respond, SignatureEncoding, TriggerMethod};
use crate::ir::Ir;
use crate::ir::trigger::{CallbackAuth, HttpTrigger, InboundAuth, TriggerKind};

use super::{graph, names, policy};

/// `src/triggers.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(PREAMBLE);
    contents.push_str(&table(ir));
    super::GeneratedFile {
        path: "src/triggers.ts".to_string(),
        contents,
    }
}

/// The part of the module that is the same in every project.
const PREAMBLE: &str = r#"//
// The composition's declared triggers (grammar 13, PRD 5.11).
//
// `./serve.ts` is the app; `httpTriggers` is the table it serves. Beside it,
// `manualTriggers` is what `./cli.ts` reads: a `manual` trigger declares no
// route — invocation from the CLI is universal and needs no trigger (Decision
// D64) — but it may declare the `session_key:` that remaps `--session` into the
// session identity a `scope: session` store partitions by (grammar 13.2, 11.3).
//
// `scheduleTriggers` and `eventTriggers` are what `./sources.ts` runs under
// `serve` (PRD resolved q78): a schedule's ticks and a command source's lines
// start executions through the same admission an `http` request does
// (`./serve.ts`'s `admit`). `eventSources` is the active target's
// `event_sources:`, every entry with its kind — a kind this release does not
// implement is refused by name when `serve` starts (grammar 14.4).
//
// A trigger's `dedupe_window:` and `concurrency:` block — defaults applied —
// are what that admission's intake ledger decides under (PRD resolved q77): a
// dedupe key's TTL, and the groups a trigger's arrivals are leased slots of.

import * as runtime from "./runtime.ts";

/**
 * The payload shape grammar 13.3 fixes, as the emitted evaluator reads it.
 *
 * The same declaration the validator typed every trigger expression against, so
 * an expression it accepted binds here through the same types.
 */
const httpPayload: runtime.Shape = {
  properties: {
    body: { properties: {}, rest: "any" },
    query: { properties: {}, rest: "string" },
    headers: { properties: {}, rest: "string" },
    path: "string",
    method: "string",
    // Present only where the trigger's `concurrency:` declares `debounce:`,
    // on the payload the debounced execution starts with (PRD resolved q77).
    coalesced: "int",
  },
};

/** One declared `http` trigger (grammar 13.3). */
export interface HttpTrigger {
  /** The trigger's own name, which is what a diagnostic and a report call it. */
  readonly name: string;
  /** `flow:` — the execution's entry module. */
  readonly flow: string;
  /** `path:` — the route, defaulting to `/triggers/<name>`. */
  readonly path: string;
  /** `method:` — defaulting to `POST`. */
  readonly method: "GET" | "POST" | "PUT";
  /** `respond:` — defaulting to `async`. */
  readonly respond: "sync" | "async";
  /** `timeout:` — the sync response budget, in milliseconds (`sync` only). */
  readonly timeoutMs?: number;
  /**
   * Whether the app decodes a body for this route.
   *
   * `false` on a `GET`, which has none: `payload.body` is then `{}` — present
   * and readable, so `has(payload.body.goal)` answers `false` instead of
   * erroring (Decision D117).
   */
  readonly readsBody: boolean;
  /** `input:` — the flow inputs this trigger binds from one payload. */
  input(payload: unknown): Record<string, unknown>;
  /** `session_key:` — the session identity for session-scoped stores (grammar 11.3). */
  sessionKey?(payload: unknown): string;
  /** `callback:` — the completion webhook, on an `async` trigger. */
  callback?(payload: unknown): string;
  /**
   * `auth:` — how a caller of this trigger is verified, and with it the resume
   * and status routes of every execution it starts (grammar 13.3, PRD resolved
   * q32).
   *
   * Absent leaves all three routes open.
   */
  readonly auth?: InboundAuth;
  /** `callback_auth:` — how a delivery identifies itself (grammar 13.3). */
  readonly callbackAuth?: CallbackAuth;
  /**
   * `callback_allow:` — where a callback may point.
   *
   * Absent is the documented test posture: the trigger signs nothing and may
   * POST anywhere. Present, it is matched when the URL is *read* — at the
   * delivery, not at the start — because the URL comes out of the request
   * payload and is attacker-controlled by construction (Decision D110, D127).
   */
  readonly callbackAllow?: readonly string[];
  /**
   * `dedupe_key:` — the key a repeat delivery is recognised by (grammar 13.3,
   * PRD resolved q77 ruling a). No default: absent, nothing is deduplicated.
   */
  dedupeKey?(payload: unknown): string;
  /** `dedupe_window:` — how long a key is held, in milliseconds; beside `dedupeKey`. */
  readonly dedupeWindowMs?: number;
  /** `concurrency:` — the intake groups this trigger's requests are admitted under. */
  readonly concurrency?: Concurrency<unknown>;
}

/**
 * A trigger's `concurrency:` block, defaults applied (grammar 13.1, PRD
 * resolved q77 ruling b, c, Decision D163).
 *
 * A group is a **lease** held by its running executions, which `./serve.ts`
 * computes from the journal's rows at every admission: the arrivals whose `key`
 * answers one value share one, `limit` of them hold it at once, and
 * `maxRunning` bounds the trigger's executions across every group.
 */
export interface Concurrency<P> {
  /** `key:` — the group an arrival belongs to; absent, one group for the trigger. */
  key?(payload: P): string;
  /** `limit:` — defaulting to `1`. */
  readonly limit: number;
  /** `on_conflict:` — defaulting to `queue`. */
  readonly onConflict: "queue" | "drop" | "supersede";
  /** `debounce:`, in milliseconds: the arrivals for a key coalesce into one execution. */
  readonly debounceMs?: number;
  /**
   * `max_running:` — **the trigger's** total across all its groups. Not a
   * placement's `max_running:` (grammar 14.1), which a worker session holds.
   */
  readonly maxRunning?: number;
}

/**
 * The inbound scheme a trigger enforces, with every grammar 13.3 default
 * already applied.
 *
 * Resolved rather than recorded-as-written, unlike `path:` and `method:`: every
 * parameter that decides whether a credential verifies carries the value the
 * trigger enforces, because a verifier that re-derived a default and got it
 * wrong would not fail a build — it would accept the wrong request.
 */
export type InboundAuth =
  | {
      readonly scheme: "bearer";
      /** Matched case-insensitively; HTTP/2 lowercases every name (grammar 13.3). */
      readonly header: string;
      readonly prefix: string;
      /** The **variable name** holding the expected token; never the token. */
      readonly tokenEnv: string;
    }
  | {
      readonly scheme: "hmac";
      readonly header: string;
      readonly algorithm: "sha1" | "sha256" | "sha512";
      readonly encoding: "hex" | "base64";
      readonly prefix: string;
      /** The **variable name** holding the signing key; never the key. */
      readonly secretEnv: string;
    };

/**
 * The outbound identity a delivery carries (grammar 13.3, PRD resolved q33).
 *
 * At least one half is present, and both together are legal: a receiver that
 * checks a token and a receiver that verifies a signature are two receivers.
 * Outbound signing takes no parameters — HMAC-SHA256 in hex under
 * `X-AgentCompose-Signature` — so one receiver-side recipe verifies every
 * agent-compose deployment.
 */
export interface CallbackAuth {
  readonly bearer?: {
    readonly header: string;
    readonly prefix: string;
    readonly tokenEnv: string;
  };
  readonly hmac?: { readonly secretEnv: string };
}

/**
 * The payload shape grammar 13.2 fixes for a `manual` trigger: one member.
 *
 * "The manual payload has exactly one member — `payload.session`, the CLI's
 * `--session <key>`", which is also what the validator types a manual trigger's
 * `session_key:` against.
 */
const manualPayload: runtime.Shape = { properties: { session: "string" } };

/** The roots an `http` trigger's expressions are evaluated against. */
function roots(payload: unknown): runtime.Roots {
  return { payload: runtime.bind(payload, httpPayload) };
}

/**
 * The payload shape grammar 13.4 fixes for a `schedule` trigger: the tick's
 * nominal instant in the trigger's zone (RFC 3339), and the trigger's name.
 */
const schedulePayload: runtime.Shape = {
  properties: { scheduled_at: "string", trigger: "string", coalesced: "int" },
};

/**
 * The payload shape grammar 13.5 fixes for an `event` trigger. A `kind:
 * command` source fills it from one line: `body` is the decoded object, `id`
 * its `checkpoint:` value or the SHA-256 of the line, `attributes` is `{}`,
 * `source` the logical name (grammar 14.4).
 */
const eventPayload: runtime.Shape = {
  properties: {
    id: "string",
    body: { properties: {}, rest: "any" },
    attributes: { properties: {}, rest: "string" },
    source: "string",
    coalesced: "int",
  },
};

/** What a command source's `checkpoint:` reads: the event before it has an id. */
const checkpointPayload: runtime.Shape = {
  properties: {
    body: { properties: {}, rest: "any" },
    attributes: { properties: {}, rest: "string" },
    source: "string",
  },
};

/** The roots a `schedule` trigger's expressions are evaluated against. */
function scheduleRoots(payload: SchedulePayload): runtime.Roots {
  return { payload: runtime.bind(payload, schedulePayload) };
}

/** The roots an `event` trigger's expressions are evaluated against. */
function eventRoots(payload: EventPayload): runtime.Roots {
  return { payload: runtime.bind(payload, eventPayload) };
}

/** The roots a command source's `checkpoint:` is evaluated against. */
function checkpointRoots(event: LineEvent): runtime.Roots {
  return { payload: runtime.bind(event, checkpointPayload) };
}

/** The roots a `manual` trigger's `session_key:` is evaluated against. */
function manualRoots(session: string): runtime.Roots {
  return { payload: runtime.bind({ session }, manualPayload) };
}

/** One trigger expression whose declared result type is `string`. */
function text(source: string, bound: runtime.Roots, site: string): string {
  const answer = runtime.toJson(runtime.evaluate(source, bound));
  if (typeof answer !== "string") {
    throw new runtime.CelError(`${site} answered ${JSON.stringify(answer)} rather than a string`);
  }
  return answer;
}

/**
 * One declared `manual` trigger (grammar 13.2).
 *
 * It contributes no route and gates nothing: `./cli.ts` runs any flow this
 * composition declares, named or not (Decision D64). The entry exists for the
 * one key that changes what a CLI run *does* — `session_key:`.
 */
export interface ManualTrigger {
  /** The trigger's own name, which is what a diagnostic and a report call it. */
  readonly name: string;
  /** `flow:` — the CLI entry this trigger describes. */
  readonly flow: string;
  /**
   * `session_key:` — the declared remap of `--session`, when there is one.
   *
   * Absent where the trigger declares none, which is the key's own
   * `"payload.session"` default: the argument *is* the identity, and evaluating
   * an expression to say so would answer what was passed in.
   */
  sessionKey?(session: string): string;
}

/** One tick's payload (grammar 13.4). */
export interface SchedulePayload {
  readonly scheduled_at: string;
  readonly trigger: string;
  /** The arrivals a debounce coalesced into this execution (PRD resolved q77 ruling c). */
  readonly coalesced?: number;
}

/** One event's payload (grammar 13.5, 14.4). */
export interface EventPayload {
  readonly id: string;
  readonly body: Readonly<Record<string, unknown>>;
  readonly attributes: Readonly<Record<string, string>>;
  readonly source: string;
  /** The arrivals a debounce coalesced into this execution (PRD resolved q77 ruling c). */
  readonly coalesced?: number;
}

/** One line of a command source, decoded, before it has an id (grammar 14.4). */
export interface LineEvent {
  readonly body: Readonly<Record<string, unknown>>;
  readonly attributes: Readonly<Record<string, string>>;
  readonly source: string;
}

/**
 * One declared `schedule` trigger (grammar 13.4), which `serve`'s scheduler
 * fires at every tick of its `cron:` in its `timezone:` (PRD resolved q78).
 */
export interface ScheduleTrigger {
  readonly name: string;
  readonly flow: string;
  /** `cron:` — five fields, already held to the evaluator's grammar by `validate`. */
  readonly cron: string;
  /** `timezone:` — an IANA name, defaulting to `UTC`. */
  readonly timezone: string;
  /** `input:` — the flow inputs this trigger binds from one tick. */
  input(payload: SchedulePayload): Record<string, unknown>;
  /** `session_key:` — the session identity for session-scoped stores. */
  sessionKey?(payload: SchedulePayload): string;
  /** `concurrency:` — the intake groups this trigger's ticks are admitted under. */
  readonly concurrency?: Concurrency<SchedulePayload>;
}

/**
 * One declared `event` trigger (grammar 13.5): every event its `source:`
 * produces is admitted to it, deduplicated on its own `dedupe_key:`.
 */
export interface EventTrigger {
  readonly name: string;
  readonly flow: string;
  /** `source:` — the logical name the active target's `event_sources:` binds. */
  readonly source: string;
  /** `input:` — the flow inputs this trigger binds from one event. */
  input(payload: EventPayload): Record<string, unknown>;
  /** `session_key:` — the session identity for session-scoped stores. */
  sessionKey?(payload: EventPayload): string;
  /** `dedupe_key:` — defaulting to `payload.id`. */
  dedupeKey(payload: EventPayload): string;
  /** `dedupe_window:` — how long a key is held, in milliseconds; defaulting to 24h. */
  readonly dedupeWindowMs: number;
  /** `concurrency:` — the intake groups this trigger's events are admitted under. */
  readonly concurrency?: Concurrency<EventPayload>;
  /**
   * `auth:` — what the status, resume and cancel routes of every execution
   * this trigger starts enforce, and what its source's `GET /sources/:name`
   * enforces (grammar 13.5, PRD resolved q32 one level up, Decision D162).
   * Nobody calls an `event` trigger, so it authenticates no start.
   *
   * Absent leaves those routes open.
   */
  readonly auth?: InboundAuth;
}

/** grammar 14.4's kinds. */
export type EventSourceKind = "command" | "redis_streams" | "sqs" | "nats";

/**
 * The kinds `serve` runs a consumer for in this release (PRD resolved q78).
 * The others keep their vocabulary and are refused at start, by name.
 */
export const IMPLEMENTED_SOURCE_KINDS: readonly EventSourceKind[] = ["command"];

/** One argument of a command source's program (grammar 14.4). */
export type CommandArgument =
  | { readonly text: readonly runtime.Interpolation[] }
  /** An argument written exactly `{checkpoint}`: the source's newest checkpoint. */
  | { readonly checkpoint: true };

/** A `kind: command` source's program, `restart:` resolved (grammar 14.4). */
export interface CommandSource {
  /** `command:` — argv[0], interpolated at start (grammar 4.3 class 2). */
  readonly command: readonly runtime.Interpolation[];
  readonly args: readonly CommandArgument[];
  /** `checkpoint:` — the event's id, and the position handed back on restart. */
  checkpoint?(event: LineEvent): string;
  readonly restartBackoffMs: number;
  readonly restartMaxMs: number;
}

/** One `event_sources:` entry of the active target (grammar 14.4). */
export interface EventSource {
  readonly name: string;
  readonly kind: EventSourceKind;
  /** The program, on a `kind: command` source and no other. */
  readonly command?: CommandSource;
}

"#;

/// Every table: the routes `./serve.ts` mounts, the CLI entries `./cli.ts`
/// reads, and the schedules, event triggers and sources `./sources.ts` runs.
fn table(ir: &Ir) -> String {
    let mut text = http_table(ir);
    text.push('\n');
    text.push_str(&manual_table(ir));
    text.push('\n');
    text.push_str(&schedule_table(ir));
    text.push('\n');
    text.push_str(&event_table(ir));
    text.push('\n');
    text.push_str(&source_table(ir));
    text
}

/// One trigger's `input:` as a function of one payload, its roots bound once.
fn input_function(
    input: Option<&crate::ir::binding::Bindings>,
    payload: &str,
    roots: &str,
) -> String {
    let bindings: Vec<_> = input
        .map(|map| map.entries.iter().collect())
        .unwrap_or_default();
    if bindings.is_empty() {
        return "    input: () => ({}),\n".to_string();
    }
    let mut text = format!(
        "    input: (payload: {payload}) => {{\n      const bound = {roots}(payload);\n      return {{\n"
    );
    for binding in bindings {
        text.push_str(&format!(
            "        {}: runtime.toJson(runtime.evaluate({}, bound)),\n",
            names::string(binding.name.value.as_str()),
            names::string(binding.value.value.as_str())
        ));
    }
    text.push_str("      };\n    },\n");
    text
}

/// The `schedule` entries, in the IR's canonical order (Decision D55).
fn schedule_table(ir: &Ir) -> String {
    let mut text = String::from("export const scheduleTriggers: readonly ScheduleTrigger[] = [\n");
    for trigger in ir
        .triggers
        .iter()
        .flat_map(|section| section.entries.values())
    {
        let TriggerKind::Schedule(schedule) = &trigger.kind else {
            continue;
        };
        let name = trigger.name.value.as_str();
        text.push_str("  {\n");
        text.push_str(&format!("    name: {},\n", names::string(name)));
        text.push_str(&format!(
            "    flow: {},\n",
            names::string(&trigger.flow.value.to_string())
        ));
        text.push_str(&format!(
            "    cron: {},\n",
            names::string(&schedule.cron.value)
        ));
        text.push_str(&format!(
            "    timezone: {},\n",
            names::string(
                schedule
                    .timezone
                    .as_ref()
                    .map_or("UTC", |timezone| timezone.value.as_str())
            )
        ));
        text.push_str(&input_function(
            schedule.input.as_ref(),
            "SchedulePayload",
            "scheduleRoots",
        ));
        if let Some(key) = &trigger.session_key {
            text.push_str(&format!(
                "    sessionKey: (payload) => text({}, scheduleRoots(payload), {}),\n",
                names::string(key.value.as_str()),
                names::string(&format!("`session_key` of the trigger `{name}`"))
            ));
        }
        text.push_str(&concurrency(trigger, "scheduleRoots"));
        text.push_str("  },\n");
    }
    text.push_str("];\n");
    text
}

/// A trigger's `concurrency:` block, defaults applied, as one table member —
/// or nothing, where it declares none (grammar 13.1, PRD resolved q77).
fn concurrency(trigger: &crate::ir::Trigger, roots: &str) -> String {
    let Some(block) = &trigger.concurrency else {
        return String::new();
    };
    let name = trigger.name.value.as_str();
    let mut text = String::from("    concurrency: {\n");
    if let Some(key) = &block.key {
        text.push_str(&format!(
            "      key: (payload) => text({}, {roots}(payload), {}),\n",
            names::string(key.value.as_str()),
            names::string(&format!("`concurrency.key` of the trigger `{name}`"))
        ));
    }
    text.push_str(&format!("      limit: {},\n", block.limit()));
    text.push_str(&format!(
        "      onConflict: {},\n",
        names::string(block.on_conflict().as_str())
    ));
    if let Some(debounce) = &block.debounce {
        text.push_str(&format!(
            "      debounceMs: {},\n",
            policy::milliseconds(&debounce.value)
        ));
    }
    if let Some(max) = &block.max_running {
        text.push_str(&format!("      maxRunning: {},\n", max.value));
    }
    text.push_str("    },\n");
    text
}

/// A trigger's `dedupe_window:`, or the `24h` grammar 13.3 and 13.5 default
/// it to, in milliseconds (PRD resolved q77 ruling a).
fn window_ms(window: Option<&crate::diag::Spanned<crate::ast::common::Duration>>) -> u64 {
    window.map_or(crate::ast::trigger::DEFAULT_DEDUPE_WINDOW_MS, |window| {
        policy::milliseconds(&window.value)
    })
}

/// The `event` entries, in the IR's canonical order (Decision D55).
fn event_table(ir: &Ir) -> String {
    let mut text = String::from("export const eventTriggers: readonly EventTrigger[] = [\n");
    for trigger in ir
        .triggers
        .iter()
        .flat_map(|section| section.entries.values())
    {
        let TriggerKind::Event(event) = &trigger.kind else {
            continue;
        };
        let name = trigger.name.value.as_str();
        text.push_str("  {\n");
        text.push_str(&format!("    name: {},\n", names::string(name)));
        text.push_str(&format!(
            "    flow: {},\n",
            names::string(&trigger.flow.value.to_string())
        ));
        text.push_str(&format!(
            "    source: {},\n",
            names::string(event.source.value.as_str())
        ));
        text.push_str(&input_function(
            event.input.as_ref(),
            "EventPayload",
            "eventRoots",
        ));
        if let Some(key) = &trigger.session_key {
            text.push_str(&format!(
                "    sessionKey: (payload) => text({}, eventRoots(payload), {}),\n",
                names::string(key.value.as_str()),
                names::string(&format!("`session_key` of the trigger `{name}`"))
            ));
        }
        // Grammar 13.5's default is the expression `payload.id`, evaluated like
        // any the author wrote — one path for both, so the default cannot read
        // the payload differently from the key it stands in for.
        let dedupe = event
            .dedupe_key
            .as_ref()
            .map_or("payload.id", |key| key.value.as_str());
        text.push_str(&format!(
            "    dedupeKey: (payload) => text({}, eventRoots(payload), {}),\n",
            names::string(dedupe),
            names::string(&format!("`dedupe_key` of the trigger `{name}`"))
        ));
        text.push_str(&format!(
            "    dedupeWindowMs: {},\n",
            window_ms(event.dedupe_window.as_ref())
        ));
        text.push_str(&concurrency(trigger, "eventRoots"));
        if let Some(auth) = &event.auth {
            text.push_str(&inbound(auth));
        }
        text.push_str("  },\n");
    }
    text.push_str("];\n");
    text
}

/// The active target's `event_sources:`, every entry, by name (grammar 14.4).
fn source_table(ir: &Ir) -> String {
    let mut text = String::from("export const eventSources: readonly EventSource[] = [\n");
    for source in ir
        .deploy
        .event_sources
        .iter()
        .flat_map(|section| section.entries.values())
    {
        let name = source.name.value.as_str();
        text.push_str("  {\n");
        text.push_str(&format!("    name: {},\n", names::string(name)));
        text.push_str(&format!(
            "    kind: {},\n",
            names::string(source.kind.as_str())
        ));
        if let (EventSourceKind::Command, Some(command)) = (source.kind, &source.command) {
            let site = format!("deploy.event_sources.{name}");
            text.push_str("    command: {\n");
            text.push_str(&format!(
                "      command: {},\n",
                graph::interpolation(&command.command.value, &format!("{site}.command"))
            ));
            text.push_str("      args: [\n");
            for (index, arg) in command.args.iter().enumerate() {
                if arg.value.as_str() == CHECKPOINT_ARGUMENT {
                    text.push_str("        { checkpoint: true },\n");
                } else {
                    text.push_str(&format!(
                        "        {{ text: {} }},\n",
                        graph::interpolation(&arg.value, &format!("{site}.args[{index}]"))
                    ));
                }
            }
            text.push_str("      ],\n");
            if let Some(checkpoint) = &command.checkpoint {
                text.push_str(&format!(
                    "      checkpoint: (event) => text({}, checkpointRoots(event), {}),\n",
                    names::string(checkpoint.value.as_str()),
                    names::string(&format!("`checkpoint` of the event source `{name}`"))
                ));
            }
            text.push_str(&format!(
                "      restartBackoffMs: {},\n",
                command.restart_backoff_ms.min(1_u64 << 53)
            ));
            text.push_str(&format!(
                "      restartMaxMs: {},\n",
                command.restart_max_ms.min(1_u64 << 53)
            ));
            text.push_str("    },\n");
        }
        text.push_str("  },\n");
    }
    text.push_str("];\n");
    text
}

/// The argument grammar 14.4 replaces with the source's newest checkpoint —
/// written exactly so, as a whole argument (PRD resolved q78 ruling c).
pub const CHECKPOINT_ARGUMENT: &str = "{checkpoint}";

/// The `manual` entries, in the IR's canonical order (Decision D55).
fn manual_table(ir: &Ir) -> String {
    let mut text = String::from("export const manualTriggers: readonly ManualTrigger[] = [\n");
    for trigger in manual(ir) {
        let name = trigger.name;
        text.push_str("  {\n");
        text.push_str(&format!("    name: {},\n", names::string(name)));
        text.push_str(&format!("    flow: {},\n", names::string(&trigger.flow)));
        if let Some(expression) = trigger.session_key {
            text.push_str(&format!(
                "    sessionKey: (session) => text({}, manualRoots(session), {}),\n",
                names::string(expression),
                names::string(&format!("`session_key` of the trigger `{name}`"))
            ));
        }
        text.push_str("  },\n");
    }
    text.push_str("];\n");
    text
}

/// The route table.
fn http_table(ir: &Ir) -> String {
    let mut text = String::from("export const httpTriggers: readonly HttpTrigger[] = [\n");
    for trigger in triggers(ir) {
        let name = trigger.name;
        text.push_str("  {\n");
        text.push_str(&format!("    name: {},\n", names::string(name)));
        text.push_str(&format!("    flow: {},\n", names::string(&trigger.flow)));
        text.push_str(&format!("    path: {},\n", names::string(&trigger.path)));
        text.push_str(&format!("    method: {},\n", names::string(trigger.method)));
        text.push_str(&format!(
            "    respond: {},\n",
            names::string(trigger.respond)
        ));
        if let Some(timeout) = trigger.timeout_ms {
            text.push_str(&format!("    timeoutMs: {timeout},\n"));
        }
        text.push_str(&format!("    readsBody: {},\n", trigger.reads_body));
        if trigger.input.is_empty() {
            text.push_str("    input: () => ({}),\n");
        } else {
            // The roots are bound **once** per request rather than once per
            // binding: `runtime.bind` copies the value it is given, and a body
            // read by three fields would otherwise be copied three times.
            text.push_str(
                "    input: (payload) => {\n      const bound = roots(payload);\n      return {\n",
            );
            for (field, expression) in &trigger.input {
                text.push_str(&format!(
                    "        {}: runtime.toJson(runtime.evaluate({}, bound)),\n",
                    names::string(field),
                    names::string(expression)
                ));
            }
            text.push_str("      };\n    },\n");
        }
        for (key, spelling, expression) in [
            ("sessionKey", "session_key", trigger.session_key.as_ref()),
            ("callback", "callback", trigger.callback.as_ref()),
        ] {
            let Some(expression) = expression else {
                continue;
            };
            text.push_str(&format!(
                "    {key}: (payload) => text({}, roots(payload), {}),\n",
                names::string(expression),
                names::string(&format!("`{spelling}` of the trigger `{name}`"))
            ));
        }
        if let Some(auth) = trigger.auth {
            text.push_str(&inbound(auth));
        }
        if let Some(auth) = trigger.callback_auth {
            text.push_str(&outbound(auth));
        }
        if let Some(allow) = &trigger.callback_allow {
            text.push_str("    callbackAllow: [\n");
            for pattern in allow {
                text.push_str(&format!("      {},\n", names::string(pattern)));
            }
            text.push_str("    ],\n");
        }
        if let Some(key) = trigger.dedupe_key {
            text.push_str(&format!(
                "    dedupeKey: (payload) => text({}, roots(payload), {}),\n",
                names::string(key),
                names::string(&format!("`dedupe_key` of the trigger `{name}`"))
            ));
            text.push_str(&format!(
                "    dedupeWindowMs: {},\n",
                trigger.dedupe_window_ms
            ));
        }
        text.push_str(&concurrency(trigger.declared, "roots"));
        text.push_str("  },\n");
    }
    text.push_str("];\n");
    text
}

/// The inbound scheme, with grammar 13.3's defaults already resolved by the IR.
///
/// The credential is the **variable name** and never the value: env refs
/// survive unresolved into generated code (grammar 4.3, PRD resolved q15), and
/// the emitted app reads `process.env` at the moment it verifies. What makes
/// that safe is [`super::env`]'s walk over this same table — a deployment
/// missing the variable is refused at launch rather than on the first real
/// call.
fn inbound(auth: &InboundAuth) -> String {
    match auth {
        InboundAuth::Bearer(bearer) => format!(
            "    auth: {{\n      scheme: \"bearer\",\n      header: {},\n      prefix: {},\n      tokenEnv: {},\n    }},\n",
            names::string(&bearer.header),
            names::string(&bearer.prefix),
            names::string(&bearer.token.value.name)
        ),
        InboundAuth::Hmac(hmac) => format!(
            "    auth: {{\n      scheme: \"hmac\",\n      header: {},\n      algorithm: {},\n      encoding: {},\n      prefix: {},\n      secretEnv: {},\n    }},\n",
            names::string(&hmac.header),
            names::string(algorithm(hmac.algorithm)),
            names::string(encoding(hmac.encoding)),
            names::string(&hmac.prefix),
            names::string(&hmac.secret.value.name)
        ),
    }
}

/// The outbound identity a delivery carries — one half, the other, or both.
fn outbound(auth: &CallbackAuth) -> String {
    let mut text = String::from("    callbackAuth: {\n");
    if let Some(bearer) = &auth.bearer {
        text.push_str(&format!(
            "      bearer: {{ header: {}, prefix: {}, tokenEnv: {} }},\n",
            names::string(&bearer.header),
            names::string(&bearer.prefix),
            names::string(&bearer.token.value.name)
        ));
    }
    if let Some(hmac) = &auth.hmac {
        text.push_str(&format!(
            "      hmac: {{ secretEnv: {} }},\n",
            names::string(&hmac.secret.value.name)
        ));
    }
    text.push_str("    },\n");
    text
}

const fn algorithm(algorithm: HmacAlgorithm) -> &'static str {
    match algorithm {
        HmacAlgorithm::Sha1 => "sha1",
        HmacAlgorithm::Sha256 => "sha256",
        HmacAlgorithm::Sha512 => "sha512",
    }
}

const fn encoding(encoding: SignatureEncoding) -> &'static str {
    match encoding {
        SignatureEncoding::Hex => "hex",
        SignatureEncoding::Base64 => "base64",
    }
}

/// One `manual` trigger, lowered to what the emitted table says about it.
struct EmittedManual<'ir> {
    name: &'ir str,
    flow: String,
    session_key: Option<&'ir str>,
}

/// The declared `manual` triggers, in the IR's canonical order.
fn manual(ir: &Ir) -> Vec<EmittedManual<'_>> {
    let Some(section) = ir.triggers.as_ref() else {
        return Vec::new();
    };
    section
        .entries
        .values()
        .filter(|trigger| matches!(trigger.kind, TriggerKind::Manual))
        .map(|trigger| EmittedManual {
            name: trigger.name.value.as_str(),
            flow: trigger.flow.value.to_string(),
            session_key: trigger.session_key.as_ref().map(|key| key.value.as_str()),
        })
        .collect()
}

/// One `http` trigger, lowered to what the emitted table says about it.
struct Emitted<'ir> {
    name: &'ir str,
    flow: String,
    path: String,
    method: &'static str,
    respond: &'static str,
    timeout_ms: Option<u64>,
    reads_body: bool,
    input: Vec<(&'ir str, &'ir str)>,
    session_key: Option<&'ir str>,
    callback: Option<&'ir str>,
    auth: Option<&'ir InboundAuth>,
    callback_auth: Option<&'ir CallbackAuth>,
    callback_allow: Option<Vec<&'ir str>>,
    dedupe_key: Option<&'ir str>,
    dedupe_window_ms: u64,
    /// The declared trigger, for what is emitted the same way on every kind.
    declared: &'ir crate::ir::Trigger,
}

/// The declared `http` triggers, in the IR's canonical order.
fn triggers(ir: &Ir) -> Vec<Emitted<'_>> {
    let Some(section) = ir.triggers.as_ref() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for trigger in section.entries.values() {
        let TriggerKind::Http(http) = &trigger.kind else {
            continue;
        };
        let name = trigger.name.value.as_str();
        let method = method(http);
        found.push(Emitted {
            name,
            flow: trigger.flow.value.to_string(),
            // Grammar 13.3's default path is the trigger's own name.
            path: http
                .path
                .as_ref()
                .map_or_else(|| format!("/triggers/{name}"), |path| path.value.clone()),
            method,
            respond: match http.respond {
                Some(Respond::Sync) => "sync",
                Some(Respond::Async) | None => "async",
            },
            // Grammar 13.3: `60s` is the default of the *effective sync* budget,
            // and an `async` trigger carries none at all (Decision D81).
            timeout_ms: match http.respond {
                Some(Respond::Sync) => Some(
                    http.timeout
                        .as_ref()
                        .map_or(DEFAULT_SYNC_TIMEOUT_MS, |timeout| {
                            policy::milliseconds(&timeout.value)
                        }),
                ),
                _ => None,
            },
            reads_body: method != "GET",
            input: http
                .input
                .as_ref()
                .map(|bindings| {
                    bindings
                        .entries
                        .iter()
                        .map(|binding| (binding.name.value.as_str(), binding.value.value.as_str()))
                        .collect()
                })
                .unwrap_or_default(),
            session_key: trigger.session_key.as_ref().map(|key| key.value.as_str()),
            callback: http
                .callback
                .as_ref()
                .map(|callback| callback.value.as_str()),
            auth: http.auth.as_ref(),
            callback_auth: http.callback_auth.as_ref(),
            callback_allow: http.callback_allow.as_ref().map(|patterns| {
                patterns
                    .iter()
                    .map(|pattern| pattern.value.as_str())
                    .collect()
            }),
            dedupe_key: http.dedupe_key.as_ref().map(|key| key.value.as_str()),
            dedupe_window_ms: window_ms(http.dedupe_window.as_ref()),
            declared: trigger,
        });
    }
    found
}

/// Grammar 13.3's `60s` default for a sync trigger's response budget.
const DEFAULT_SYNC_TIMEOUT_MS: u64 = 60_000;

const fn method(http: &HttpTrigger) -> &'static str {
    match http.method {
        Some(TriggerMethod::Get) => "GET",
        Some(TriggerMethod::Put) => "PUT",
        Some(TriggerMethod::Post) | None => "POST",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    const PROJECT: &str = r#"version: "0.1"

state:
  answer: { type: string, default: "" }

triggers:
  on_request:
    type: http
    flow: flow.ask
    path: /answers
    input:
      question: "payload.body.question"
    session_key: "payload.headers['x-session-id']"
    callback: "payload.body.callback_url"

  quick:
    type: http
    flow: flow.ask
    method: GET
    respond: sync
    timeout: 5s
    input:
      question: "payload.query.q"

  cli:
    type: manual
    flow: flow.ask

flow.ask:
  inputs:
    question: { type: string }
  outputs:
    answer: { type: string }
  nodes:
    reply:
      exec: { command: printf, args: ["an answer"] }
      writes: { stdout: answer }
  edges:
    - { from: start, to: reply }
    - { from: reply, to: end }
"#;

    #[test]
    fn a_project_with_no_triggers_emits_an_empty_table() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.contains("export const httpTriggers: readonly HttpTrigger[] = [\n];\n"));
        assert!(
            emitted.contains("export const manualTriggers: readonly ManualTrigger[] = [\n];\n"),
            "{emitted}"
        );
    }

    /// Every key grammar 13.3 makes a property of the route, and the defaults
    /// for the ones a trigger left out.
    #[test]
    fn each_http_trigger_carries_its_route_its_mode_and_its_bindings() {
        let emitted = module(&ir_of(PROJECT)).contents;
        assert!(emitted.contains("name: \"on_request\","), "{emitted}");
        assert!(emitted.contains("path: \"/answers\","));
        assert!(emitted.contains("method: \"POST\","), "the default method");
        assert!(emitted.contains("respond: \"async\","), "the default mode");
        assert!(emitted.contains("readsBody: true,"));
        assert!(
            emitted.contains(
                "\"question\": runtime.toJson(runtime.evaluate(\"payload.body.question\", bound)),"
            ),
            "{emitted}"
        );
        assert!(emitted.contains(
            "sessionKey: (payload) => text(\"payload.headers['x-session-id']\", roots(payload),"
        ));
        assert!(emitted.contains(
            "callback: (payload) => text(\"payload.body.callback_url\", roots(payload),"
        ));

        // The sync one: its budget is a number of milliseconds, and a `GET`
        // decodes no body (Decision D117).
        assert!(emitted.contains("respond: \"sync\","));
        assert!(emitted.contains("timeoutMs: 5000,"));
        assert!(emitted.contains("method: \"GET\","));
        assert!(emitted.contains("readsBody: false,"));

        // A `manual` trigger contributes no *route* (Decision D64): it appears
        // in the other table, and with no `path:`, `method:` or `input:`.
        let routes = emitted
            .split("export const manualTriggers")
            .next()
            .expect("the route table comes first");
        assert!(!routes.contains("\"cli\""), "{emitted}");
    }

    /// The composition every authentication key is written out in, so the
    /// emitted table can be read for what each one lands as.
    const GUARDED: &str = r#"version: "0.1"

flow.support:
  outputs: {}
  nodes:
    approve:
      human:
        input: {}
        output:
          decision: { enum: [approve, reject] }
  edges:
    - { from: start, to: approve }
    - { from: approve, to: end }

triggers:
  intake:
    type: http
    flow: flow.support
    callback: "payload.body.callback_url"
    auth:
      hmac:
        secret: ${WEBHOOK_SECRET}
        header: X-Hub-Signature-256
        algorithm: sha512
        encoding: base64
        prefix: "sha512="
    callback_auth:
      bearer:
        token: ${CALLBACK_TOKEN}
        header: X-Delivery-Token
        prefix: "Token "
      hmac:
        secret: ${CALLBACK_SECRET}
    callback_allow:
      - "https://hooks.example.com/*"
      - "http://localhost:9000/*"

  minimal:
    type: http
    flow: flow.support
    path: /minimal
    auth:
      bearer:
        token: ${WEBHOOK_TOKEN}
"#;

    /// Every parameter that decides whether a credential verifies reaches the
    /// table, **resolved** (grammar 13.3, PRD resolved q32).
    ///
    /// A verifier is the wrong place to re-derive a default: getting one wrong
    /// there does not fail a build, it accepts the wrong request. So the
    /// undeclared half of `minimal`'s `bearer:` lands as `Authorization` and
    /// `Bearer ` rather than as an absence for the app to fill in.
    #[test]
    fn an_authenticated_trigger_carries_its_scheme_resolved() {
        let emitted = module(&ir_of(GUARDED)).contents;
        assert!(
            emitted.contains(
                "    auth: {\n      scheme: \"hmac\",\n      header: \"X-Hub-Signature-256\",\n      \
                 algorithm: \"sha512\",\n      encoding: \"base64\",\n      prefix: \"sha512=\",\n      \
                 secretEnv: \"WEBHOOK_SECRET\",\n    },\n"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains(
                "    auth: {\n      scheme: \"bearer\",\n      header: \"Authorization\",\n      \
                 prefix: \"Bearer \",\n      tokenEnv: \"WEBHOOK_TOKEN\",\n    },\n"
            ),
            "{emitted}"
        );
    }

    /// Both outbound schemes together are legal, and the allowlist travels
    /// with them (grammar 13.3, PRD resolved q33).
    #[test]
    fn a_signed_delivery_carries_both_schemes_and_its_allowlist() {
        let emitted = module(&ir_of(GUARDED)).contents;
        assert!(
            emitted.contains(
                "    callbackAuth: {\n      bearer: { header: \"X-Delivery-Token\", \
                 prefix: \"Token \", tokenEnv: \"CALLBACK_TOKEN\" },\n      \
                 hmac: { secretEnv: \"CALLBACK_SECRET\" },\n    },\n"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains(
                "    callbackAllow: [\n      \"https://hooks.example.com/*\",\n      \
                 \"http://localhost:9000/*\",\n    ],\n"
            ),
            "{emitted}"
        );
    }

    /// A credential reaches the table as the **variable name** it was written
    /// as, never as a value (grammar 4.3, PRD resolved q15).
    ///
    /// `build` resolves nothing, so the emitted project is committable and the
    /// same artifact runs in two deployments holding different secrets. What
    /// makes that safe is [`super::env`]'s walk: the launch check refuses a
    /// deployment missing one of these names.
    #[test]
    fn a_credential_reaches_the_table_as_a_name_and_never_as_a_value() {
        let emitted = module(&ir_of(GUARDED)).contents;
        for reference in [
            "${WEBHOOK_SECRET}",
            "${CALLBACK_TOKEN}",
            "${CALLBACK_SECRET}",
        ] {
            assert!(
                !emitted.contains(reference),
                "the table interpolates nothing: {emitted}"
            );
        }
        assert!(
            emitted.contains("secretEnv: \"WEBHOOK_SECRET\""),
            "{emitted}"
        );
    }

    /// A trigger declaring none of the three carries none of the three.
    #[test]
    fn an_open_trigger_carries_no_authentication_keys() {
        let emitted = module(&ir_of(PROJECT)).contents;
        for key in ["auth:", "callbackAuth:", "callbackAllow:"] {
            let routes = emitted
                .split("export const httpTriggers")
                .nth(1)
                .expect("the route table is emitted");
            assert!(
                !routes.contains(key),
                "an open trigger emits no `{key}`: {emitted}"
            );
        }
    }

    /// A sync trigger that declares no `timeout:` takes grammar 13.3's default.
    #[test]
    fn a_sync_trigger_with_no_timeout_takes_the_declared_default() {
        let source = PROJECT.replace("    timeout: 5s\n", "");
        let emitted = module(&ir_of(&source)).contents;
        assert!(emitted.contains("timeoutMs: 60000,"), "{emitted}");
    }

    /// A `manual` trigger that declares no `session_key:` carries no remap: the
    /// key's default is `--session` itself (grammar 13.2).
    #[test]
    fn a_manual_trigger_with_no_session_key_carries_no_remap() {
        let emitted = module(&ir_of(PROJECT)).contents;
        assert!(
            emitted.contains(
                "export const manualTriggers: readonly ManualTrigger[] = [\n  \
                 {\n    name: \"cli\",\n    flow: \"flow.ask\",\n  },\n];\n"
            ),
            "{emitted}"
        );
    }

    /// …and one that declares a remap emits it, over the manual payload whose
    /// single member is the CLI's `--session` (grammar 13.2).
    #[test]
    fn a_manual_trigger_emits_the_session_key_it_declares() {
        let source = PROJECT.replace(
            "  cli:\n    type: manual\n    flow: flow.ask\n",
            "  cli:\n    type: manual\n    flow: flow.ask\n    session_key: \"'tenant-' + payload.session\"\n",
        );
        let emitted = module(&ir_of(&source)).contents;
        assert!(
            emitted.contains(
                "    sessionKey: (session) => text(\"'tenant-' + payload.session\", \
                 manualRoots(session), \"`session_key` of the trigger `cli`\"),\n"
            ),
            "{emitted}"
        );
    }

    /// The intake every trigger kind but `manual` may declare (grammar 13.1,
    /// 13.3, 13.5, PRD resolved q77): an empty `concurrency:` block is one
    /// key-less group of one that queues, and a dedupe key held for the `24h`
    /// window both grammar sections default it to.
    #[test]
    fn an_intake_block_left_to_its_defaults_emits_one_queueing_group_of_one() {
        let source = r#"version: "0.1"

triggers:
  on_request:
    type: http
    flow: flow.work
    dedupe_key: "payload.headers['x-delivery']"
    input: { tag: "payload.body.tag" }
    concurrency: {}
  nightly:
    type: schedule
    cron: "0 3 * * *"
    flow: flow.work
    input: { tag: "payload.scheduled_at" }
    concurrency: {}
  feed_in:
    type: event
    source: feed
    flow: flow.work
    input: { tag: "payload.id" }
    concurrency: {}

flow.work:
  inputs:
    tag: { type: string }
  outputs: {}
  nodes:
    work:
      exec: { command: "true" }
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
"#;
        let emitted = module(&ir_of(source)).contents;
        let defaults =
            "    concurrency: {\n      limit: 1,\n      onConflict: \"queue\",\n    },\n";
        assert_eq!(
            emitted.matches(defaults).count(),
            3,
            "each of the three triggers carries the defaulted block: {emitted}"
        );
        assert_eq!(
            emitted.matches("    dedupeWindowMs: 86400000,\n").count(),
            2,
            "the http key and the event's `payload.id` key are each held 24h: {emitted}"
        );
        assert!(!emitted.contains("debounceMs:"), "{emitted}");
        assert!(!emitted.contains("maxRunning:"), "{emitted}");
    }

    /// Every key of the block, spelled, and a `dedupe_window:` of its own on
    /// each kind that takes one.
    #[test]
    fn an_intake_block_carries_every_key_it_declares() {
        let source = r#"version: "0.1"

triggers:
  on_request:
    type: http
    flow: flow.work
    dedupe_key: "payload.headers['x-delivery']"
    dedupe_window: 90s
    input: { tag: "payload.body.tag" }
    concurrency:
      key: "payload.body.repo"
      limit: 2
      on_conflict: supersede
      debounce: 1500ms
      max_running: 3
  feed_in:
    type: event
    source: feed
    flow: flow.work
    dedupe_window: 2h
    input: { tag: "payload.id" }
    concurrency:
      on_conflict: drop

flow.work:
  inputs:
    tag: { type: string }
  outputs: {}
  nodes:
    work:
      exec: { command: "true" }
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
"#;
        let emitted = module(&ir_of(source)).contents;
        assert!(
            emitted.contains(
                "    dedupeKey: (payload) => text(\"payload.headers['x-delivery']\", roots(payload), \
                 \"`dedupe_key` of the trigger `on_request`\"),\n    dedupeWindowMs: 90000,\n"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains(
                "    concurrency: {\n      key: (payload) => text(\"payload.body.repo\", roots(payload), \
                 \"`concurrency.key` of the trigger `on_request`\"),\n      limit: 2,\n      \
                 onConflict: \"supersede\",\n      debounceMs: 1500,\n      maxRunning: 3,\n    },\n"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains("    dedupeWindowMs: 7200000,\n"),
            "{emitted}"
        );
        assert!(
            emitted.contains(
                "    concurrency: {\n      limit: 1,\n      onConflict: \"drop\",\n    },\n"
            ),
            "{emitted}"
        );
    }
}
