//
// The project's own command line: what `agent-compose run`,
// `agent-compose resume` and `agent-compose serve` launch (PRD 5.11,
// grammar 13.2).
//
// The compiler builds this directory and then runs it — `bun src/index.ts run
// flow.review --input goal=…`, or `node src/index.ts` under the fallback — which
// is the same launch surface the emitted `README.md` documents. Nothing about
// invoking a compiled graph lives in the Rust binary: it validates, it builds,
// it checks the environment, and it starts this.
//
// ```text
// src/index.ts run <flow> [--input k=v]... [--session <key>] [--format human|json]
// src/index.ts resume <execution> [--format human|json]
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
// terminal is useful for. The file is a whole `runtime.TraceDocument` rather than
// a bare list of entries: it is the one surface that arrives without the record
// around it, so it carries its own `trace_version` (`docs/trace.md`), the flow,
// the execution id and how the run ended.
//
// Both machine surfaces carry that version — the JSON record beside its `trace`,
// the file at the head of its envelope — so a reader pins one number and knows
// which fields it may rely on. The **human** report is not one of them: it is a
// summary written for a terminal, and `docs/trace.md` says outright that nothing
// should be parsed out of it.
//
// # Carrying on an execution the machine lost
//
// Every invocation is journaled as it runs (PRD resolved q26-q29,
// `docs/durability.md`), and `resume <execution>` re-runs one from its entry
// with every recorded effect **consumed** rather than re-issued: the model
// answers it got, the results its tools produced, what its stores read, and what
// a person answered. Only the frontier — the first effect the journal does not
// hold — reaches the network.
//
// It takes **no `--input` and no `--session`**. The invocation a resume replays
// is the one the lifecycle row recorded, and a second set of inputs would be one
// execution's record replayed into another execution's run, which is the
// divergence resolved q29 refuses. Everything else about it is `run`: the same
// prompt loop for a pause it re-parks at, the same trace file, the same four
// exit codes.
//
// The four ways it refuses each name what a reader has to look at rather than
// what went wrong internally (PRD G3): no journal at all, an id the journal does
// not hold, an execution that has already ended, and a flow this build no longer
// declares. And a journal whose record does not describe **this** composition
// fails the run naming the divergent step, rather than re-executing an effect
// the record claims to hold.
//
// # Why `--input` values are coerced
//
// A CLI argument is text and a flow's `inputs:` is typed (grammar 13.2: "a value
// that does not fit the declared type fails the run naming the field"). So each
// value is read as the field's declared kind — an integer field takes `3`, a
// boolean takes `true`, an object or array takes JSON — and a value that cannot
// be read that way fails the run naming the field rather than reaching Zod as a
// string and failing about a type the author never wrote.
//
// # A run that stops at a `human` node
//
// A pause is a question, and this command answers it **where there is somebody
// to ask**: standard input. A `run` whose stdin is a terminal renders each pause
// it reaches — the wait id, the flow and node, the `input:` the human is shown,
// the shape their answer has to fit, and the deadline where the node declares one
// — reads one line of JSON back, holds it to the node's `output:` exactly as the
// resume route does, and carries on in the same process (grammar 8.7). The
// prompts go to **stderr**, because stdout is the run's answer.
//
// Everything that is not the delivery is unchanged, and deliberately so: it is
// the same wait board, the same schema check, the same refusal sentences, and the
// same trace record a resumed pause leaves. A `timeout:` budget keeps running
// while the terminal waits, so a wait that expires mid-prompt routes through
// `on_timeout:` exactly as it would under `serve` — the prompt is withdrawn
// saying so, and the next pause is asked.
//
// A run whose stdin is **not** a terminal has nobody to ask, so it does what it
// has always done: it reports what it reached and exits `3` — its own code,
// beside `1` for a run that produced no answer and `2` for a command that could
// not be run. The trace document is still written, with `status: "interrupted"`;
// the entry of the node it stopped at carries the pause, and `agent-compose
// serve` is where the question gets answered instead.
//
// `AGENT_COMPOSE_INTERACTIVE` decides it where a terminal cannot. `1` prompts
// whatever stdin is, which is how a pause is answered from a script or a test
// harness — one line of JSON per prompt, and each prompt names the pause it
// belongs to; `0` never prompts, which is how a run under a terminal is kept to
// the exit-`3` behaviour a supervisor may be reading. Unset, stdin's own `isTTY`
// decides. A value that is neither is a command that could not be run (exit
// `2`), rather than a setting nobody read.
//
// **Standard input ending withdraws the surface.** A script that answered fewer
// pauses than the run reached leaves the run with nothing that can answer the
// rest, which is the same shape as a run that never had a surface at all — so it
// ends exactly there: `status: "interrupted"`, exit `3`, the pause on the entry
// of the node it stopped at.
//
// Grammar 13.2 puts three failures in one sentence — "an unknown argument name,
// a missing REQUIRED field, or a value that does not fit the declared type fails
// the run naming the field" — so all three are one kind of failure here: the
// arguments are held to the flow's own schema **before** the run starts, and a
// set that does not fit is a command that could not run (exit `2`) rather than a
// run that produced no answer.
//
// # `--session` and the remap a `manual` trigger may declare
//
// `--session <key>` is the manual payload's one member (grammar 13.2). A
// declared `manual` trigger naming this flow may carry a `session_key:`, and
// that expression — over that one-member payload — is what turns the argument
// into the identity a `scope: session` store partitions by (grammar 11.3). Left
// undeclared it is `"payload.session"`, the argument itself.
//
// A flow that reaches a `scope: session` store cannot run without one, and
// grammar 11.3 says where that is decided: "supplying the value is a run-time
// requirement (`--session`), checked at run start like env-ref presence (§4.3)".
// So it is decided here, beside the arguments — an invocation missing it is a
// command that could not run (exit `2`), not a run that produced no answer.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";

