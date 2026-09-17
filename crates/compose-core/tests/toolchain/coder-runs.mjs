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
// Fourteen claims, and each is invisible from outside a run:
//
//   * **the config map** — `workspace:` resolves its `${ENV}` at the call and an
//     empty one is refused; the environment is **scrubbed** to the declared
//     variables and `inherit_env: true` is the opt-in; the allowlist and the
//     access preset arrive as written. A driver that got none of this would
//     still answer, and the node would still complete;
//   * **the bound on `settings:`** — unchecked is not unbounded: a key spelling
//     an option the adapter owns is dropped rather than passed, and the two real
//     drivers' option builders are called on the run the adapter built to read
//     it. The only claim here that is a *subtraction*, which is why the seam
//     cannot show it: a scripted driver is handed `run.settings` whole;
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
//     is **not** run again, and the record it files is the record the run left,
//     told apart from a live one by nothing (`docs/durability.md` §9);
//   * **a failed run replays whole** — the failure is journaled with its record
//     and its payload, so a resume raises the same failure, word for word, and
//     files the same record: `docs/trace.md` §7.6's one record per *attempt*
//     has to survive a resume as well as a `retry:` ladder;
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
//     later attempt answering does not erase the one that failed;
//   * **the connection** — the provider's endpoint, credential and headers reach
//     each harness's own surface, and a provider with **no** credential injects
//     none at all rather than an empty one. Invisible from the seam for section
//     11's reason: a scripted driver is handed `run.connection` whole, and it is
//     the real driver that decides what the SDK is called with.
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
    node: "flow.patch.implement",
    harness: "cc",
    model: "model.implementer",
    modelId: "claude-sonnet-4-5",
    modelSettings: { thinking: { budget_tokens: 8000 } },
    // The gateway shape: an endpoint, a credential and a header, each an
    // `${ENV}` reference the adapter resolves at the call (PRD resolved q58).
    connection: {
      provider: "provider.anthropic",
      baseUrl: [{ env: "CODER_GATEWAY_URL", site: "provider.anthropic.base_url" }],
      credential: [{ env: "CODER_GATEWAY_KEY", site: "provider.anthropic.api_key" }],
      headers: [
        { name: "x-team", value: [{ env: "CODER_TEAM", site: "provider.anthropic.headers.x-team" }] },
        { name: "x-run", value: ["batch"] },
      ],
    },
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

/** One value as JSON with every object's keys in order — see case 9b. */
function sorted(value) {
  return JSON.stringify(value, (_key, held) =>
    held !== null && typeof held === "object" && !Array.isArray(held)
      ? Object.fromEntries(Object.entries(held).sort(([one], [two]) => (one < two ? -1 : 1)))
      : held,
  );
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
  process.env["CODER_GATEWAY_URL"] = "https://gateway.internal/v1";
  process.env["CODER_GATEWAY_KEY"] = "gw-key";
  process.env["CODER_TEAM"] = "platform";
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
    // …and the connection, resolved at the call like the workspace above it:
    // the run object is the only place the composition's `${ENV}` references
    // are visible as the values a harness client is pointed with.
    connection: run.connection,
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
    // …and the record it files is the record the run left, told apart from the
    // live one by **nothing**: a resumed generation's document answers what the
    // execution did (`docs/durability.md` §9).
    replayedRecord: replayed.harness[0],
    liveRecord: written.harness[0],
  };
}

