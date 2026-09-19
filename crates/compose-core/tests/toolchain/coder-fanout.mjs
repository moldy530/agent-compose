// A `map` over a coder node, and a `workspace: fresh` run, driven through a
// compiled graph (grammar 8.9, PRD resolved q61).
//
// `coder-graph.mjs` runs a graph whose two coder nodes name one directory each,
// fixed for the process — which is every coder composition this project could
// write before resolved q61, and says nothing at all about the ruling. This
// runs the other shape: `workspace:` is an expression evaluated in the node's
// input scope at **each dispatch**, so what a fan-out is contained by is a
// property of the dispatch, and `workspace: fresh` is a directory the runtime
// provisions per dispatch and remakes empty per attempt.
//
// What it decides, and none of it is reachable from the adapter alone:
//
//   * **two dispatches, two directories.** The map binds each item's own
//     `worktree` into the instance and the coder node reads `input.worktree`;
//     the two runs are handed two different paths, and the paths are the ones
//     the items named. That is `max_concurrency` becoming real for coding;
//   * **both recorded.** Each run's `HarnessRecord` carries the directory that
//     run really held, which is the field resolved q61 added to the trace
//     because the composition's own text stopped answering the question
//     (`docs/trace.md` §7.6);
//   * **`fresh` is deterministic and under the execution's scratch.** The
//     summarising run's directory is
//     `<data>/workspaces/<execution>/summarise/0` — the §9.4 instance path,
//     joined the way every other frame is;
//   * **`fresh` is clean per attempt.** The first attempt writes a file into
//     its directory and then fails; the node's `retry:` runs a second attempt,
//     and what that one finds is an empty directory. A retry meeting the wreck
//     of the attempt it is retrying is the case ruling c is about.
//
// The seam is `registerHarnessDriver`, as everywhere else a coder node is
// driven: a harness SDK speaks its own wire, so the mock provider never sees it
// and the scripted driver `src/harness.ts` emits is the test surface.
//
// Usage: node coder-fanout.mjs <generated project directory> <scratch dir>
// Output: one JSON object, read by `generated_code_gates.rs`.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch] = process.argv;
if (project === undefined || scratch === undefined) {
  throw new Error("usage: node coder-fanout.mjs <generated project directory> <scratch dir>");
}

fs.mkdirSync(scratch, { recursive: true });
const data = path.resolve(scratch, "data");
fs.mkdirSync(data, { recursive: true });

// The two checkouts the items carry. They exist before the run because
// provisioning source control is a step a graph writes, never a guess the
// adapter makes — which is exactly what `fresh` does *not* do for the other
// node (PRD resolved q61 ruling c).
const worktrees = ["alpha", "beta"].map((name) => {
  const directory = path.resolve(scratch, "checkouts", name);
  fs.mkdirSync(directory, { recursive: true });
  return directory;
});

// The composition's own `${ENV}`, set before the barrel loads: that is what
// "process start" means for a generated project, and a missing one is refused
// there (PRD 5.9).
process.env["ANTHROPIC_API_KEY"] = "fanout-key";
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const harness = await import(pathToFileURL(path.resolve(project, "src/harness.ts")).href);

/** What each run was handed, in the order the driver was called. */
const handed = [];

/** What the summarising node's attempts found in their directory, in order. */
const found = [];

/** One answering script for a dispatched `implement` run. */
function implemented(summary) {
  return [
    { source: { type: "system", subtype: "init" } },
    { source: { type: "assistant" }, tap: { kind: "turn", usage: { inputTokens: 10 } } },
    { source: { type: "result", subtype: "success" }, tap: { kind: "settled", cost: {} } },
    { source: { type: "structured_output" }, tap: { kind: "output", value: { summary } } },
  ];
}

/** How many runs this driver has made **for one node**. */
const made = new Map();

