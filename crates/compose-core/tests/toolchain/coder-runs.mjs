// What one `coder:` node run does, driven against a generated project's own
// runtime and its own `src/harness.ts` (grammar 8.9, PRD resolved q57).
//
// A harness SDK speaks its own wire, so the mock provider — deliberately — never
// sees it and there is nothing for the acceptance suite to point a compiled
// graph at. The **scripted driver** is what this drives instead, and it is the
// golden's own: `runtime.runCoder` takes the registry as a parameter, so a test
// hands in a script and everything above the seam — the config map, the journal,
// the stream tap, the output gate — is the code a deployment really ships.
//
// Eleven claims, and each is invisible from outside a run:
//
//   * **the config map** — `workspace:` resolves its `${ENV}` at the call and an
//     empty one is refused; the environment is **scrubbed** to the declared
//     variables and `inherit_env: true` is the opt-in; the allowlist and the
//     access preset arrive as written. A driver that got none of this would
//     still answer, and the node would still complete;
//   * **the lowering** — the schema the *harness* is handed is the composition's
//     projected through this harness's own table, with every stripped bound
//     folded into a `description` (PRD resolved q55 rulings a and b). Only the
//     run object says what was really sent;
//   * **the gate** — and the other half of the same ruling: the answer is parsed
//     against the **full** declared schema, so an answer that overruns a bound
//     the harness never saw fails the node (ruling c);
//   * **the envelope's depth** — an event a driver taps nothing off reaches the
//     journal's payload and **not** the trace. That is where a harness's
//     subagent transcripts go, and a record that carried them would be the one
//     thing resolved q57 ruling a forbids;
//   * **the record** — turns with their usage, tool events in the three-outcome
//     vocabulary, the cost rollup and the `sdk@version` the manifest pinned;
//   * **one effect per run** — journaled once, whatever the run did inside;
//   * **replay** — a resumed generation consumes the recorded answer, the driver
//     is **not** run again, and the record says `replayed: true`;
//   * **the request identity** — and because replay does not re-gate, the whole
//     binding is in it: a narrowed `output:`, a moved `env:` reference, the
//     scrub turned off or a different model setting each diverge the resume
//     rather than handing the graph an answer to a question it stopped asking;
//   * **a failed run still reports** — a driver that throws leaves a record with
//     the turns it took, carried out on the failure, because an activity that
//     throws returns no answer;
//   * **a missing driver** — a harness with no driver in the registry is an
//     execution failure naming the requirement, never a silent skip;
//   * **a `retry:` ladder** — every attempt's run is on the node's entry, so a
//     later attempt answering does not erase the one that failed.
//
// Usage: node coder-runs.mjs <generated project directory> <scratch dir>
// Output: one JSON object, read by `generated_code_gates.rs`.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch] = process.argv;
if (project === undefined || scratch === undefined) {
  throw new Error("usage: node coder-runs.mjs <generated project directory> <scratch dir>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const harness = await import(pathToFileURL(path.resolve(project, "src/harness.ts")).href);
const journal = await import(pathToFileURL(path.resolve(project, "src/journal.ts")).href);

fs.mkdirSync(scratch, { recursive: true });
const workspace = path.join(scratch, "checkout");
fs.mkdirSync(workspace, { recursive: true });

/** A context with a signal nothing aborts and no journal behind it. */
function context(overrides = {}) {
  return {
    execution: { id: "exec_coder", session_key: undefined, item_index: undefined },
    signal: new AbortController().signal,
    node: "implement",
    harnessRuns: [],
    ...overrides,
  };
}

/** The emitted Zod for the answer these runs are gated against. */
const RESULT = {
  safeParse(value) {
    if (value === null || typeof value !== "object") {
      return { success: false, error: { issues: [{ path: [], message: "not an object" }] } };
    }
    const held = value;
    if (typeof held.summary !== "string") {
      return {
        success: false,
        error: { issues: [{ path: ["summary"], message: "expected a string" }] },
      };
    }
    if (!Array.isArray(held.touched) || held.touched.length > 2) {
      return {
        success: false,
        error: {
          issues: [{ path: ["touched"], message: "expected at most 2 items" }],
        },
      };
    }
    return { success: true, data: held };
  },
};

/** The declared `output:`, as JSON Schema — the column the gate parses. */
const SCHEMA = {
  type: "object",
  additionalProperties: false,
  properties: {
    summary: { type: "string" },
    touched: {
      type: "array",
      maxItems: 2,
      items: { type: "string", minLength: 1 },
    },
  },
  required: ["summary", "touched"],
};

/** One coder binding, with whatever this case overrides. */
function binding(overrides = {}) {
  return {
    node: "flow.patch.node.implement",
    harness: "cc",
    model: "model.implementer",
    modelId: "claude-sonnet-4-5",
    modelSettings: { thinking: { budget_tokens: 8000 } },
    prompt: "Fix the failing test.",
    workspace: [{ env: "CODER_WORKSPACE", site: "flow.patch.node.implement.workspace" }],
    access: "workspace_write",
    allowTools: ["Bash", "Read"],
    env: [
      { name: "PATH", value: ["/usr/bin:/bin"] },
      { name: "TOKEN", value: [{ env: "CODER_TOKEN", site: "…env.TOKEN" }] },
    ],
    settings: { max_turns: 40 },
    schema: SCHEMA,
    result: RESULT,
    ...overrides,
  };
}

/** The answer a healthy run gives, as a script the stub yields. */
function script(answer) {
  return [
    { source: { type: "system", subtype: "init" } },
    {
      source: { type: "assistant", turn: 0 },
      tap: { kind: "turn", usage: { inputTokens: 120, outputTokens: 40 } },
    },
    {
      source: { type: "tool_result", tool: "Read" },
      tap: { kind: "tool", name: "Read", outcome: "completed" },
    },
    // A subagent's own turn: yielded so the journal's payload has it, tapped
    // with nothing so the envelope does not (PRD resolved q57 ruling a).
    { source: { type: "assistant", parent_tool_use_id: "toolu_sub", turn: "nested" } },
    {
      source: { type: "tool_result", tool: "Write", is_error: true },
      tap: { kind: "tool", name: "Write", outcome: "failed", error: "read-only file system" },
    },
    {
      source: { type: "permission_denied", tool: "Bash" },
      tap: { kind: "tool", name: "Bash", outcome: "refused", error: "not in `allow_tools`" },
    },
    {
      source: { type: "result", subtype: "success" },
      tap: {
        kind: "settled",
        cost: { inputTokens: 120, outputTokens: 40, usd: 0.0125 },
        extra: { subtype: "success", stopReason: "end_turn" },
      },
    },
    { source: { type: "structured_output" }, tap: { kind: "output", value: answer } },
  ];
}

const results = {};

// --- 1. The config map, the lowering, the record, and the depth rule --------
{
  process.env["CODER_WORKSPACE"] = workspace;
  process.env["CODER_TOKEN"] = "shh";
  process.env["LEAKED"] = "should-not-travel";
  const stub = harness.scriptedDriver("cc", script({ summary: "done", touched: ["a.ts"] }));
  const held = context();
  const answer = await runtime.runCoder(binding(), { goal: "fix it" }, held, { cc: stub.driver });
  const run = stub.runs[0];
  const record = answer.harness[0];
  results["configMap"] = {
    runs: stub.runs.length,
    workspace: run.workspace === workspace,
    access: run.access,
    allowTools: run.allowTools,
    // The scrubbed child: exactly the declared names, with the `${ENV}` one
    // resolved and nothing this process holds beside them.
    env: Object.keys(run.env).sort(),
    token: run.env["TOKEN"],
    leaked: Object.hasOwn(run.env, "LEAKED"),
    inheritEnv: run.inheritEnv,
    // The user turn is the node's input rendered exactly as an agent's is.
    input: run.input,
    instructions: run.instructions,
    model: run.model,
    modelSettings: run.modelSettings,
  };
  results["lowering"] = {
    // The bound came off the wire schema…
    hasMaxItems: Object.hasOwn(run.schema.properties.touched, "maxItems"),
    hasMinLength: Object.hasOwn(run.schema.properties.touched.items, "minLength"),
    // …and was folded into a description the model still reads.
    description: run.schema.properties.touched.description ?? null,
    itemsDescription: run.schema.properties.touched.items.description ?? null,
    // …while the binding's own schema is untouched: the gate parses that one.
    bindingKeepsMaxItems: Object.hasOwn(SCHEMA.properties.touched, "maxItems"),
    // The table this harness projects through, read off the runtime itself.
    away: [...runtime.harnessLoweredAway("cc")].sort(),
    codexAway: [...runtime.harnessLoweredAway("codex")].sort(),
  };
  results["record"] = {
    output: answer.output,
    harness: record.harness,
    sdk: record.sdk,
    model: record.model,
    modelId: record.modelId,
    outcome: record.outcome,
    turns: record.turns,
    toolCalls: record.toolCalls,
    cost: record.cost,
    extra: record.extra,
    replayed: record.replayed ?? null,
    collected: held.harnessRuns.length,
    // The envelope carries three tool events and no fourth: the subagent's own
    // turn is not one, and neither is the `system` event.
    turnCount: record.turns.length,
  };
}

// --- 2. The output gate parses the FULL schema ------------------------------
{
  const stub = harness.scriptedDriver(
    "cc",
    script({ summary: "done", touched: ["a.ts", "b.ts", "c.ts"] }),
  );
  const held = context();
  let failure;
  try {
    await runtime.runCoder(binding(), { goal: "fix it" }, held, { cc: stub.driver });
  } catch (error) {
    failure = error;
  }
  const record = runtime.harnessRecordOf(failure);
  results["gate"] = {
    threw: failure !== undefined,
    name: failure?.name ?? null,
    // The run really happened, and its record rides out on the failure — turns
    // and all — because an activity that throws returns no answer.
    recordOutcome: record?.outcome ?? null,
    recordTurns: record?.turns.length ?? null,
    recordToolCalls: record?.toolCalls?.length ?? null,
    // …and the collector holds it all the same: a `retry:` ladder's later,
    // successful attempt must not erase the attempt that failed.
    collected: held.harnessRuns.length,
    collectedOutcome: held.harnessRuns[0]?.outcome ?? null,
    // The harness was asked without the bound and the answer was held to it.
    sentMaxItems: Object.hasOwn(stub.runs[0].schema.properties.touched, "maxItems"),
  };
}

// --- 3. A run that produced no answer at all -------------------------------
{
  const stub = harness.scriptedDriver("cc", [
    { source: { type: "assistant" }, tap: { kind: "turn" } },
    { source: { type: "result" }, tap: { kind: "settled", cost: { inputTokens: 5 } } },
  ]);
  let failure;
  try {
    await runtime.runCoder(binding(), { goal: "fix it" }, context(), { cc: stub.driver });
  } catch (error) {
    failure = error;
  }
  results["noAnswer"] = {
    name: failure?.name ?? null,
    saysWhat: `${failure?.message ?? ""}`.includes("no structured output"),
    recordOutcome: runtime.harnessRecordOf(failure)?.outcome ?? null,
  };
}

// --- 4. A harness that reported a fatal error of its own -------------------
{
  const stub = harness.scriptedDriver("codex", [
    { source: { type: "turn.completed" }, tap: { kind: "turn" } },
    { source: { type: "error" }, tap: { kind: "error", message: "the sandbox refused to start" } },
  ]);
  let failure;
  try {
    await runtime.runCoder(
      binding({ harness: "codex" }),
      { goal: "fix it" },
      context(),
      { codex: stub.driver },
    );
  } catch (error) {
    failure = error;
  }
  const record = runtime.harnessRecordOf(failure);
  results["fatal"] = {
    name: failure?.name ?? null,
    quotes: `${failure?.message ?? ""}`.includes("the sandbox refused to start"),
    recordOutcome: record?.outcome ?? null,
    recordError: record?.error ?? null,
    // The codex table is the one that was read, not the cc one.
    away: [...runtime.harnessLoweredAway("codex")].sort(),
  };
}

// --- 5. A harness with no driver -------------------------------------------
{
  let failure;
  try {
    await runtime.runCoder(binding({ harness: "codex" }), {}, context(), {});
  } catch (error) {
    failure = error;
  }
  results["missingDriver"] = {
    name: failure?.name ?? null,
    namesTheHarness: `${failure?.message ?? ""}`.includes("harness: codex"),
    namesTheRequirement: `${failure?.message ?? ""}`.includes("package.json"),
  };
}

// --- 6. An `${ENV}` workspace that resolved empty --------------------------
{
  process.env["EMPTY_WORKSPACE"] = "";
  const stub = harness.scriptedDriver("cc", script({ summary: "x", touched: [] }));
  let failure;
  try {
    await runtime.runCoder(
      binding({ workspace: [{ env: "EMPTY_WORKSPACE", site: "…workspace" }] }),
      {},
      context(),
      { cc: stub.driver },
    );
  } catch (error) {
    failure = error;
  }
  results["emptyWorkspace"] = {
    threw: failure !== undefined,
    // Quoted **as written**, never resolved (`docs/trace.md` §11.1).
    asWritten: `${failure?.message ?? ""}`.includes("${EMPTY_WORKSPACE}"),
    ranTheDriver: stub.runs.length > 0,
  };
}

// --- 7. `inherit_env: true` is the opt-in ----------------------------------
{
  const stub = harness.scriptedDriver("cc", script({ summary: "x", touched: [] }));
  await runtime.runCoder(binding({ inheritEnv: true }), {}, context(), { cc: stub.driver });
  const run = stub.runs[0];
  results["inherited"] = {
    inheritEnv: run.inheritEnv,
    sawThisProcess: run.env["LEAKED"] === "should-not-travel",
    // The declared entries still layer over whatever was inherited.
    token: run.env["TOKEN"],
  };
}

// --- 8. A `retry:` ladder: every attempt's run is on the entry ------------
{
  // One node execution, two attempts: `runActivity` hands every attempt the
  // same context, so the collector is what makes the first run survive the
  // second answering (`docs/trace.md` §7.6, PRD resolved q57).
  const held = context();
  const failing = harness.scriptedDriver("cc", [
    { source: { type: "assistant" }, tap: { kind: "turn" } },
    { source: { type: "error" }, tap: { kind: "error", message: "the workspace was busy" } },
  ]);
  let first;
  try {
    await runtime.runCoder(binding(), { goal: "fix it" }, held, { cc: failing.driver });
  } catch (error) {
    first = error;
  }
  const answering = harness.scriptedDriver("cc", script({ summary: "second", touched: [] }));
  const answer = await runtime.runCoder(binding(), { goal: "fix it" }, held, {
    cc: answering.driver,
  });
  results["ladder"] = {
    firstThrew: first !== undefined,
    // Both runs, in the order they were made.
    collected: held.harnessRuns.map((run) => run.outcome),
    // …and the answer carries only the attempt it came out of, which is why the
    // collector has to exist.
    answered: answer.harness.map((run) => run.outcome),
    // The failed run's record rides out on the failure too, and it is the same
    // object — `runNode` reconciles the two by identity.
    sameObject: runtime.harnessRecordOf(first) === held.harnessRuns[0],
  };
}

// --- 9. One journaled effect, and a replay that consumes it ----------------
{
  process.env["AGENT_COMPOSE_DATA"] = path.join(scratch, "data");
  const held = await journal.openJournal();
  const answer = { summary: "journaled", touched: ["one.ts"] };

  // The live generation: one effect, recorded.
  journal.openSession("exec_journal", held, false);
  const first = harness.scriptedDriver("cc", script(answer));
  const live = context({
    execution: { id: "exec_journal" },
    effects: journal.recorderFor("exec_journal", "flow.patch/implement/0"),
  });
  const written = await runtime.runCoder(binding(), { goal: "fix it" }, live, {
    cc: first.driver,
  });
  journal.closeSession("exec_journal");

  const records = held.effectsUnder("exec_journal", "flow.patch/implement/0");
  const harnessRecords = records.filter((record) => record.kind === "harness");

  // …and the resumed one: same site, same ordinal, no driver run.
  journal.openSession("exec_journal", held, true);
  const second = harness.scriptedDriver("cc", script({ summary: "SHOULD NOT RUN", touched: [] }));
  const resumed = context({
    execution: { id: "exec_journal" },
    effects: journal.recorderFor("exec_journal", "flow.patch/implement/0"),
  });
  const replayed = await runtime.runCoder(binding(), { goal: "fix it" }, resumed, {
    cc: second.driver,
  });
  journal.closeSession("exec_journal");

  const payload = JSON.parse(
    harnessRecords[0]?.outcome?.kind === "value"
      ? JSON.stringify(harnessRecords[0].outcome.value)
      : "null",
  );
  results["journal"] = {
    effects: harnessRecords.length,
    kind: harnessRecords[0]?.kind ?? null,
    key: harnessRecords[0]?.key ?? null,
    // The three halves of what one run journals.
    keptOutput: payload?.output ?? null,
    keptRecordOutcome: payload?.record?.outcome ?? null,
    // The **whole** stream, subagent event included — the private payload.
    streamLength: payload?.stream?.length ?? null,
    streamHasSubagent:
      (payload?.stream ?? []).some((event) => event?.parent_tool_use_id === "toolu_sub") ?? false,
    // …which the envelope does not carry.
    envelopeToolCalls: written.harness[0].toolCalls.length,
    // The resume consumed the answer and ran nothing.
    replayedOutput: replayed.output,
    replayedDriverRuns: second.runs.length,
    replayedFlag: replayed.harness[0].replayed ?? null,
    liveFlag: written.harness[0].replayed ?? null,
  };
}

// --- 10. The request identity is the whole binding -------------------------
//
// `docs/durability.md` §3.9, and the reason it has to be the whole binding is
// section 9 above: **a replay does not re-gate.** The held answer is returned
// and `performHarnessRun` — where the `output:` gate lives — never runs, so a
// binding the composition has since edited either diverges on the request
// identity or does not diverge at all, and the graph goes on with an answer to a
// question this build stopped asking.
//
// Each case is one edit, resumed against a journal written under the unedited
// binding. The control comes first: the same binding replays clean, so a
// divergence below is the edit rather than a request that never matched.
{
  process.env["AGENT_COMPOSE_DATA"] = path.join(scratch, "identity");
  process.env["CODER_WORKSPACE"] = workspace;
  // Set, and set to the **same** value the entry it replaces resolves to: what
  // this case is about is the reference the composition wrote, so a divergence
  // that came from the resolved value changing would prove the opposite of the
  // claim (`docs/trace.md` §11.1 keeps resolved values out of the identity).
  process.env["CODER_TOKEN_V2"] = process.env["CODER_TOKEN"];
  const held = await journal.openJournal();
  const site = "flow.patch/implement/0";
  const answer = { summary: "recorded", touched: ["one.ts"] };

  journal.openSession("exec_identity", held, false);
  const live = harness.scriptedDriver("cc", script(answer));
  await runtime.runCoder(
    binding(),
    { goal: "fix it" },
    context({
      execution: { id: "exec_identity" },
      effects: journal.recorderFor("exec_identity", site),
    }),
    { cc: live.driver },
  );
  journal.closeSession("exec_identity");

  /** Resume the recorded effect under `overrides`, and say how it went. */
  const resumeWith = async (overrides) => {
    journal.openSession("exec_identity", held, true);
    const stub = harness.scriptedDriver("cc", script({ summary: "SHOULD NOT RUN", touched: [] }));
    let failure;
    let output;
    try {
      const replayed = await runtime.runCoder(
        binding(overrides),
        { goal: "fix it" },
        context({
          execution: { id: "exec_identity" },
          effects: journal.recorderFor("exec_identity", site),
        }),
        { cc: stub.driver },
      );
      output = replayed.output;
    } catch (error) {
      failure = error;
    }
    journal.closeSession("exec_identity");
    return {
      diverged: runtime.divergenceOf(failure) !== undefined,
      output: output ?? null,
      ranTheDriver: stub.runs.length > 0,
    };
  };

  results["identity"] = {
    untouched: await resumeWith({}),
    // The narrowed `output:`: `touched` is gone from the contract, and the
    // recorded answer carries it. Nothing downstream would notice — the gate is
    // behind the replay arm — so this is the divergence or nothing.
    narrowedOutput: await resumeWith({
      schema: {
        type: "object",
        additionalProperties: false,
        properties: { summary: { type: "string" } },
        required: ["summary"],
      },
    }),
    // An `env:` entry pointed at a different variable. The **name written**
    // changes what the run was handed; the resolved value is not in the identity
    // and never will be (`docs/trace.md` §11.1).
    movedEnv: await resumeWith({
      env: [
        { name: "PATH", value: ["/usr/bin:/bin"] },
        { name: "TOKEN", value: [{ env: "CODER_TOKEN_V2", site: "…env.TOKEN" }] },
      ],
    }),
    // The scrub turned off, which is the widest edit `env:` has.
    inheritedNow: await resumeWith({ inheritEnv: true }),
    // A model setting the harness takes: a different thinking budget is a
    // different run (Decision D141).
    movedModelSettings: await resumeWith({
      modelSettings: { thinking: { budget_tokens: 32000 } },
    }),
  };
}

process.stdout.write(`${JSON.stringify(results, null, 2)}\n`);