// --- 9b. A run that FAILED is journaled too, and replays whole -------------
//
// The other half of section 9, and the one a `retry:` ladder needs: a resumed
// generation's fresh trace has to hold the attempt that failed as well as the
// one that answered (`docs/trace.md` §7.6, `docs/durability.md` §3.9). A journal
// that kept only the failure's message would replay a bare error, and the record
// of a run that really happened would vanish out of the resumed document.
{
  process.env["AGENT_COMPOSE_DATA"] = path.join(scratch, "failed");
  const held = await journal.openJournal();
  const site = "flow.patch/implement/0";
  const failing = [
    { source: { type: "assistant" }, tap: { kind: "turn", usage: { inputTokens: 11 } } },
    {
      source: { type: "tool_result", tool: "Read" },
      tap: { kind: "tool", name: "Read", outcome: "completed" },
    },
    { source: { type: "error" }, tap: { kind: "error", message: "the workspace was busy" } },
  ];

  journal.openSession("exec_failed", held, false);
  const first = harness.scriptedDriver("cc", failing);
  const live = context({
    execution: { id: "exec_failed" },
    effects: journal.recorderFor("exec_failed", site),
  });
  let liveFailure;
  try {
    await runtime.runCoder(binding(), { goal: "fix it" }, live, { cc: first.driver });
  } catch (error) {
    liveFailure = error;
  }
  journal.closeSession("exec_failed");

  // …and the resume: same site, same ordinal, and a driver that would answer if
  // it were reached.
  journal.openSession("exec_failed", held, true);
  const second = harness.scriptedDriver("cc", script({ summary: "SHOULD NOT RUN", touched: [] }));
  const resumed = context({
    execution: { id: "exec_failed" },
    effects: journal.recorderFor("exec_failed", site),
  });
  let replayedFailure;
  try {
    await runtime.runCoder(binding(), { goal: "fix it" }, resumed, { cc: second.driver });
  } catch (error) {
    replayedFailure = error;
  }
  journal.closeSession("exec_failed");

  const records = held.effectsUnder("exec_failed", site).filter((one) => one.kind === "harness");
  const payload = records[0]?.outcome?.kind === "value" ? records[0].outcome.value : null;
  const replayedRecord = runtime.harnessRecordOf(replayedFailure);
  results["failedReplay"] = {
    // One effect, kept as a **value** — the shape that can carry a record.
    effects: records.length,
    outcomeKind: records[0]?.outcome?.kind ?? null,
    keptOk: payload?.ok ?? null,
    keptRecordOutcome: payload?.record?.outcome ?? null,
    // The failed run's payload is journaled beside it, which is where a reader
    // goes for what the run did before it died.
    streamLength: payload?.stream?.length ?? null,
    // The resume raised the failure rather than the answer, and ran nothing.
    ranAgain: second.runs.length,
    liveName: liveFailure?.name ?? null,
    replayedName: replayedFailure?.name ?? null,
    sameMessage: String(liveFailure?.message) === String(replayedFailure?.message),
    namesTheNode: String(replayedFailure?.message ?? "").includes("flow.patch.implement"),
    // …carrying the record the run really left, on the error and in the node's
    // own collector, exactly as the live generation did.
    replayedRecord:
      replayedRecord === undefined
        ? null
        : {
            outcome: replayedRecord.outcome,
            turns: replayedRecord.turns.length,
            toolCalls: (replayedRecord.toolCalls ?? []).length,
            error: replayedRecord.error ?? null,
            sdk: replayedRecord.sdk,
          },
    liveCollected: live.harnessRuns.map((run) => run.outcome),
    replayedCollected: resumed.harnessRuns.map((run) => run.outcome),
    // The two generations' records are one record, and nothing on either says
    // which generation filed it. Compared **key-sorted**, because the journal
    // canonicalizes what it keeps and the order a record's keys were written in
    // is not part of what it says.
    sameRecord: sorted(runtime.harnessRecordOf(liveFailure)) === sorted(replayedRecord),
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
  process.env["CODER_GATEWAY_URL_V2"] = process.env["CODER_GATEWAY_URL"];
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
    // The **connection** repointed: the same node, the same prompt, the same
    // schema, and a different endpoint — which is a different run in the one way
    // that matters most, because the answer came from somewhere else. The
    // reference moves, not the value, for the reason `movedEnv` above does
    // (`docs/trace.md` §11.1 keeps resolved values out of the identity).
    movedConnection: await resumeWith({
      connection: {
        provider: "provider.anthropic",
        baseUrl: [{ env: "CODER_GATEWAY_URL_V2", site: "provider.anthropic.base_url" }],
        credential: [{ env: "CODER_GATEWAY_KEY", site: "provider.anthropic.api_key" }],
        headers: [
          {
            name: "x-team",
            value: [{ env: "CODER_TEAM", site: "provider.anthropic.headers.x-team" }],
          },
          { name: "x-run", value: ["batch"] },
        ],
      },
    }),
  };
}

