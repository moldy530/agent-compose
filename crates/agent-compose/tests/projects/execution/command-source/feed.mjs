// The `feed` source's program: one JSON event per line on stdout (grammar 14.4,
// PRD resolved q78).
//
// It is what a reader's own program would be, steered by variables a test sets:
//
//   FEED_TOTAL       the last sequence number it emits (default 100)
//   FEED_EXIT_AFTER  on a run that starts with no checkpoint, exit after
//                    emitting this one rather than going on (default: never)
//   FEED_REWIND      on a run that starts with a checkpoint, begin this many
//                    events *before* the one after it — the at-least-once
//                    re-emission a real source makes after a restart
//   FEED_PACE_MS     wait this long between events (default 0)
//   FEED_RUN_MS      on a run that exits part-way, run this long before the
//                    exit (default 0): how a test makes a run outlast
//                    `restart.max`
//   FEED_GARBAGE     write a line that is not JSON, and one that is JSON but not
//                    an object, before the first event
//   FEED_UNBINDABLE  after the last event, emit one more whose `seq` is not an
//                    integer — so the trigger's `input:` binds nothing the
//                    flow accepts and every bound trigger refuses it — and then
//                    a line that is not JSON: lines are taken in one at a time,
//                    so that line's refusal being counted is the sign the event
//                    before it has been dealt with
//   FEED_HOLD_AT     before emitting this sequence number, wait for the file
//   FEED_HOLD_FILE   FEED_HOLD_FILE names to exist: how a test does something
//                    to the journal between two events
//   FEED_LOG         append every sequence number emitted, one per line — the
//                    oracle the acceptance suite counts executions against
//   FEED_STARTS      append, per start, the checkpoint it was handed: the
//                    `{checkpoint}` argument and `AGENT_COMPOSE_CHECKPOINT`
//   FEED_LINGER      be a program that will not go quietly: start a child
//                    (`sleep`) in its own process group, write both pids to
//                    this file, ignore SIGTERM and the end of stdin — so only
//                    the SIGKILL `serve` sends the group after its grace
//                    ends either of them
//
// The checkpoint is read from both places the runtime hands it back, and they
// must agree. After the last event it emits nothing more and waits for its
// stdin to end — which is how a program learns its `serve` has gone, whatever
// stopped it.
import { spawn } from "node:child_process";
import { appendFileSync, existsSync, writeFileSync } from "node:fs";
import process from "node:process";

const argument = process.argv[2] ?? "";
const variable = process.env.AGENT_COMPOSE_CHECKPOINT;
if (process.env.FEED_STARTS) {
  appendFileSync(
    process.env.FEED_STARTS,
    `${JSON.stringify({ argument, variable: variable ?? null })}\n`,
  );
}

const total = Number(process.env.FEED_TOTAL ?? "100");
const exitAfter = process.env.FEED_EXIT_AFTER ? Number(process.env.FEED_EXIT_AFTER) : undefined;
const rewind = Number(process.env.FEED_REWIND ?? "0");
const pace = Number(process.env.FEED_PACE_MS ?? "0");

const resumed = argument !== "";
const first = resumed ? Math.max(1, Number(argument) + 1 - rewind) : 1;
const last = !resumed && exitAfter !== undefined ? exitAfter : total;

const linger = process.env.FEED_LINGER;
if (linger) {
  // Not detached: the child is a member of the group `serve` started this
  // program in, which is the group a stop is sent to.
  const child = spawn("sleep", ["600"], { stdio: "ignore" });
  writeFileSync(linger, `${process.pid}\n${child.pid}\n`);
  process.on("SIGTERM", () => {});
}
process.stdin.on("data", () => {});
process.stdin.on("end", () => {
  if (!linger) process.exit(0);
});
process.stdout.on("error", () => {
  if (!linger) process.exit(0);
});

function write(line) {
  return new Promise((resolve) => {
    if (process.stdout.write(`${line}\n`)) resolve();
    else process.stdout.once("drain", resolve);
  });
}

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

if (process.env.FEED_GARBAGE) {
  await write("this line is not JSON");
  await write("[1, 2, 3]");
}
const holdAt = process.env.FEED_HOLD_AT ? Number(process.env.FEED_HOLD_AT) : undefined;
for (let seq = first; seq <= last; seq += 1) {
  if (seq === holdAt) {
    while (!existsSync(process.env.FEED_HOLD_FILE ?? "")) await sleep(50);
  }
  await write(JSON.stringify({ seq, cursor: String(seq), text: `event ${seq}` }));
  if (process.env.FEED_LOG) appendFileSync(process.env.FEED_LOG, `${seq}\n`);
  if (pace > 0) await sleep(pace);
}
if (process.env.FEED_UNBINDABLE) {
  const seq = last + 1;
  await write(JSON.stringify({ seq: "not a number", cursor: String(seq), text: `event ${seq}` }));
  await write("the event before this line binds no input");
}
if (!resumed && exitAfter !== undefined) {
  await sleep(Number(process.env.FEED_RUN_MS ?? "0"));
  process.exit(0);
}
// Nothing more to say: wait for stdin to end.
setInterval(() => {}, 1 << 30);
