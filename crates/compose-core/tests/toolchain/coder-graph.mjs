// A compiled graph that carries `coder:` nodes, **run** (grammar 8.9, PRD
// resolved q57).
//
// `coder-runs.mjs` drives `runtime.runCoder` directly, which is the adapter and
// nothing above it: the config map, the journal, the stream tap and the output
// gate. Everything *around* a coder node is a different body of code — the node
// wrapper that assembles a trace entry, the policy chain that decides what a
// failed run does, the `writes:` that lands an answer in state, and the routers
// that read `<node>.output.<field>` — and none of it runs until a real graph
// runs. That is what this drives, and it is the same golden project: `runFlow`
// off `src/index.ts`, over `src/graph.ts`'s own bindings.
//
// **The seam is `registerHarnessDriver`.** A harness SDK speaks its own wire, so
// the mock provider — deliberately — never sees it and there is no endpoint to
// point a compiled graph at. `src/graph.ts` hands `runCoder` the emitted
// registry, which a test cannot reach, so the runtime consults a host's
// registration first and this registers the scripted driver `src/harness.ts`
// already emits into every project. Nothing else about the run is stubbed.
//
// Two runs of `flow.patch`, because the two things a node wrapper does with a
// harness run are its two endings:
//
//   * **the cycle** — `implement` answers, `review` says `revise`, the back edge
//     runs `implement` again and the second `review` approves. What this decides
//     is that a run's record reaches its node's trace entry, that a coder node's
//     `writes:` lands in state, that an edge guard reads `review.output.verdict`
//     off a harness answer, and that the flow's `outputs:` are what the last
//     implementing run said;
//   * **the fallback** — `review`'s run fails, and its node's
//     `on_error: { fallback: end }` routes the graph out through the target it
//     names. The failed run's record has to be on that entry: it rides out on
//     the failure rather than through the answer, which is the one path where a
//     record and an entry meet through the error.
//
// Usage: node coder-graph.mjs <generated project directory> <scratch dir>
// Output: one JSON object, read by `generated_code_gates.rs`.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch] = process.argv;
if (project === undefined || scratch === undefined) {
  throw new Error("usage: node coder-graph.mjs <generated project directory> <scratch dir>");
}

fs.mkdirSync(scratch, { recursive: true });
const workspace = path.join(scratch, "checkout");
fs.mkdirSync(workspace, { recursive: true });

// The composition's own `${ENV}` references, set before the barrel is imported:
// loading it is what "process start" means for a generated project, and a
// missing variable is refused there (PRD 5.9).
process.env["REPO_ROOT"] = workspace;
process.env["ANTHROPIC_API_KEY"] = "harness-key-anthropic";
process.env["OPENAI_API_KEY"] = "harness-key-openai";
process.env["AGENT_COMPOSE_DATA"] = path.join(scratch, "data");

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const harness = await import(pathToFileURL(path.resolve(project, "src/harness.ts")).href);

/** Which of the two runs below is in flight — the scripts branch on it. */
const phase = { name: "cycle" };

/** One `cc` run that answers, with the summary this pass reports. */
function implemented(summary) {
  return [
    { source: { type: "system", subtype: "init" } },
    {
      source: { type: "assistant" },
      tap: { kind: "turn", usage: { inputTokens: 200, outputTokens: 60 } },
    },
    {
      source: { type: "tool_result", tool: "Edit" },
      tap: { kind: "tool", name: "Edit", outcome: "completed" },
    },
    {
      source: { type: "result", subtype: "success" },
      tap: { kind: "settled", cost: { inputTokens: 200, outputTokens: 60, usd: 0.02 } },
    },
    {
      source: { type: "structured_output" },
      tap: { kind: "output", value: { summary, touched: ["src/lib.rs"] } },
    },
  ];
}

/** …and one `codex` run that answers with a verdict. */
function reviewed(verdict, feedback) {
  return [
    { source: { type: "turn.completed" }, tap: { kind: "turn", usage: { inputTokens: 90 } } },
    {
      source: { type: "item.completed", item: { type: "command_execution" } },
      tap: { kind: "tool", name: "command_execution", outcome: "completed" },
    },
    { source: { type: "agent-compose.settled" }, tap: { kind: "settled", cost: {} } },
    {
      source: { type: "agent-compose.agent_message" },
      tap: { kind: "output", value: { verdict, feedback } },
    },
  ];
}

// The two drivers, registered once and consulted by every `coder:` node of the
// composition. The scripts are functions of the attempt, which is how one
// registration serves both runs and both passes of the cycle.
const implementer = harness.scriptedDriver("cc", (_run, attempt) => implemented(`pass ${attempt}`));
const reviewer = harness.scriptedDriver("codex", (_run, attempt) => {
  if (phase.name === "fallback") {
    return [
      { source: { type: "turn.completed" }, tap: { kind: "turn" } },
      {
        source: { type: "error" },
        tap: { kind: "error", message: "the sandbox refused to start" },
      },
    ];
  }
  return attempt === 1 ? reviewed("revise", "tighten the error path") : reviewed("approve", "");
});
runtime.registerHarnessDriver("cc", implementer.driver);
runtime.registerHarnessDriver("codex", reviewer.driver);

const { runFlow } = await import(pathToFileURL(path.resolve(project, "src/index.ts")).href);

/** What one entry says about the runs its node made. */
function harnessOf(entry) {
  return (entry.harness ?? []).map((run) => ({
    harness: run.harness,
    outcome: run.outcome,
    sdk: run.sdk,
    model: run.model,
    modelId: run.modelId,
    turns: run.turns.length,
    toolCalls: (run.toolCalls ?? []).length,
    ...(run.error === undefined ? {} : { error: run.error }),
  }));
}

/** …and what the whole run says, entry by entry. */
function reported(run) {
  return run.trace.map((entry) => ({
    node: entry.node,
    outcome: entry.outcome,
    attempts: entry.attempts,
    harness: harnessOf(entry),
    // Where the graph went next: a routed entry says so through its edges, and
    // an entry that took an `on_error: { fallback: … }` says so through the
    // target it was scheduled at instead — the node's own edges are never
    // evaluated on that path (grammar 9.2).
    targets: entry.routing?.targets ?? null,
    fallback: entry.fallback ?? null,
  }));
}

const results = {};

// --- 1. The cycle ----------------------------------------------------------
{
  const run = await runFlow("flow.patch", { goal: "make the failing test pass" });
  results["cycle"] = {
    outputs: run.outputs,
    entries: reported(run),
    // The `codex` node was contained as its own `access:` says, and the `cc`
    // node as its own — read off the runs the graph really made, which is the
    // one place a binding and a driver meet.
    implementerAccess: implementer.runs[0].access,
    reviewerAccess: reviewer.runs[0].access,
    // The second implementing run was handed the feedback the first review
    // wrote, through `state.feedback` and the node's `input:` (grammar 8.0).
    secondInputCarriesFeedback: implementer.runs[1].input.includes("tighten the error path"),
    // And the workspace each run was given is the `${ENV}` the composition
    // wrote, resolved at the call.
    workspaces: [...new Set([...implementer.runs, ...reviewer.runs].map((run) => run.workspace))],
  };
}

// --- 2. The fallback -------------------------------------------------------
{
  phase.name = "fallback";
  const before = reviewer.runs.length;
  const run = await runFlow("flow.patch", { goal: "make the failing test pass" });
  results["fallback"] = {
    outputs: run.outputs,
    entries: reported(run),
    reviewRuns: reviewer.runs.length - before,
  };
}

process.stdout.write(JSON.stringify(results, null, 1));