import {
  CallbackRetryError,
  insisting,
  shipTrace,
  sinkAuth,
  sinkConfigured,
  workDelivery,
} from "./delivery.ts";
import { type CompiledFlow, type FlowRun, flows, runFlow, sessionRefusal } from "./graph.ts";
import {
  TRACE_VERSION,
  closeHumanWaits,
  deliverHumanAnswer,
  deliveriesOf,
  divergenceOf,
  executionReport,
  humanWaitEnded,
  humanWaits,
  intendDelivery,
  interruptOf,
  journalExists,
  journalPath,
  journaledExecution,
  openExecutions,
  watchHumanPauses,
} from "./runtime.ts";
import type * as runtime from "./runtime.ts";
import { dataRoot } from "./stores.ts";
import { httpTriggers, manualTriggers } from "./triggers.ts";

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
 * `0` is a clean run, `1` is a run that produced no answer, `2` is a command
 * that could not run at all, and `3` is a run that stopped at a `human` pause
 * with **nobody to ask** — the same four meanings `agent-compose` itself gives
 * them, so a caller reads one table rather than two.
 *
 * `3` is a code of its own rather than a shade of `1` because the three ask for
 * different things. `2` is an invocation to fix and `1` is a run to look into;
 * this is neither — the run did everything it was asked to and is holding a
 * question. What closes a pause is a person answering it, and there are two
 * surfaces they can (grammar 8.7, PRD §9.21): the terminal this command prompts
 * at when standard input is one, where the run carries on and ends `0` like any
 * other, and the app's `POST /executions/:id/resume`. `3` is what is left when
 * neither was available — standard input was not a terminal,
 * `AGENT_COMPOSE_INTERACTIVE=0` said not to ask, or the terminal went away
 * mid-run — and it points at `serve`, which is where the same question can be
 * asked of a process that has a surface for the answer. A supervisor that
 * retried `1` would re-run a graph whose effects have already happened, and one
 * that reported `2` would send someone to look at the command line.
 */
