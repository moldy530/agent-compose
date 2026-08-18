// A scalar `input:` written to a command that never reads it.
//
// Grammar 8.2 sends a scalar `input:` to the child's standard input, and
// nothing obliges the child to drain it: `printf "%s" done` writes its argument
// and exits. The pipe then closes under a write that is still in flight, and
// Node reports that as an `error` **event on the stream** rather than as a
// rejected promise — so an unhandled one is not a node error at all. It is an
// abort of the whole process, past `retry`, `timeout`, `skip` and `fallback`
// alike (grammar 9), past the `catch` that writes a run's trace (PRD 5.3), and
// under `serve` it takes every concurrent execution with it.
//
// The payload is deliberately larger than a pipe buffer. A small one fits in the
// kernel's buffer and the write completes whether anybody reads it or not, so a
// runtime with no handler passes a small-payload test and dies on a real
// document.
//
// Usage: node unread-stdin.mjs <generated project directory>
// Output: { "sent": <bytes>, "result": … } as JSON. Printing at all is half the
// claim; the other half is that the result is the command's own.

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node unread-stdin.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

// Larger than the 64 KiB a pipe buffers on the platforms CI runs.
const PAYLOAD = "x".repeat(300_000);

const context = {
  execution: { id: "exec_unread_stdin", session_key: "" },
  signal: new AbortController().signal,
  node: "piped",
};

const result = await runtime.runExec(
  {
    command: ["printf"],
    args: [["%s"], ["done"]],
    env: [],
    expectExit: [0],
    decoding: { envelope: ["exit_code", "stdout"], decoded: [], empty: false },
  },
  PAYLOAD,
  context,
);

process.stdout.write(JSON.stringify({ sent: PAYLOAD.length, result }));