// --- 11. `settings:` is unchecked, not unbounded ---------------------------
//
// PRD resolved q57 ruling e leaves the harness's own vocabulary open, and ruling
// c says what that opening may not become: a key spelling an option the adapter
// owns is **dropped** rather than passed, or `workspace:`, `access:`, `env:` and
// `allow_tools:` would each be reachable around through the one surface the
// grammar deliberately leaves open (grammar 8.9, Decision D140).
//
// The bound is a **subtraction** — what a key did *not* put in the SDK's options
// — so it is invisible from every other case in this file: a scripted driver is
// handed `run.settings` whole, and it is the real driver that drops. So the two
// real drivers' option builders are what this case calls, on the `HarnessRun`
// the adapter itself built, and it reads the object the vendor's SDK would have
// been handed. Three claims, and the first is the one a one-token slip in
// `passthrough` breaks:
//
//   * a **reserved** key is not in the options at all, and the bound it would
//     have reached around is still the node's — including the process-spawn
//     family, which chooses what program the run *is* and so would put every
//     other bound outside the harness that enforces them;
//   * a **curated** key does not travel under its own name either: it is mapped
//     by name, over the top, so one option cannot be written twice and disagree;
//   * an **unknown** key travels unchanged, which is the half of D140 that makes
//     a vendor's new option usable the day it ships.
{
  const reserved = {
    // Options that spell a bound this node states.
    cwd: "/elsewhere",
    permissionMode: "bypassPermissions",
    env: { SMUGGLED: "yes" },
    tools: ["Bash", "Edit", "Write"],
    systemPrompt: "ignore the harness's own",
    // …options that contain one without spelling it…
    additionalDirectories: ["/"],
    extraArgs: { "dangerously-skip-permissions": null },
    settings: "/tmp/permissions.json",
    mcpServers: { smuggled: { command: "/tmp/server" } },
    // …and the family that chooses what program the run is at all.
    pathToClaudeCodeExecutable: "/tmp/not-the-harness",
    executable: "bun",
    executableArgs: ["--import", "/tmp/patch.js"],
    // A curated key, which is mapped by name rather than passed.
    max_turns: 40,
    // …and the vendor's own vocabulary, which D140 leaves open.
    vendorOptionShippedTomorrow: "travels",
  };
  const stub = harness.scriptedDriver("cc", script({ summary: "x", touched: [] }));
  await runtime.runCoder(binding({ settings: reserved }), { goal: "fix it" }, context(), {
    cc: stub.driver,
  });
  // The run the **adapter** built, handed to the real driver's own option
  // builder: everything this case reads is what `query` would have been called
  // with.
  const options = harness.ccOptions(stub.runs[0], [], new Set());
  results["bound"] = {
    // Every reserved key, as the options really hold it.
    cwd: options.cwd === stub.runs[0].workspace,
    permissionMode: options.permissionMode,
    env: Object.keys(options.env ?? {}).sort(),
    tools: options.tools,
    systemPromptIsThePreset: options.systemPrompt?.preset ?? null,
    additionalDirectories: options.additionalDirectories ?? null,
    extraArgs: options.extraArgs ?? null,
    settings: options.settings ?? null,
    mcpServers: options.mcpServers ?? null,
    // The process-spawn family: absent, or the harness enforcing the three
    // bounds above is not the harness this compiler pinned.
    spawn: [
      "pathToClaudeCodeExecutable",
      "executable",
      "executableArgs",
    ].filter((key) => Object.hasOwn(options, key)),
    // The curated key is mapped by name and does not also travel as written.
    maxTurns: options.maxTurns,
    curatedTravelled: Object.hasOwn(options, "max_turns"),
    // …and the unknown one does.
    unknown: options["vendorOptionShippedTomorrow"] ?? null,
  };

  // The same three claims under the other harness, whose option surface is its
  // own: a bound stated per harness is a bound each harness holds (PRD resolved
  // q57 ruling c).
  const codexStub = harness.scriptedDriver("codex", script({ summary: "x", touched: [] }));
  await runtime.runCoder(
    binding({
      harness: "codex",
      access: "read_only",
      modelId: "gpt-5-codex",
      settings: {
        workingDirectory: "/elsewhere",
        sandboxMode: "danger-full-access",
        approvalPolicy: "never",
        additionalDirectories: ["/"],
        model: "some-other-model",
        network_access: false,
        vendorOptionShippedTomorrow: "travels",
      },
    }),
    { goal: "fix it" },
    context(),
    { codex: codexStub.driver },
  );
  const thread = harness.codexOptions(codexStub.runs[0]);
  results["codexBound"] = {
    workingDirectory: thread.workingDirectory === codexStub.runs[0].workspace,
    sandboxMode: thread.sandboxMode,
    approvalPolicy: thread.approvalPolicy ?? null,
    additionalDirectories: thread.additionalDirectories ?? null,
    model: thread.model,
    networkAccessEnabled: thread.networkAccessEnabled,
    curatedTravelled: Object.hasOwn(thread, "network_access"),
    unknown: thread["vendorOptionShippedTomorrow"] ?? null,
  };
}