export async function main(argv: readonly string[]): Promise<number> {
  const [verb, ...rest] = argv;
  try {
    if (verb === "run") return await run(rest);
    if (verb === "resume") return await resumeVerb(rest);
    if (verb === "serve") return await serveVerb(rest);
    throw new UsageError(
      `\`${verb ?? ""}\` is not a verb of this project: it takes \`run\`, \`resume\` or \`serve\` (see README.md)`,
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

  const options = parse("run", rest, {
    input: "repeatable",
    session: "single",
    format: "single",
  });
  const inputs = bindInputs(flow, options.repeated["input"] ?? []);
  const session = sessionOf(flow, options.single["session"] ?? "");
  const format = formatOf(options.single["format"]);
  requireSession(address, flow, session);

  // Minted here rather than left to `runFlow`, because this command needs it on
  // both of its ways out: it is what makes the trace file's name unique (see
  // [`writeTrace`]), and a failed run has no `FlowRun` to read one back off.
  const execution = `exec_${globalThis.crypto.randomUUID()}`;
  return await execute({ address, inputs, session, format, execution, resuming: false });
}

/**
 * `resume <execution-id> [--format human|json]` — carry on an execution this
 * project's journal holds open (PRD resolved q28, `docs/durability.md` §6).
 *
 * A `run` that crashed is not re-run: the graph is re-executed from its entry
 * with every recorded effect **consumed** — the model answers it got, the
 * results its tools produced, what its stores read and what a person answered —
 * and only the frontier, the first effect the journal does not hold, reaches
 * the network. So a resumed execution costs what is left of it and not what it
 * had already paid for.
 *
 * Everything about the invocation comes off the lifecycle row: the flow, the
 * inputs as the flow's `inputs:` parsed them, and the session identity. There
 * are no `--input` or `--session` flags here, and that is a rule rather than an
 * omission — a resume that took different inputs would be replaying one
 * execution's record into another execution's run, which is the divergence
 * resolved q29 refuses.
 *
 * It composes with the interactive surface exactly as `run` does: a resumed
 * execution whose wait is **not** in the journal re-parks under its original
 * wait id, and a terminal (or `AGENT_COMPOSE_INTERACTIVE=1`) answers it there.
 */
async function resumeVerb(argv: readonly string[]): Promise<number> {
  const [execution, ...rest] = argv;
  if (execution === undefined) {
    throw new UsageError(
      "`resume` takes an execution id: `resume <execution-id>` — a `run` prints it on stderr as `execution: exec_…`, and `serve` answers it as `execution_id`",
    );
  }
  const options = parse("resume", rest, { format: "single" });
  const format = formatOf(options.single["format"]);

  if (!journalExists()) {
    throw new UsageError(
      `this project has never journaled an execution, so there is none to resume: \`${journalPath()}\` does not exist, and it is written by the \`run\` or \`serve\` that starts an execution`,
    );
  }
  const row = await journaledExecution(execution);
  if (row === undefined) {
    const open = await openExecutions();
    throw new UsageError(
      `\`${execution}\` is not an execution in \`${journalPath()}\`: ${
        open.length === 0
          ? "it holds none open"
          : `the executions it holds open are ${open.map((held) => `\`${held.id}\``).join(", ")}`
      }`,
    );
  }
  if (row.status !== "open") {
    throw new UsageError(
      `\`${execution}\` has already ${row.status}${
        row.endedAt === undefined ? "" : ` (${row.endedAt})`
      }, so there is nothing to resume: the journal keeps the record of an execution that ended, and re-running it would re-issue effects that record says already happened${
        row.error === undefined ? "" : ` — it ended with ${row.error}`
      }`,
    );
  }
  const flow = flows[row.flow];
  if (flow === undefined) {
    throw new UsageError(
      `\`${execution}\` was running \`${row.flow}\`, which this build does not declare: it names ${Object.keys(flows).join(", ")}. Build the composition that started it, or delete \`${journalPath()}\``,
    );
  }
  requireSession(row.flow, flow, row.sessionKey);
  return await execute({
    address: row.flow,
    inputs: row.inputs,
    session: row.sessionKey,
    format,
    execution,
    resuming: true,
    trigger: row.trigger,
    // What this execution's request asked to be told when it ends, off the
    // lifecycle row that recorded it (`docs/durability.md` §3.5). A `run` never
    // has one; a resume of an `http` execution may, and finishing one here is
    // the one place outside `serve` where such a row closes.
    ...(row.callback === undefined ? {} : { callback: row.callback }),
  });
}

/** One invocation of a flow, whichever verb asked for it. */
interface Job {
  readonly address: string;
  readonly inputs: Record<string, unknown>;
  readonly session: string;
  readonly format: Format;
  readonly execution: string;
  /** Whether the journal's record is consumed rather than only written. */
  readonly resuming: boolean;
  /** What started it, for a fresh execution's lifecycle row. */
  readonly trigger?: string;
  /** Where its `settled` webhook goes, for a `resume` that closes one. */
  readonly callback?: string;
}

/**
 * Run one flow and report it — the body `run` and `resume` share.
 *
 * They differ in where the invocation came from and in nothing else: the same
 * prompt loop, the same trace file, the same four exit codes. Sharing it is
 * what makes "a resumed execution that reaches an unanswered `human` wait
 * prompts at the terminal exactly as an interactive `run` does" true by
 * construction rather than by two implementations agreeing.
 */
async function execute(job: Job): Promise<number> {
  // The trace export this run journals, if it journals one. Held in a box rather
  // than returned, because it is written by a hook **inside** the run and read
  // after the run has reported: PRD resolved q50 puts the sink "wherever
  // executions settle, `run` included", and never in front of the answer.
  const exported: { record?: runtime.DeliveryRecord } = {};
  const code = await executing(job, exported);
  await shipped(exported.record);
  return code;
}

/**
 * Send the trace this run journaled, as far as a command can.
 *
 * `false` — the command posture of `workDelivery`: every offset already due is
 * attempted, which with `docs/durability.md` §3.7's schedule is the first one,
 * and the rest is left on the row for a `serve` start to pick up. A command that
 * waited out fifteen minutes of retries would hold a terminal open over a
 * courtesy; one that sent nothing would make resolved q50's `run` clause false.
 *
 * After the answer is on stdout and never before it, and its own failure is its
 * own: a collector that is down is not this run's outcome and does not touch its
 * exit code.
 */
async function shipped(record: runtime.DeliveryRecord | undefined): Promise<void> {
  if (record === undefined) return;
  try {
    await workDelivery(record, sinkAuth(), false);
  } catch (error) {
    process.stderr.write(
      `\`${record.id}\` could not be delivered and stays in the journal: ${describe(error)}\n`,
    );
  }
}

/** [`execute`]'s body: everything up to and including the run's report. */
async function executing(
  job: Job,
  exported: { record?: runtime.DeliveryRecord },
): Promise<number> {
  const { address, format, execution } = job;
  const asking = interactively();

  // Where a reader finds the id: it is what `agent-compose resume` takes, and a
  // run the machine loses has no other way to have said it. Written **first**,
  // before anything can fail, so a run killed mid-flight has still printed it.
  //
  // Under `--format json` it is not written at all, and that is the format's
  // own rule rather than an exception to this one: the whole answer is the
  // document on stdout, `execution_id` is a field of it on both of a run's ways
  // out, and a line on stderr would be a second surface carrying the same value
  // — the thing that format exists not to have. A `json` run the machine loses
  // before it answers is found through `resume`'s own listing instead, which
  // names every execution the journal holds open.
  if (format === "human") process.stderr.write(`execution: ${execution}\n`);

  // Started rather than awaited, because a run that pauses is one this command
  // may have to *answer* while it is still going: the prompt loop reads the same
  // wait board the graph is parked on (grammar 8.7), so the two run side by side
  // in one process. A run with no `human` node in it never prompts and never
  // reads stdin — [`answerPauses`] attaches to it at the first question.
  const running = runFlow(address, job.inputs, {
    executionId: execution,
    sessionKey: job.session,
    resumable: asking,
    ...(job.trigger === undefined ? {} : { trigger: job.trigger }),
    ...(job.resuming ? { resume: true } : {}),
    // What this run owes at the moment its lifecycle row closes: the webhook a
    // `serve`-started execution finished here still owes ([`owed`]), and the
    // trace every settled execution ships under a target that declares a
    // `trace_sink:` (grammar 14.5). The hook is attached only where there is
    // something to owe, so a run under neither pays for neither.
    ...(job.callback === undefined && !sinkConfigured()
      ? {}
      : {
          closing: (produced: FlowRun | undefined, error: unknown) =>
            settled(job, exported, produced, error),
        }),
  });
  const prompting = asking
    ? answerPauses(execution, settling(running), {
        input: process.stdin,
        output: process.stderr,
      }).catch((error: unknown) => {
        // The prompt loop is the only thing that can answer this run, so a
        // failure inside it is a run with no answer surface rather than a
        // command that should keep waiting for one: the pauses become the
        // interrupt they already are, and the run ends the way a non-interactive
        // one does (see `runtime.closeHumanWaits`).
        process.stderr.write(
          `\nthis run stopped being able to ask: ${describe(error)}\n`,
        );
        closeHumanWaits(execution);
      })
    : undefined;

  let produced: FlowRun;
  try {
    produced = await running;
  } catch (error) {
    await prompting;
    const trace = (error as { trace?: readonly runtime.TraceEntry[] }).trace ?? [];
    // A run that stopped at a `human` pause is not a run that failed, and the
    // whole way out is written for that difference: its own document `status`,
    // its own exit code, and a message that says where the answer goes rather
    // than what went wrong (grammar 8.7, PRD 5.11). `runFlow` raises it as a
    // `FlowFailure` like any other — nothing else about a parked run is
    // different — so it is recognized by what is on the chain rather than by
    // what class arrived.
    const interrupt = interruptOf(error);
    const status = interrupt === undefined ? "failed" : "interrupted";
    // A **divergence** is read off the chain for the reason an interrupt is: the
    // wrapper says the run did not reach quiescence, which is true of every
    // failure, and what a reader has to act on is the step the journal and this
    // run disagree at (PRD resolved q29, `docs/durability.md` §7). It is still a
    // failed run — exit `1`, `status: "failed"` — because nothing about the
    // composition can absorb it; only the sentence changes.
    const divergence = interrupt === undefined ? divergenceOf(error) : undefined;
    const reason =
      interrupt !== undefined
        ? describe(interrupt)
        : divergence !== undefined
          ? describe(divergence)
          : describe(error);
    const written = writeTrace(address, execution, status, trace, reason);
    if (format === "json") {
      // The same record the completed run answers with, `error` where its
      // `outputs` would be — the trace file's path included, because a run that
      // failed is the one a reader most wants the whole trace for.
      process.stdout.write(
        `${JSON.stringify(
          {
            flow: address,
            execution_id: execution,
            status,
            error: reason,
            trace_version: TRACE_VERSION,
            trace,
            ...(written === undefined ? {} : { trace_path: written }),
          },
          null,
          2,
        )}\n`,
      );
    } else {
      process.stderr.write(render(trace));
      process.stderr.write(`\n${reason}\n`);
      if (interrupt === undefined) {
        for (let cause: unknown = (error as { cause?: unknown }).cause; cause !== undefined; ) {
          process.stderr.write(`  cause: ${describe(cause)}\n`);
          cause = (cause as { cause?: unknown }).cause;
        }
      }
      if (written !== undefined) process.stderr.write(`\ntrace: ${written}\n`);
    }
    return interrupt === undefined ? 1 : 3;
  }

  await prompting;
  const written = writeTrace(address, execution, "completed", produced.trace);
  if (format === "json") {
    process.stdout.write(
      `${JSON.stringify(
        {
          flow: address,
          execution_id: execution,
          status: "completed",
          outputs: produced.outputs,
          trace_version: TRACE_VERSION,
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

/**
 * What this run owes the outside world, journaled while its lifecycle row is
 * still open.
 *
 * `runFlow`'s `closing` hook for this command, and it has two things on it: a
 * `resume` closes an execution somebody subscribed to with a `callback:`
 * ([`owed`]), and **every** settled execution ships its trace where the deploy
 * layer names a sink (grammar 14.5, PRD resolved q50). The order is the ordinal's
 * — a receiver orders on it — and it is the order a `serve` settle uses too.
 *
 * Only the **intents** are recorded here. The attempts wait until the run has
 * reported ([`shipped`]), which is what keeps the sink off the path between a
 * run and its answer.
 */
async function settled(
  job: Job,
  exported: { record?: runtime.DeliveryRecord },
  produced: FlowRun | undefined,
  error: unknown,
): Promise<void> {
  if (job.callback !== undefined) await owed(job, produced, error);
  if (!sinkConfigured()) return;
  // A failure carries its trace on the chain and a completion carries it on the
  // answer; a failure raised before the graph ran carries none, and ships an
  // envelope with no entries in it — which `docs/trace.md` §2 makes a statement
  // about the run rather than a way of being absent.
  const trace =
    produced === undefined
      ? (error as { trace?: readonly runtime.TraceEntry[] } | null)?.trace
      : produced.trace;
  // **Insisted on rather than tried once**, for the reason `src/serve.ts` insists
  // on the same write: the lifecycle row closes as this hook returns, so an
  // intent the journal would not take here is a trace nothing will ever ship —
  // no start would find an open execution for it and no `pending` row would be
  // there to pick up. Bounded like every other ladder on this ledger
  // (`docs/durability.md` §3.7): its end is a sentence on stderr and a trace file
  // that still holds the run.
  await insisting(async () => {
    try {
      exported.record = await shipTrace({
        execution: job.execution,
        flow: job.address,
        // Two of the envelope's three: this hook is reached only where the
        // lifecycle row closes, and a run holding a pause leaves it open.
        status: produced === undefined ? "failed" : "completed",
        ...(produced === undefined ? { error: describe(error) } : {}),
        entries: trace ?? [],
      });
      return true;
    } catch (failure) {
      process.stderr.write(
        `\`${job.execution}\`'s trace could not be journaled: ${describe(failure)}\n`,
      );
      return false;
    }
  });
}

