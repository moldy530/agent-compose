//
// The project's own command line: what `agent-compose run` and
// `agent-compose serve` launch (PRD 5.11, grammar 13.2).
//
// The compiler builds this directory and then runs it — `bun src/index.ts run
// flow.review --input goal=…`, or `node src/index.ts` under the fallback — which
// is the same launch surface the emitted `README.md` documents. Nothing about
// invoking a compiled graph lives in the Rust binary: it validates, it builds,
// it checks the environment, and it starts this.
//
// ```text
// src/index.ts run <flow> [--input k=v]... [--session <key>] [--format human|json]
// src/index.ts serve [--host <host>] [--port <port>]
// ```
//
// # What `run` prints
//
// **stdout carries the answer, stderr carries the report.** A flow's `outputs:`
// is a declared schema, so the answer is one JSON object — the only shape that
// survives the schema's own types — and a caller can read it without parsing
// prose. Under `--format json` the answer grows into the whole record: the
// execution id, the outputs, and the routing trace of PRD 5.3, so a machine
// reading one document gets everything a human reads across two.
//
// The trace is also written to a file under the project's data directory and its
// path is named on stderr, because a run of any size produces more of it than a
// terminal is useful for.
//
// # Why `--input` values are coerced
//
// A CLI argument is text and a flow's `inputs:` is typed (grammar 13.2: "a value
// that does not fit the declared type fails the run naming the field"). So each
// value is read as the field's declared kind — an integer field takes `3`, a
// boolean takes `true`, an object or array takes JSON — and a value that cannot
// be read that way fails the run naming the field rather than reaching Zod as a
// string and failing about a type the author never wrote.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

import { type CompiledFlow, type FlowRun, flows, runFlow } from "./graph.ts";
import type * as runtime from "./runtime.ts";
import { dataRoot } from "./stores.ts";

/** What a `--format` selects. */
type Format = "human" | "json";

/** A usage error: the command could not run, which is the caller's to fix. */
class UsageError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "UsageError";
  }
}

/**
 * Run the command line, flush what it wrote, and end the process.
 *
 * The exit is explicit rather than left to the event loop draining: a provider
 * connection `fetch` kept alive outlives the work that opened it, and a run that
 * had already printed its answer would sit there until the socket timed out. The
 * streams are flushed first, because `process.exit` truncates a pipe that is
 * still holding bytes — and stdout is a pipe whenever a caller is reading the
 * outputs, which is the case this exists for.
 *
 * `serve` never comes back through here: its app owns the process.
 */
export async function runMain(argv: readonly string[]): Promise<void> {
  const code = await main(argv);
  await flush();
  process.exit(code);
}

/** Wait for what has been written to leave the process. */
async function flush(): Promise<void> {
  for (const stream of [process.stdout, process.stderr]) {
    if (stream.writableLength === 0) continue;
    await new Promise<void>((resolve) => {
      stream.write("", () => resolve());
    });
  }
}

/**
 * Run the project's own command line, and answer with the exit code.
 *
 * `0` is a clean run, `1` is a run that produced no answer, and `2` is a command
 * that could not run at all — the same three meanings `agent-compose` itself
 * gives them, so a caller reads one table rather than two.
 */
export async function main(argv: readonly string[]): Promise<number> {
  const [verb, ...rest] = argv;
  try {
    if (verb === "run") return await run(rest);
    if (verb === "serve") return await serveVerb(rest);
    throw new UsageError(
      `\`${verb ?? ""}\` is not a verb of this project: it takes \`run\` or \`serve\` (see README.md)`,
    );
  } catch (error) {
    if (error instanceof UsageError) {
      process.stderr.write(`error: ${error.message}\n`);
      return 2;
    }
    throw error;
  }
}

/** `run <flow> [--input k=v]... [--session <key>] [--format human|json]`. */
async function run(argv: readonly string[]): Promise<number> {
  const [address, ...rest] = argv;
  if (address === undefined) {
    throw new UsageError("`run` takes a flow address: `run flow.<name> [--input k=v]...`");
  }
  const flow = flows[address];
  if (flow === undefined) {
    throw new UsageError(
      `\`${address}\` is not a flow of this composition: ${Object.keys(flows).join(", ")}`,
    );
  }

  const options = parse(rest, ["input"]);
  const inputs = bindInputs(flow, options.repeated["input"] ?? []);
  const session = options.single["session"] ?? "";
  const format = formatOf(options.single["format"]);

  // Minted here rather than left to `runFlow`, because this command needs it on
  // both of its ways out: it is what makes the trace file's name unique (see
  // [`writeTrace`]), and a failed run has no `FlowRun` to read one back off.
  const execution = `exec_${globalThis.crypto.randomUUID()}`;

  let produced: FlowRun;
  try {
    produced = await runFlow(address, inputs, { executionId: execution, sessionKey: session });
  } catch (error) {
    const trace = (error as { trace?: readonly runtime.TraceEntry[] }).trace ?? [];
    const written = writeTrace(address, execution, trace);
    if (format === "json") {
      process.stdout.write(
        `${JSON.stringify({ flow: address, status: "failed", error: describe(error), trace }, null, 2)}\n`,
      );
    } else {
      process.stderr.write(render(trace));
      process.stderr.write(`\n${describe(error)}\n`);
      for (let cause: unknown = (error as { cause?: unknown }).cause; cause !== undefined; ) {
        process.stderr.write(`  cause: ${describe(cause)}\n`);
        cause = (cause as { cause?: unknown }).cause;
      }
      if (written !== undefined) process.stderr.write(`\ntrace: ${written}\n`);
    }
    return 1;
  }

  const written = writeTrace(address, execution, produced.trace);
  if (format === "json") {
    process.stdout.write(
      `${JSON.stringify(
        {
          flow: address,
          status: "completed",
          outputs: produced.outputs,
          trace: produced.trace,
          ...(written === undefined ? {} : { trace_path: written }),
        },
        null,
        2,
      )}\n`,
    );
    return 0;
  }
  process.stdout.write(`${JSON.stringify(produced.outputs, null, 2)}\n`);
  process.stderr.write(render(produced.trace));
  if (written !== undefined) process.stderr.write(`\ntrace: ${written}\n`);
  return 0;
}