// One driver for both nodes, which is what a composition binding one harness
// gets: it branches on the node the run belongs to, exactly as the graph does.
// The attempt is counted per node rather than taken from the driver's own
// call ordinal, which is a count over every run the registration served.
const stub = harness.scriptedDriver("cc", (run) => {
  const attempt = (made.get(run.node) ?? 0) + 1;
  made.set(run.node, attempt);
  handed.push({ node: run.node, attempt, workspace: run.workspace });
  if (run.node === "flow.fix.implement") {
    // Written into the directory this dispatch was given, so the answer is
    // evidence about the path and not only about the string: a run that was
    // handed the wrong checkout leaves its file in the wrong one.
    fs.writeFileSync(path.join(run.workspace, "touched.txt"), run.input);
    return implemented(`changed ${path.basename(run.workspace)}`);
  }
  // …and the summarising node, whose first attempt wrecks its own directory
  // and fails. `readdirSync` is taken *before* anything is written, so what it
  // reports is what the attempt started with — and `wrote` says the wreck was
  // real, which is what makes the second attempt's empty listing mean
  // something.
  const started = fs.readdirSync(run.workspace).sort();
  const leftover = path.join(run.workspace, `leftover-${attempt}.txt`);
  fs.writeFileSync(leftover, "half a run");
  found.push({ attempt, entries: started, wrote: fs.existsSync(leftover) });
  if (attempt === 1) {
    return [
      { source: { type: "assistant" }, tap: { kind: "turn" } },
      { source: { type: "error" }, tap: { kind: "error", message: "the note was not written" } },
    ];
  }
  return [
    { source: { type: "assistant" }, tap: { kind: "turn" } },
    { source: { type: "result", subtype: "success" }, tap: { kind: "settled", cost: {} } },
    { source: { type: "structured_output" }, tap: { kind: "output", value: { note: "two changes" } } },
  ];
});
runtime.registerHarnessDriver("cc", stub.driver);

const { runFlow } = await import(pathToFileURL(path.resolve(project, "src/index.ts")).href);

// The execution id is fixed rather than generated, because half of what this
// gate reads is a **path** derived from it: a `workspace: fresh` directory is
// `<data>/workspaces/<execution>/<instance path>`, and an assertion about a
// deterministic name has to know both halves.
const run = await runFlow(
  "flow.batch",
  {
    tasks: [
      { goal: "fix the parser", worktree: worktrees[0] },
      { goal: "fix the lexer", worktree: worktrees[1] },
    ],
  },
  { executionId: "exec_fanout" },
);

/** The harness records one entry carries, with the field this gate is about. */
function recorded(entry) {
  return (entry.harness ?? []).map((record) => ({
    outcome: record.outcome,
    workspace: record.workspace,
  }));
}

// The dispatched instances' entries live under the map's dispatch records, and
// the summarising node's is a top-level entry — which is the trace's own
// nesting (`docs/trace.md` §5) rather than anything this ruling changed.
const dispatched = (run.trace.find((entry) => entry.node === "work")?.dispatches ?? []).map(
  (dispatch) => ({
    index: dispatch.index,
    entries: (dispatch.inner ?? []).map((entry) => ({
      node: entry.node,
      harness: recorded(entry),
    })),
  }),
);

const summarise = run.trace.find((entry) => entry.node === "summarise");

process.stdout.write(
  JSON.stringify(
    {
      outputs: run.outputs,
      // What each run was handed, and what the items asked for beside it.
      handed,
      worktrees,
      dispatched,
      summarise: {
        outcome: summarise?.outcome ?? null,
        attempts: summarise?.attempts ?? null,
        harness: recorded(summarise ?? {}),
      },
      // The directory `fresh` derived: the execution id is the one this run
      // was started with and the frame is the node's, so the whole path is a
      // derivation rather than a choice (grammar 9.4).
      fresh: {
        expected: path.join(data, "workspaces", "exec_fanout", "summarise", "0"),
        found,
        // …and what is left of it once the run settles, which is the lifetime
        // it inherits by sitting under the execution's own scratch: the whole
        // tree goes, exactly as a built-in's defaulted workspace does.
        standing: fs.existsSync(path.join(data, "workspaces", "exec_fanout")),
      },
    },
    null,
    1,
  ),
);