/**
 * Journal the `settled` webhook a resumed execution owes, **while its lifecycle
 * row is still open** (`docs/durability.md` §3.7, §6.2, PRD resolved q35).
 *
 * `runFlow`'s `closing` hook, and the same one `serve` supplies for the same
 * reason: an execution an `http` trigger started with a `callback:` is owed one
 * push whichever process gets to the end of it, and this command is a process
 * that can. The order is the whole of it. A row that closes with no delivery
 * intent beside it is an execution `serve` will never look at again — `recover`
 * enumerates open executions and finds none, the delivery ledger holds no
 * pending row — so a caller who was handed a `202` and, by resolved q34's own
 * reasoning, is *not* polling would simply never be told. Recorded first, the
 * webhook survives this command exiting a millisecond later.
 *
 * **Recorded, not sent.** The schedule `docs/durability.md` §3.7 states runs for
 * fifteen minutes and a command that exits when its run does cannot work one;
 * `serve` picks up every `pending` row at start (§6.1), matches the URL against
 * the trigger's `callback_allow:` and signs it with the identity that trigger
 * declared — all of which is the app's to do, and none of which this command has
 * an app for. So the row goes down and the sending waits, exactly as it does for
 * the row a build that no longer declares an execution's trigger leaves behind.
 *
 * A settle is **once per execution**, and the journal is what says so across
 * processes: a generation that journaled the intent and died before the row
 * closed leaves an execution that is still open *and* already has its `settled`
 * row, and announcing a second one here would tell a receiver that one execution
 * finished twice.
 */
async function owed(job: Job, produced: FlowRun | undefined, error: unknown): Promise<void> {
  const url = job.callback;
  if (url === undefined) return;
  try {
    const held = await deliveriesOf(job.execution);
    // The **kind** as well as the event: a target that declares a `trace_sink:`
    // puts two `settled` rows on the ledger, and the trace export is not the
    // webhook this caller is waiting for (grammar 14.5).
    if (held.some((record) => record.kind === "callback" && record.event === "settled")) return;
    // A failure carries its trace on the chain and a completion carries it on
    // the answer; a failure raised before the graph ran carries none, which is
    // the report that goes without the version beside it.
    const trace =
      produced === undefined
        ? (error as { trace?: readonly runtime.TraceEntry[] } | null)?.trace
        : produced.trace;
    await intendDelivery({
      execution: job.execution,
      kind: "callback",
      // Which trigger's identity the app that finally sends this row is to sign
      // it with, written beside the row for the reason `src/serve.ts` writes it:
      // a delivery names its own trigger rather than depending on a lifecycle
      // row being readable when it is picked up (`docs/durability.md` §3.7).
      // A `run` names no trigger and never reaches here, because a `callback:`
      // arrives only on a row a resume read.
      ...(job.trigger === undefined ? {} : { trigger: job.trigger }),
      event: "settled",
      url,
      body: JSON.stringify(
        await executionReport({
          id: job.execution,
          flow: job.address,
          // Off the row this resume read, so the report says what started the
          // execution rather than what finished it.
          trigger: job.trigger ?? "manual",
          status: produced === undefined ? "failed" : "completed",
          ...(produced === undefined ? { error: describe(error) } : { outputs: produced.outputs }),
          ...(trace === undefined ? {} : { trace }),
        }),
      ),
      // A settle reports no pauses: the row is closing.
      pauses: [],
    });
  } catch (failure) {
    // Not this run's failure — it produced whatever it produced, and the journal
    // holds it — but not something to swallow either: what failed is the record,
    // and a reader has no other way to learn that a webhook was lost.
    process.stderr.write(
      `\`${job.execution}\`'s \`settled\` webhook could not be journaled: ${describe(failure)}\n`,
    );
    return;
  }
  process.stderr.write(
    `\`${job.execution}\`'s \`settled\` webhook is journaled for \`${url}\`: \`serve\` delivers it\n`,
  );
}

// ---------------------------------------------------------------------------
// Answering a pause at the terminal
// ---------------------------------------------------------------------------