/** `serve [--host <host>] [--port <port>]`. */
async function serveVerb(argv: readonly string[]): Promise<number> {
  const options = parse(argv, []);
  const port = options.single["port"];
  if (port !== undefined && !/^[0-9]+$/.test(port)) {
    throw new UsageError(`\`--port ${port}\` is not a port number`);
  }
  // Loaded here rather than imported at the top, so a `run` never pays for the
  // HTTP framework and a project used as a library never loads it at all.
  const { serve } = await import("./serve.ts");
  const { httpTriggers } = await import("./triggers.ts");
  if (httpTriggers.length === 0) {
    throw new UsageError(
      "this composition declares no `http` triggers, so the generated app exposes no routes: declare one in `triggers:` (grammar 13.3)",
    );
  }
  const host = options.single["host"] ?? "127.0.0.1";
  try {
    await serve({
      host,
      ...(port === undefined ? {} : { port: Number(port) }),
    });
  } catch (error) {
    // Everything that keeps the app from *starting* is the same kind of failure
    // as a bad flag: the address is taken, the port needs a privilege this
    // process does not have, the host does not resolve. Answering with a
    // framework stack trace and `1` would report "a run produced no answer",
    // which is the one thing that did not happen — nothing ran. So it is a `2`
    // with a sentence, like every other command that could not be run.
    //
    // One of them is not about the address at all, and saying it was would send
    // a reader to look at a port that is fine. Routes are registered when the
    // app is made ready, which `listen` does, so a **route collision** arrives
    // here too — and it is the composition's, not the machine's: two `http`
    // triggers whose paths the compiler could not tell apart (a parameter's
    // *name* differs, and grammar 13.3 has the router read parameters
    // unexamined), or one claiming a route the app mounts for itself. The
    // compiler refuses every collision it can decide; this is what the router
    // decides, reported as what it is.
    if ((error as { code?: unknown } | null)?.code === "FST_ERR_DUPLICATED_ROUTE") {
      throw new UsageError(
        `the app could not mount its routes: ${describe(error)}. Two routes of this composition are one route to the router — an \`http\` trigger's \`path:\` and \`method:\`, or one of the app's own \`GET /executions/:id\` and \`POST /executions/:id/resume\` — so give one of them a path the other cannot be read as (grammar 13.3)`,
      );
    }
    throw new UsageError(
      `the app could not listen on ${host}:${port ?? 0}: ${describe(error)}`,
    );
  }
  // The app owns the process from here: `serve` never returns on its own, and
  // the signal handlers it installed are what end it.
  await new Promise<void>(() => {});
  return 0;
}

/** The `--input k=v` arguments, read as the flow's declared field types. */
function bindInputs(flow: CompiledFlow, given: readonly string[]): Record<string, unknown> {
  const inputs: Record<string, unknown> = {};
  for (const argument of given) {
    const split = argument.indexOf("=");
    if (split <= 0) {
      throw new UsageError(`\`--input ${argument}\` is not \`field=value\``);
    }
    const field = argument.slice(0, split);
    const text = argument.slice(split + 1);
    if (!flow.inputs.includes(field)) {
      throw new UsageError(
        flow.inputs.length === 0
          ? `\`${flow.address}\` declares no \`inputs:\`, so it takes no \`--input ${field}=…\``
          : `\`${field}\` is not an input of \`${flow.address}\`: it declares ${flow.inputs.join(", ")}`,
      );
    }
    inputs[field] = coerce(flow, field, text);
  }
  return inputs;
}