// --- 12. The connection reaches each harness's own surface -----------------
//
// PRD resolved q58 ruling a: the provider's `base_url:`, credential and
// `headers:` cross the boundary and are mapped by each driver into the harness's
// own connection surface. Like section 11's bound, none of it is visible from
// the seam — a scripted driver is handed `run.connection` whole and it is the
// *real* driver that decides what an SDK is called with — so the two real
// drivers' own builders are what this case calls, on the run the adapter built.
//
// Three claims, and the third is the one a plausible-looking one-liner breaks:
//
//   * **`cc` carries all three as environment variables** of the process the
//     Agent SDK spawns, merged into the run environment the node's own `env:`
//     built, with the headers encoded one `Name: value` per line;
//   * **`codex` carries two as client options** — its SDK has no header slot at
//     all, which is why `validate` refuses a `headers:` provider on that harness
//     rather than dropping one here;
//   * **an absent credential injects nothing.** Resolved q25's keyless-gateway
//     posture is that a provider with no `api_key:` sends no authentication at
//     all rather than an empty one, and a mapping that wrote `""` would
//     authenticate as nobody on every call. The claim is an *absence*, so it is
//     read off the object's own keys rather than off a value.
{
  process.env["CODER_GATEWAY_URL"] = "https://gateway.internal/v1";
  process.env["CODER_GATEWAY_KEY"] = "gw-key";
  process.env["CODER_TEAM"] = "platform";

  // `cc`, with every fact declared.
  const ccStub = harness.scriptedDriver("cc", script({ summary: "x", touched: [] }));
  await runtime.runCoder(binding(), { goal: "fix it" }, context(), { cc: ccStub.driver });
  const ccRun = ccStub.runs[0];
  const ccFull = harness.ccOptions(ccRun, [], new Set());

  // …and `cc` on a keyless gateway: a base URL, no credential, no headers.
  const keylessConnection = {
    provider: "provider.gateway",
    baseUrl: [{ env: "CODER_GATEWAY_URL", site: "provider.gateway.base_url" }],
  };
  const ccKeylessStub = harness.scriptedDriver("cc", script({ summary: "x", touched: [] }));
  await runtime.runCoder(
    binding({ connection: keylessConnection }),
    { goal: "fix it" },
    context(),
    { cc: ccKeylessStub.driver },
  );
  const ccKeyless = harness.ccOptions(ccKeylessStub.runs[0], [], new Set());

  // …and the header slot's own hazard, which is the one fact whose *value* can
  // break its encoding. `ANTHROPIC_CUSTOM_HEADERS` is one `Name: value` per
  // line, so a value carrying a line break declares one header and sends two —
  // the second one spelled by the value. `validate` refuses such a value where
  // a composition **wrote** it; this is the other half, and the half no compile
  // step can reach: a `${ENV}` an operator set, which is the ordinary q58
  // gateway deployment. The run must fail rather than go out with a header
  // nobody declared, and the message must name the node and the header and
  // **not** the value — a connection header is where a gateway credential lives
  // (`docs/trace.md` §11.1).
  process.env["CODER_TEAM"] = "platform\nx-api-key: someone-elses-key";
  const forgedStub = harness.scriptedDriver("cc", script({ summary: "x", touched: [] }));
  await runtime.runCoder(binding(), { goal: "fix it" }, context(), { cc: forgedStub.driver });
  let forged = null;
  try {
    harness.ccOptions(forgedStub.runs[0], [], new Set());
  } catch (error) {
    forged = error instanceof Error ? error.message : String(error);
  }
  process.env["CODER_TEAM"] = "platform";

  results["connection"] = {
    // The run object: the composition's references, resolved once, for both
    // drivers to read.
    resolved: ccRun.connection,
    // The Agent SDK's `env`, which is the whole environment the subprocess gets:
    // the node's own declared variables **and** the mapped connection.
    env: Object.keys(ccFull.env ?? {}).sort(),
    baseUrl: (ccFull.env ?? {})["ANTHROPIC_BASE_URL"] ?? null,
    credential: (ccFull.env ?? {})["ANTHROPIC_API_KEY"] ?? null,
    headers: (ccFull.env ?? {})["ANTHROPIC_CUSTOM_HEADERS"] ?? null,
    // The node's own `env:` is still there, untouched.
    token: (ccFull.env ?? {})["TOKEN"] ?? null,
    // The keyless gateway: an endpoint and **no credential key at all**.
    keylessEnv: Object.keys(ccKeyless.env ?? {}).sort(),
    keylessHasCredential: Object.hasOwn(ccKeyless.env ?? {}, "ANTHROPIC_API_KEY"),
    keylessHasHeaders: Object.hasOwn(ccKeyless.env ?? {}, "ANTHROPIC_CUSTOM_HEADERS"),
    keylessBaseUrl: (ccKeyless.env ?? {})["ANTHROPIC_BASE_URL"] ?? null,
    // The forged header: that the run failed, and what the failure does not say.
    forgedHeader: forged,
    forgedNamesTheValue: forged === null ? null : forged.indexOf("someone-elses-key") >= 0,
  };

  // `codex`, whose surface is its client's options rather than an environment.
  const codexStub = harness.scriptedDriver("codex", script({ summary: "x", touched: [] }));
  await runtime.runCoder(
    binding({
      harness: "codex",
      modelId: "gpt-5-codex",
      modelSettings: {},
      // No harness config, so the thread's options below are exactly what the
      // adapter put there — which is what makes that key list a statement.
      settings: {},
      connection: {
        provider: "provider.openai",
        baseUrl: [{ env: "CODER_GATEWAY_URL", site: "provider.openai.base_url" }],
        credential: [{ env: "CODER_GATEWAY_KEY", site: "provider.openai.api_key" }],
      },
    }),
    { goal: "fix it" },
    context(),
    { codex: codexStub.driver },
  );
  const client = harness.codexClient(codexStub.runs[0]);

  const codexKeylessStub = harness.scriptedDriver("codex", script({ summary: "x", touched: [] }));
  await runtime.runCoder(
    binding({
      harness: "codex",
      modelId: "gpt-5-codex",
      modelSettings: {},
      settings: {},
      connection: keylessConnection,
    }),
    { goal: "fix it" },
    context(),
    { codex: codexKeylessStub.driver },
  );
  const keylessClient = harness.codexClient(codexKeylessStub.runs[0]);

  results["codexConnection"] = {
    baseUrl: client.baseUrl ?? null,
    apiKey: client.apiKey ?? null,
    // The scrubbed environment is still the client's, beside the connection.
    env: Object.keys(client.env ?? {}).sort(),
    // Nothing of the connection leaked into the **thread's** options, which are
    // the bounds the node states rather than the connection it inherits.
    threadKeys: Object.keys(harness.codexOptions(codexStub.runs[0])).sort(),
    // …and the keyless gateway: an endpoint and no key on the client at all.
    keylessHasApiKey: Object.hasOwn(keylessClient, "apiKey"),
    keylessBaseUrl: keylessClient.baseUrl ?? null,
  };
}

process.stdout.write(`${JSON.stringify(results, null, 2)}\n`);