/**
 * The variable that decides whether a `run` prompts, where stdin cannot.
 *
 * Not a test seam: a pause answered from a script is the same delivery a person
 * makes, and both the emitted `README.md` and grammar 8.7 document this as a
 * feature. `1` prompts whatever stdin is; `0` never prompts; unset, stdin's own
 * `isTTY` decides.
 */
const INTERACTIVE = "AGENT_COMPOSE_INTERACTIVE";

/**
 * Whether this `run` may ask a person a question (grammar 8.7).
 *
 * A value that is neither `1` nor `0` is a **usage error** rather than a
 * silently-ignored setting, which is D50's posture applied to an environment
 * variable: `AGENT_COMPOSE_INTERACTIVE=true` accepted-and-ignored is a caller
 * who believes their script will answer the pause it is piping into, watching it
 * exit `3` with nothing anywhere to say why.
 */
function interactively(): boolean {
  const forced = process.env[INTERACTIVE];
  if (forced === undefined || forced === "") return process.stdin.isTTY === true;
  if (forced === "1") return true;
  if (forced === "0") return false;
  throw new UsageError(
    `\`${INTERACTIVE}=${forced}\` is not one of \`1\` (answer \`human\` pauses at standard input) or \`0\` (never): unset it to decide by whether standard input is a terminal`,
  );
}

/** Where a prompt is written, and where the answer to it is read from. */
export interface Terminal {
  /** Answers arrive here, one JSON value per line. */
  readonly input: NodeJS.ReadableStream;
  /** Prompts and refusals go here — stderr, so stdout stays the run's answer. */
  readonly output: { write(text: string): unknown };
}

/**
 * A promise that says when the run stopped, and never rejects.
 *
 * The prompt loop needs the *moment* rather than the outcome — a run that failed
 * has no more questions either — and a rejection nobody handled would end the
 * process before the failure reached the reporting below.
 */
function settling(running: Promise<unknown>): Promise<void> {
  return running.then(
    () => {},
    () => {},
  );
}

/**
 * Ask every pause this run opens, one at a time, and deliver the answers
 * (grammar 8.7, PRD 5.11).
 *
 * Exported so an ejected project keeps the surface: this is the second delivery
 * surface for a human answer, beside the app's resume route, and both go through
 * `runtime.deliverHumanAnswer`. Nothing here decides which pause an answer
 * addresses, what schema it is held to, or what a refusal means — those are the
 * board's, and asking them here would be a second implementation of the resume
 * route that could disagree with it.
 *
 * # What it does, in order
 *
 * Each question is the **lowest-id pause open when it is asked**, which is the
 * order the status route publishes them in and for that function's reason
 * (`runtime.humanWaits`): the order pauses *open* is the scheduler's, so a `map`
 * over two items that parked in one instant would otherwise ask its two
 * questions in a different order on a different machine. Ids are instance paths,
 * derived from the composition, so pauses that are open together are asked in an
 * order the composition fixes rather than the scheduler.
 *
 * It is "when it is asked" rather than a total order over the run's pauses, and
 * the difference is a question already on the screen: a pause that opens while
 * one is being asked is asked **after** it, even where its id sorts first,
 * because the only way to put it first would be to take back a question a person
 * is already answering. So a run whose pauses open at moments its own `agent:`
 * and `http:` latencies decide can ask them in an order that latency decided;
 * what is fixed is that no *set* of pauses waiting together is asked in the
 * scheduler's order, and that every prompt names the id it belongs to.
 *
 * One line of JSON per answer, and that is the framing: a value spanning lines
 * has no terminator a prompt could recognize without either guessing or hanging
 * on a malformed one. A line that is not JSON, or that the node's `output:`
 * refuses, is a refusal and a **re-prompt** — the wait is not consumed, exactly
 * as a `400` from the resume route does not consume it. A blank line is not an
 * answer at all and re-prompts without a refusal.
 *
 * # The two ways a prompt ends without an answer
 *
 * A pause can stop waiting while its question is on the screen, and both ways it
 * can are the run's rather than this loop's: its `timeout:` ran out and
 * `on_timeout:` routed the execution on, or the run ended some other way and the
 * board was released. Either way the prompt is **withdrawn** — with the sentence
 * the resume route would have refused a late answer with — and the next pause is
 * asked. A line typed *for* the withdrawn question, arriving after it is gone, is
 * read as the next question's answer: a stream of typed lines carries no
 * addressing, so a line meant for a question that has just been taken away is
 * indistinguishable from one meant for the question that replaced it. Every
 * prompt names its own wait id for that reason (see [`Lines.abandon`]).
 *
 * And **standard input can end**, which is the surface itself going away:
 * `runtime.closeHumanWaits` turns every pause still waiting, and every one this
 * run opens later, into the interrupt a run with no surface raises — so the run
 * ends where it stood instead of parking on a question nothing can answer.
 */
export async function answerPauses(
  execution: string,
  finished: Promise<unknown>,
  terminal: Terminal,
): Promise<void> {
  // Subscribed **before** anything is read, and against the execution id rather
  // than a board — `runtime.watchHumanPauses` takes it either way, so this does
  // not race the run's own opening of the board.
  const listeners = new Set<() => void>();
  const wake = (): void => {
    for (const listener of [...listeners]) listener();
  };
  const unwatch = watchHumanPauses(execution, wake);
  let over = false;
  const stop = (): void => {
    over = true;
    wake();
  };
  void finished.then(stop, stop);

  // Built at the first question rather than here: a run with no `human` node in
  // it must not attach a `data` listener to stdin, which would drain a stream
  // this command was never given for itself.
  let reading: Lines | undefined;

  /** Resolve as soon as `ready` answers something, or when the run stops. */
  const upon = <T,>(ready: () => T | undefined): Promise<T | undefined> =>
    new Promise<T | undefined>((resolve) => {
      const check = (): void => {
        const answer = over ? undefined : ready();
        if (answer === undefined && !over) return;
        listeners.delete(check);
        resolve(answer);
      };
      listeners.add(check);
      check();
    });

  try {
    for (;;) {
      // The lowest id **open at this moment**: `runtime.humanWaits` orders the
      // board, so a set of pauses waiting together is asked in the
      // composition's order rather than the scheduler's. A pause that opens
      // later is asked later, whatever its id sorts as — see the note above.
      const wait = await upon((): runtime.HumanWait | undefined => {
        const open = humanWaits(execution);
        return open.length === 0 ? undefined : open[0];
      });
      if (wait === undefined) return;
      if (reading === undefined) {
        reading = lines(terminal.input);
        // One turn of the event loop between attaching to the stream and asking
        // anything of it, and only ever this once. A stream that is **already**
        // at its end — a run launched with nothing on standard input, which is
        // every `agent-compose run` in a script that forgot to pipe an answer —
        // announces that end on the `end` event, and that event cannot have
        // fired before the listener `lines` just attached existed. So the very
        // first `spent()` would be read a turn too early and answer `false` for
        // a surface that was gone before the run started: the whole block
        // printed and withdrawn on the line under it, which is the one thing
        // the guard in [`ask`] exists to prevent. Every later question is asked
        // with the listener attached for the whole of the run behind it, so
        // there is nothing left to wait for.
        await new Promise<void>((resolve) => setTimeout(resolve, 0));
        // Asked again rather than asked now: the board can have moved in that
        // turn — the pause read above may have expired, and the run itself may
        // have ended — and a question is owed to what is open when it is
        // printed, not to what was open a turn before.
        continue;
      }
      const ended = await ask(execution, wait, reading, terminal.output, upon);
      if (ended === "input-ended") {
        terminal.output.write(
          `\nstandard input ended, so nothing can answer this run's pauses any more.\n`,
        );
        closeHumanWaits(execution);
        return;
      }
    }
  } finally {
    unwatch();
    reading?.stop();
  }
}