/** One `--input` value, as the kind the field declares (grammar 13.2). */
function coerce(flow: CompiledFlow, field: string, text: string): unknown {
  const kind = flow.inputKinds[field] ?? "string";
  if (kind === "string") return text;
  if (kind === "boolean") {
    if (text === "true") return true;
    if (text === "false") return false;
    throw new UsageError(`\`--input ${field}=${text}\`: \`${field}\` is a boolean, so it takes \`true\` or \`false\``);
  }
  if (kind === "integer" || kind === "number") {
    const value = Number(text);
    if (!Number.isFinite(value) || (kind === "integer" && !Number.isInteger(value))) {
      throw new UsageError(`\`--input ${field}=${text}\`: \`${field}\` is ${kind === "integer" ? "an integer" : "a number"}`);
    }
    return value;
  }
  try {
    return JSON.parse(text) as unknown;
  } catch {
    throw new UsageError(
      `\`--input ${field}=${text}\`: \`${field}\` is a structured field, so its value is read as JSON`,
    );
  }
}

/** The flags this command line takes: `--name value`, some repeatable. */
function parse(
  argv: readonly string[],
  repeatable: readonly string[],
): { single: Record<string, string>; repeated: Record<string, string[]> } {
  const single: Record<string, string> = {};
  const repeated: Record<string, string[]> = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]!;
    if (!argument.startsWith("--")) {
      throw new UsageError(`\`${argument}\` is not an option: every argument after the verb is \`--name value\``);
    }
    const name = argument.slice(2);
    const value = argv[index + 1];
    if (value === undefined) throw new UsageError(`\`--${name}\` takes a value`);
    index += 1;
    if (repeatable.includes(name)) {
      (repeated[name] ??= []).push(value);
    } else {
      single[name] = value;
    }
  }
  return { single, repeated };
}

function formatOf(given: string | undefined): Format {
  if (given === undefined || given === "human") return "human";
  if (given === "json") return "json";
  throw new UsageError(`\`--format ${given}\` is not one of \`human\`, \`json\``);
}

/**
 * Write the run's whole trace beside the project's data, and answer where.
 *
 * A trace is the routing record PRD 5.3 asks for and it grows with the run, so
 * the terminal gets a summary and the file gets everything.
 *
 * The name is the flow's and the **execution id**, not a timestamp: two runs of
 * one flow started together — which is what a shell loop and a CI matrix both
 * do — land in the same millisecond often enough that a `Date.now()` name is a
 * run silently overwriting another's record while both print the same path. The
 * id is unique per execution by construction, and it is also the one thing that
 * ties the file to the run that wrote it.
 */
function writeTrace(
  address: string,
  execution: string,
  trace: readonly runtime.TraceEntry[],
): string | undefined {
  // A run that failed before it started made no routing decisions, and an empty
  // file named as a trace would be a file a reader opens for nothing.
  if (trace.length === 0) return undefined;
  try {
    const directory = path.join(dataRoot(), "traces");
    fs.mkdirSync(directory, { recursive: true });
    const file = path.join(
      directory,
      `${address.replace(/[^A-Za-z0-9_.-]/g, "_")}-${execution.replace(/[^A-Za-z0-9_.-]/g, "_")}.json`,
    );
    fs.writeFileSync(file, `${JSON.stringify(trace, null, 1)}\n`);
    return file;
  } catch {
    // A data directory that cannot be written is not a reason to lose a run that
    // otherwise succeeded; stdout still carries the answer.
    return undefined;
  }
}

/** The human report: one line per node, plus what its model and stores did. */
function render(trace: readonly runtime.TraceEntry[]): string {
  let text = "";
  for (const entry of trace) {
    text += `step ${entry.step}  ${entry.flow} \`${entry.node}\`  ${entry.outcome} (${entry.attempts} attempt(s))\n`;
    for (const call of entry.models ?? []) {
      // PRD 5.9's own phrasing: failover is data a reader can see, not
      // behaviour they have to infer from a provider's own logs.
      const served =
        call.servedBy === undefined
          ? // Nothing answered it: the ladder ran out, and the member whose
            // refusal ended it is what stands in for "served by".
            `refused by ${call.refused?.model ?? "every member"}`
          : call.fallback === 0
            ? `served by ${call.servedBy}`
            : `served by ${call.servedBy}, fallback #${call.fallback}`;
      const after =
        call.failovers.length === 0
          ? ""
          : ` after ${call.failovers.map((one) => `${one.model}: ${one.condition}`).join(", ")}`;
      text += `    model ${call.model} — ${served}${after}\n`;
    }
    for (const store of entry.stores ?? []) {
      const key = store.key === undefined ? "" : ` key=${JSON.stringify(store.key)}`;
      const deduped = store.deduped === true ? " (deduped)" : "";
      text += `    store ${store.store} ${store.op}${key} [${store.effect}, ${store.via}]${deduped}\n`;
    }
    for (const decision of entry.routing?.edges ?? []) {
      const guard = decision.when === undefined ? (decision.else === true ? "else" : "always") : decision.when;
      text += `    edge → ${decision.to}  ${decision.taken ? "taken" : "untaken"}  ${guard}\n`;
    }
    if (entry.error !== undefined) text += `    error: ${entry.error}\n`;
  }
  return text;
}

function describe(error: unknown): string {
  return error instanceof Error ? `${error.name}: ${error.message}` : String(error);
}
