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
//   FEED_UNREADABLE_AT
//                    before emitting this sequence number, emit two lines whose
//                    `checkpoint:` cannot be evaluated: one with no `cursor` at
//                    all, and one whose `cursor` is the number rather than the
//                    string a checkpoint must be
//   FEED_LOG         append every sequence number emitted, one per line — the
//                    oracle the acceptance suite counts executions against
//   FEED_STARTS      append, per start, the checkpoint it was handed: the
//                    `{checkpoint}` argument and `AGENT_COMPOSE_CHECKPOINT`
//   FEED_LINGER      be a program that will not go quietly: start a child
//                    (`sleep`) in its own process group, write both pids to
//                    this file, ignore SIGTERM and the end of stdin — so only
//                    the SIGKILL `serve` sends the group after its grace
//                    ends either of them
//   FEED_HELPER      on every run, start a helper beside itself — a `sleep`
//                    that ignores SIGTERM, in the group `serve` started this
//                    program in, holding none of its pipes — and append the
//                    helper's pid to this file: what a run leaves in its group
//                    when it ends, unless something ends the group
//   FEED_HELPER_STDIO
//                    `inherit`: the FEED_HELPER helper keeps this program's
//                    stdout and stderr instead, as `cmd &` in a shell or a
//                    `Popen` left with the default stdio does — so the pipes
//                    `serve` reads stay open after this program exits, for as
//                    long as the helper lives
//   FEED_CLEANER     start a helper in this program's group, holding none of
//                    its pipes, that on SIGTERM takes a second to clean up and
//                    then writes `cleaned` to this file and exits — while this
//                    program itself goes at once: whether the helper is given
//                    its grace once its leader has gone
//   FEED_EXIT_CODE   after the last event, exit with this code rather than
//                    wait for stdin to end — on every run, so a restart whose
//                    checkpoint is already the last event exits as soon as it
//                    starts: a program that keeps crashing
//   FEED_UNHANDABLE  on a run that exits part-way (FEED_EXIT_AFTER), emit
//                    three more lines after its last event whose `cursor` no
//                    program could be handed back — one holding a NUL, one a
//                    lone UTF-16 surrogate, one 65537 bytes long — before the
//                    exit: journaled, any of them would be the checkpoint the
//                    restart is started with
//   FEED_FRAMING     on a run that starts with no checkpoint, test the line
//                    framing instead of emitting the stream: a stderr line of
//                    1.5 MiB and one after it; event 1; event 2, exactly
//                    1 MiB long; an event object of 1.5 MiB; event 3; a
//                    stderr fragment and then event 4, neither ended by a
//                    newline; then exit. Every event's checkpoint is its seq,
//                    so FEED_TOTAL=4 makes the restart emit nothing
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
const helper = process.env.FEED_HELPER;
if (helper) {
  // Not detached: a member of this program's group. With its stdio closed it
  // keeps none of its pipes open, so the run's output ends when this process
  // exits and the helper is still running; with FEED_HELPER_STDIO=inherit it
  // keeps stdout and stderr open past this process's exit. `trap '' TERM`
  // survives the `exec`: an ignored signal stays ignored across one.
  const stdio =
    process.env.FEED_HELPER_STDIO === "inherit" ? ["ignore", "inherit", "inherit"] : "ignore";
  const child = spawn("sh", ["-c", "trap '' TERM; exec sleep 600"], { stdio });
  appendFileSync(helper, `${child.pid}\n`);
  child.unref();
}
const cleaner = process.env.FEED_CLEANER;
if (cleaner) {
  // Not detached, and its stdio closed: a member of this program's group that
  // holds none of its pipes, so the run's output ends the moment this process
  // goes. The `SIGTERM` a stop sends the group interrupts the `wait`; the trap
  // then cleans up for a second — a `sleep` started after the signal, which
  // the signal never reached — and only then marks the cleanup done.
  const child = spawn(
    "sh",
    ["-c", 'trap \'sleep 1; echo cleaned > "$1"; exit 0\' TERM; sleep 600 & wait', "cleaner", cleaner],
    { stdio: "ignore" },
  );
  child.unref();
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

/** Write `text` to `stream` as it stands, resolving once it is flushed. */
function flushed(stream, text) {
  return new Promise((resolve) => stream.write(text, resolve));
}

/** Event `seq`, as one line of JSON — padded out to `length` bytes when given. */
function event(seq, length) {
  const head = `{"seq":${seq},"cursor":"${seq}","text":"event ${seq}","pad":"`;
  const tail = '"}';
  const pad = length === undefined ? 0 : length - head.length - tail.length;
  return `${head}${"x".repeat(pad)}${tail}`;
}

if (process.env.FEED_FRAMING && !resumed) {
  const mib = 1 << 20;
  await flushed(process.stderr, `${"e".repeat(mib + mib / 2)}\nafter the long line\n`);
  for (const [seq, line] of [
    [1, event(1)],
    [2, event(2, mib)],
    [undefined, event(99, mib + mib / 2)],
    [3, event(3)],
  ]) {
    await flushed(process.stdout, `${line}\n`);
    if (seq !== undefined && process.env.FEED_LOG) appendFileSync(process.env.FEED_LOG, `${seq}\n`);
  }
  await flushed(process.stderr, "the last words, unterminated");
  await flushed(process.stdout, event(4));
  if (process.env.FEED_LOG) appendFileSync(process.env.FEED_LOG, "4\n");
  process.exit(0);
}

if (process.env.FEED_GARBAGE) {
  await write("this line is not JSON");
  await write("[1, 2, 3]");
}
const holdAt = process.env.FEED_HOLD_AT ? Number(process.env.FEED_HOLD_AT) : undefined;
const unreadableAt = process.env.FEED_UNREADABLE_AT
  ? Number(process.env.FEED_UNREADABLE_AT)
  : undefined;
for (let seq = first; seq <= last; seq += 1) {
  if (seq === holdAt) {
    while (!existsSync(process.env.FEED_HOLD_FILE ?? "")) await sleep(50);
  }
  if (seq === unreadableAt) {
    await write(JSON.stringify({ seq, text: `event ${seq}, with no cursor` }));
    await write(JSON.stringify({ seq, cursor: seq, text: `event ${seq}, its cursor a number` }));
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
  if (process.env.FEED_UNHANDABLE) {
    const seq = last + 1;
    for (const cursor of [`${seq}\u0000`, `${seq}\ud800`, "9".repeat(65537)]) {
      await write(JSON.stringify({ seq, cursor, text: `event ${seq}` }));
    }
  }
  await sleep(Number(process.env.FEED_RUN_MS ?? "0"));
  process.exit(0);
}
if (process.env.FEED_EXIT_CODE) process.exit(Number(process.env.FEED_EXIT_CODE));
// Nothing more to say: wait for stdin to end.
setInterval(() => {}, 1 << 30);