/** How one prompt ended: with an answer, without one, or with no input left. */
type Asked = "settled" | "input-ended";

/**
 * Ask one pause and read until it is answered, withdrawn, or stdin ends.
 *
 * `upon` is [`answerPauses`]'s subscription, passed in so the withdrawal watch
 * and the pause watch are one registration on the wait board rather than two
 * that could see different moments.
 */
async function ask(
  execution: string,
  wait: runtime.HumanWait,
  reading: Lines,
  output: Terminal["output"],
  upon: <T>(ready: () => T | undefined) => Promise<T | undefined>,
): Promise<Asked> {
  // Asked before the block is rendered, because standard input can end while
  // the loop is parked with no pause open — there is no read outstanding then,
  // so nothing notices until the next question goes looking for an answer. A
  // question printed in full and withdrawn on the line under it is a prompt
  // that never existed; the surface was already gone. The same is true of a
  // stream that had already ended when the reader attached, which is why
  // [`answerPauses`] gives the `end` event a turn to arrive before it asks the
  // first question — this check reads a stream's state, and a state nothing has
  // reported yet is not one it can read.
  if (reading.spent()) return "input-ended";
  output.write(question(wait));
  // One withdrawal watch for the whole prompt, resolving with the sentence the
  // resume route refuses a late answer with. The `??` branch is the run ending:
  // `upon` answers `undefined` for that rather than for a settlement, because the
  // board goes with the run and there is no wait left to have one.
  const withdrawn = upon(() => humanWaitEnded(execution, wait.id)).then(
    (detail) => detail ?? `the wait at \`${wait.id}\` is no longer held: the run ended`,
  );
  for (;;) {
    const read = await Promise.race([
      reading.next(),
      withdrawn.then((detail): Read => ({ kind: "withdrawn", detail })),
    ]);
    if (read.kind === "withdrawn") {
      // The read this prompt had outstanding is dropped, so the next line to
      // arrive is read by whatever question is being asked *then* rather than by
      // a promise nobody is waiting on any more. A line already in hand is not
      // affected: it reached the question it was typed for and was refused by it
      // as settled, one branch below.
      reading.abandon();
      output.write(`\nthis question is withdrawn: ${read.detail}\n`);
      return "settled";
    }
    if (read.kind === "end") return "input-ended";
    const typed = read.line.trim();
    // A stray newline is not an answer, and refusing it as malformed JSON would
    // be a sentence about a value nobody typed.
    if (typed === "") {
      output.write(asked(wait));
      continue;
    }
    let payload: unknown;
    try {
      payload = JSON.parse(typed) as unknown;
    } catch (error) {
      output.write(
        `an answer is one line of JSON, and this line is not one: ${describe(error)}\n`,
      );
      output.write(asked(wait));
      continue;
    }
    const outcome = deliverHumanAnswer(execution, wait.id, payload);
    if (outcome.ok) {
      output.write(`taken.\n`);
      return "settled";
    }
    if (outcome.reason === "mismatch") {
      // The wording the resume route refuses a `400` with, and the same fact
      // behind it: the wait was not consumed, so the corrected answer goes to
      // the same question.
      output.write(
        `that answer does not fit the \`human\` node's \`output:\`, so \`${wait.id}\` is still waiting for one that does: ${outcome.detail}\n`,
      );
      output.write(asked(wait));
      continue;
    }
    // Every other refusal is about *which* pause rather than about what was
    // typed, and each of them means this question is over — the answer raced an
    // expiry and lost, or the board stopped holding it. Its own sentence, and on
    // to the next.
    output.write(`${outcome.detail}\n`);
    return "settled";
  }
}

/** The block one pause is presented as, on stderr. */
function question(wait: runtime.HumanWait): string {
  let block = `\npause \`${wait.id}\` — ${wait.flow} node \`${wait.node}\`\n`;
  block += "  shown:\n";
  for (const line of JSON.stringify(wait.shown, null, 2).split("\n")) {
    block += `    ${line}\n`;
  }
  block += `  answer: ${outline(wait.schema)}\n`;
  if (wait.expiresAt !== undefined) block += `  expires: ${wait.expiresAt}\n`;
  return `${block}${asked(wait)}`;
}

/** The one-line prompt itself, repeated after every refusal. */
function asked(wait: runtime.HumanWait): string {
  return `answer \`${wait.id}\` with one line of JSON: `;
}

/**
 * The node's `output:` schema, in one readable line.
 *
 * The published JSON Schema is what a *program* is given (the status route's
 * `output_schema`), and printing it at a prompt would bury the two things a
 * person needs — the field names and what each will take — in the keywords that
 * carry the rest. So this is a sketch: `{ decision: "approve" | "reject", note?:
 * string }`, where `?` is a property the schema does not require.
 *
 * Bounded in depth, because a prompt is a line: past three levels the shape is
 * elided rather than wrapped, and the full schema is a poll of the status route
 * away for anyone who needs it.
 */
function outline(schema: unknown, depth = 0): string {
  if (typeof schema !== "object" || schema === null) return "any";
  const node = schema as Record<string, unknown>;
  const variants = node["enum"];
  if (Array.isArray(variants)) {
    return variants.map((variant) => JSON.stringify(variant)).join(" | ");
  }
  const union = node["oneOf"] ?? node["anyOf"];
  if (Array.isArray(union)) {
    return depth >= 3 ? "…" : union.map((one) => outline(one, depth + 1)).join(" | ");
  }
  const type = node["type"];
  if (type === "array") return `${outline(node["items"], depth + 1)}[]`;
  if (type === "object" || node["properties"] !== undefined) {
    if (depth >= 3) return "{ … }";
    const properties = (node["properties"] ?? {}) as Record<string, unknown>;
    const required = new Set(
      (Array.isArray(node["required"]) ? node["required"] : []).map((name) => String(name)),
    );
    const fields = Object.entries(properties).map(
      ([name, property]) =>
        `${name}${required.has(name) ? "" : "?"}: ${outline(property, depth + 1)}`,
    );
    return fields.length === 0 ? "{}" : `{ ${fields.join(", ")} }`;
  }
  if (typeof type === "string") return type;
  if (Array.isArray(type)) return type.map((one) => String(one)).join(" | ");
  return "any";
}

/** What one read off the answer stream produced. */
type Read =
  | { readonly kind: "line"; readonly line: string }
  | { readonly kind: "end" }
  | { readonly kind: "withdrawn"; readonly detail: string };

/** Reading an answer stream one line at a time. */
interface Lines {
  /** The next line, or `end` once the stream has no more. */
  next(): Promise<Read>;
  /**
   * Drop the outstanding read, so the next line goes to the next caller.
   *
   * What a withdrawn prompt does with the read it had open: nobody is waiting on
   * that promise any more, and a queue entry left in front of the next question's
   * read would swallow the line meant for it. The rule this leaves is the one a
   * reader can hold in their head — **a line is answered by whatever question is
   * being asked when it arrives** — and it is the honest one, because a stream of
   * answers carries no addressing: a line typed for a question that has just been
   * withdrawn is indistinguishable from one typed for the question that replaced
   * it. Each prompt names its own wait id for exactly that reason.
   */
  abandon(): void;
  /**
   * Whether the stream has ended and holds nothing a read could still answer.
   *
   * What [`ask`] consults before it renders a question: the end arrives on the
   * stream's own event, so a loop parked with no pause open learns about it
   * with no read outstanding to be answered `end`. Without this a run whose
   * standard input closed while it was busy would print a whole prompt — the
   * wait id, the `shown:` block, the schema, the deadline — and withdraw the
   * surface on the line under it.
   */
  spent(): boolean;
  /** Stop reading the stream, and stop holding it open. */
  stop(): void;
}

/**
 * One line at a time off a stream, in the order the bytes arrive.
 *
 * Hand-rolled rather than `node:readline`, because what a prompt needs is
 * exactly this and the module brings a terminal's worth of behaviour with it —
 * echo, history, key handling — that would differ between the two supported
 * runtimes on a surface where they must not (PRD §9.18). What is left is the
 * seam that really is the engine's: a `data` event, a `end` event, and the
 * encoding. Those are what the generated-code gates ask of both.
 */
function lines(input: Terminal["input"]): Lines {
  let held = "";
  let ended = false;
  const waiting: ((read: Read) => void)[] = [];

  const serve = (): void => {
    for (;;) {
      if (waiting.length === 0) return;
      const at = held.indexOf("\n");
      if (at >= 0) {
        const line = held.slice(0, at);
        held = held.slice(at + 1);
        waiting.shift()!({ kind: "line", line });
        continue;
      }
      if (!ended) return;
      // The stream is over. A last line with no newline on it is still a line —
      // `printf '{"decision":"approve"}'` is a script that answered — and every
      // read after it is the end.
      if (held.trim() !== "") {
        const line = held;
        held = "";
        waiting.shift()!({ kind: "line", line });
        continue;
      }
      held = "";
      waiting.shift()!({ kind: "end" });
    }
  };

  const onData = (chunk: unknown): void => {
    held += typeof chunk === "string" ? chunk : String(chunk);
    serve();
  };
  const onEnd = (): void => {
    ended = true;
    serve();
  };
  input.setEncoding("utf8");
  input.on("data", onData);
  input.on("end", onEnd);

  return {
    next(): Promise<Read> {
      return new Promise<Read>((resolve) => {
        waiting.push(resolve);
        serve();
      });
    },
    abandon(): void {
      // Only ever the read at the head, and only ever one still waiting: a read
      // that had a line was resolved and shifted off inside `serve`.
      waiting.shift();
    },
    spent(): boolean {
      // What is held matters as much as the end: a stream that ended on
      // `printf '{"decision":"approve"}'` has no newline on it and still has an
      // answer in hand, which is the line `serve` gives the next read.
      return ended && held.trim() === "";
    },
    stop(): void {
      input.removeListener("data", onData);
      input.removeListener("end", onEnd);
      input.pause();
    },
  };
}

/** `serve [--host <host>] [--port <port>]`. */
async function serveVerb(argv: readonly string[]): Promise<number> {
  const options = parse("serve", argv, { host: "single", port: "single" });
  const port = options.single["port"];
  if (port !== undefined && !/^[0-9]+$/.test(port)) {
    throw new UsageError(`\`--port ${port}\` is not a port number`);
  }
  // Loaded here rather than imported at the top, so a `run` never pays for the
  // HTTP framework and a project used as a library never loads it at all. Only
  // the app: `./triggers.ts` is the composition's own table and is imported
  // above, because `run` reads it too.
  const { BlankCredentialError, serve } = await import("./serve.ts");
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
    // Another that is not about the address: the callback retry schedule an
    // operator overrode with something that is not one. It is read before a
    // route exists, so it arrives here — as the usage error it is, in its own
    // words, rather than dressed as a port that would not bind (Decision D50,
    // `docs/durability.md` §3.7).
    if (error instanceof CallbackRetryError) throw new UsageError(error.message);
    // And a third: a credential a trigger declares that resolved to the empty
    // string. `src/env.ts` counts it as present (grammar 4.3), and for a
    // credential it is not — an empty token admits every caller — so the app
    // refuses to mount rather than serving an open route (grammar 13.3). It is
    // the environment's to fix, like a missing variable, so it is a `2` and a
    // sentence naming what to set.
    if (error instanceof BlankCredentialError) throw new UsageError(error.message);
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
  // …and then the whole set against the flow's own `inputs:` schema, which is
  // what decides the other two failures grammar 13.2 names in the same sentence
  // as the unknown-argument one above: a REQUIRED field nobody passed, and a
  // value the field's declared type refuses past the kind it was read as (an
  // `enum` variant that is not one, a string below its `min_length`, a JSON
  // document whose properties are not the declared ones). `runFlow` parses with
  // the same schema and would refuse the same set — but as a *run* that produced
  // no answer, which is not what happened: nothing ran. Asked here, all three
  // are one usage error naming the field.
  try {
    flow.parse(inputs);
  } catch (error) {
    // `CompiledFlow.parse` reports through `runtime.parseResult`, whose message
    // is already the sentence this wants; the class name it would carry through
    // [`describe`] belongs to a stack trace rather than to a usage line.
    throw new UsageError(error instanceof Error ? error.message : String(error));
  }
  return inputs;
}

/**
 * The session identity this run addresses (grammar 13.2, 11.3).
 *
 * `--session <key>` is the whole manual payload — "the manual payload has
 * exactly one member — `payload.session`" — and a declared `manual` trigger
 * naming this flow may remap it through `session_key:`. Left undeclared, the key
 * is its own `"payload.session"` default and the argument is the identity, so
 * there is nothing to evaluate.
 *
 * **An argument that was not passed is not remapped.** `--session` is what
 * grammar 13.2 makes mandatory for a run whose flow reaches a session-scoped
 * store, and a remap of nothing would answer with something — `'tenant-' +
 * payload.session` is `"tenant-"` for the run nobody gave a session — which is a
 * partition named after an identity that does not exist. So the empty argument
 * passes through as itself and [`requireSession`] refuses the run naming the
 * store.
 *
 * Which remap applies is not a choice: two `manual` triggers naming one flow may
 * not declare different `session_key:` expressions, and the compiler refuses a
 * composition where they do.
 */
function sessionOf(flow: CompiledFlow, given: string): string {
  if (given === "") return "";
  for (const trigger of manualTriggers) {
    if (trigger.flow !== flow.address) continue;
    const remap = trigger.sessionKey;
    if (remap === undefined) continue;
    try {
      return remap(given);
    } catch (error) {
      throw new UsageError(
        `\`--session ${given}\` could not be read as this run's session identity: ${describe(error)}`,
      );
    }
  }
  return given;
}

/**
 * Refuse a `run` whose flow needs a session identity and was given none
 * (grammar 11.3, 13.2).
 *
 * A **usage** error, so the command exits `2` rather than `1`. The three
 * failures grammar 13.2 puts in one sentence — an unknown argument name, a
 * missing REQUIRED field, a value that does not fit — are already decided here
 * for that reason, and this is the fourth of the same kind: an argument the
 * caller has to add before anything can run. Grammar 11.3 says so itself,
 * likening the check to env-ref presence (§4.3) — which is a `2` in this
 * project's exit-code table, where `1` means a run produced no answer and `2`
 * means the command could not be run at all. Nothing ran here, and a supervisor
 * that retries `1` and reports `2` would otherwise retry an invocation that
 * cannot succeed however many times it is repeated.
 *
 * `runFlow` keeps the same guard for the callers this one cannot stand in for: a
 * `serve` request, whose session key is the request's rather than the command
 * line's, and an ejected caller invoking a compiled flow directly. There it
 * really is one run that failed.
 */
function requireSession(address: string, flow: CompiledFlow, session: string): void {
  if (session !== "" || flow.sessionStores.length === 0) return;
  throw new UsageError(sessionRefusal(address, flow.sessionStores));
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

/** Whether an option may be given more than once. */
type Arity = "single" | "repeatable";

/**
 * The flags one verb takes: `--name value`, some repeatable.
 *
 * The verb declares **every** option it takes, and a `--name` outside that list
 * is a usage error rather than an entry nothing ever reads. That is D50's
 * posture — a typo is a diagnostic, never a silent no-op — applied to the one
 * surface the compiler does not stand in front of: `agent-compose run` is
 * shielded by its own argument parser, but an ejected project's command line is
 * this function, and `--fromat json` answering with human output and exit `0`
 * is a caller parsing the wrong document with nothing to tell it so.
 *
 * The options are declared in the order the README documents them, because that
 * order is what the refusal lists back.
 */
function parse(
  verb: string,
  argv: readonly string[],
  options: Readonly<Record<string, Arity>>,
): { single: Record<string, string>; repeated: Record<string, string[]> } {
  const single: Record<string, string> = {};
  const repeated: Record<string, string[]> = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index]!;
    if (!argument.startsWith("--")) {
      throw new UsageError(`\`${argument}\` is not an option: every argument after the verb is \`--name value\``);
    }
    const name = argument.slice(2);
    // `Object.hasOwn` rather than a lookup: every object carries `constructor`
    // and `toString`, so `--constructor x` would read an inherited function as
    // this option's arity and be accepted as one — the same property-versus-key
    // confusion the compiler refuses a *channel* named `constructor` over.
    const arity = Object.hasOwn(options, name) ? options[name] : undefined;
    if (arity === undefined) {
      const taken = Object.keys(options).map((option) => `\`--${option}\``);
      throw new UsageError(
        `\`--${name}\` is not an option of \`${verb}\`: it takes ${taken.length === 0 ? "no options" : taken.join(", ")} (see README.md)`,
      );
    }
    const value = argv[index + 1];
    if (value === undefined) throw new UsageError(`\`--${name}\` takes a value`);
    index += 1;
    if (arity === "repeatable") {
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
 * What lands is a `runtime.TraceDocument` — the **envelope** of `docs/trace.md`,
 * not a bare array of entries. A file is the one delivery surface that arrives
 * on its own: the JSON record `--format json` prints already names the flow, the
 * execution and the version around its `trace`, and a reader who opens the file
 * a month later has none of that unless the file carries it. `trace_version` is
 * the load-bearing half — a reader pins it and knows which fields it may rely on
 * — and `flow`, `execution_id` and `status` are what tie the document back to
 * the run without parsing the file's own name.
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
  status: "completed" | "failed" | "interrupted",
  trace: readonly runtime.TraceEntry[],
  error?: string,
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
    const document: runtime.TraceDocument = {
      trace_version: TRACE_VERSION,
      flow: address,
      execution_id: execution,
      status,
      ...(error === undefined ? {} : { error }),
      entries: trace,
    };
    fs.writeFileSync(file, `${JSON.stringify(document, null, 1)}\n`);
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
    if (entry.human !== undefined) {
      const pause = entry.human;
      const until = pause.expiresAt === undefined ? "no deadline" : `until ${pause.expiresAt}`;
      const how =
        pause.settled === undefined
          ? "still waiting when the run ended"
          : `${pause.settled} at ${pause.settledAt}`;
      text += `    human — paused at ${pause.pausedAt} (${until}), ${how}\n`;
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
