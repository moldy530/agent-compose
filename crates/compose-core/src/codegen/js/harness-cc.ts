/**
 * The keys of a `cc` node's `settings:` this compiler release maps by name
 * (Decision D140's first tier).
 *
 * Everything else travels to `query`'s options unchanged, under the warning
 * `validate` printed for it. Both halves are deliberate: a gating table is the
 * treadmill PRD resolved q30 refused, and a key with a *meaning* — a turn bound,
 * a budget — is worth checking before a run rather than after a 400.
 */
const CC_SETTINGS: readonly string[] = [
  "max_turns",
  "max_budget_usd",
  "forward_subagent_text",
  "disallowed_tools",
];

/**
 * The keys of `query`'s options the **adapter** owns, and which a `settings:`
 * key therefore never reaches (see [`passthrough`]).
 *
 * A composition that spelled one of these would be reaching around the
 * construct that states it, through the surface this grammar deliberately
 * leaves open — so they are dropped rather than passed. Two kinds of name are
 * on the list, and the second kind is why it is a list rather than a reading of
 * what the driver below assigns:
 *
 *  * an option that **spells** a bound another key states. `workspace:` is
 *    `cwd`, `access:` is `permissionMode`, the plan-mode body beside it and the
 *    flag `bypassPermissions` requires, `env:` is `env`, `output:` is
 *    `outputFormat`, `prompt:` is `systemPrompt`, `allow_tools:` is the
 *    available tool set, the allowlist and the callback over them, `timeout:`
 *    is the abort controller, and `model:` is the model and the one thinking
 *    budget Decision D141 maps into it (`thinking` is here for that last
 *    reason: the SDK documents it as taking precedence over the
 *    `maxThinkingTokens` D141 writes);
 *  * an option that **contains** one without spelling it, which a driver never
 *    assigns and a list keyed off the driver could therefore never hold.
 *    `extraArgs` is an arbitrary CLI flag — `dangerously-skip-permissions` and
 *    `add-dir` among them — so it is every bound at once. `settings`,
 *    `managedSettings` and `settingSources` carry permission rules;
 *    `additionalDirectories` carries roots beside `workspace:`; `sandbox`
 *    carries containment; `mcpServers`, `agents`, `agent` and `toolAliases`
 *    each put a tool or a whole loop outside `allow_tools:` within reach —
 *    `agent` also carrying its own model and prompt — and `plugins` carries
 *    hooks, agents and skills together; `hooks`, `permissionPrompts` and
 *    `permissionPromptToolName` each move or silence the decision `canUseTool`
 *    makes; and `fallbackModel` is the failover ladder D141 stops at the
 *    boundary. The **process-spawn family** — `pathToClaudeCodeExecutable`,
 *    `executable` and `executableArgs` — contains *every* bound at once and for
 *    the worst reason: those three choose which program runs and what the
 *    runtime loads before it, so a key among them replaces or re-arms the very
 *    harness that is supposed to be enforcing `tools`, `canUseTool` and
 *    `permissionMode`. An adapter never assigns them — it wants the SDK's own
 *    executable — which is exactly why a list read off the driver could not
 *    hold them. The `resume` family — `resume`, `continue`, `forkSession`,
 *    `sessionId`, `resumeSessionAt`, `resumeDropsTurn` — is dropped for PRD
 *    resolved q57 ruling b's reason rather than for containment: harness-native
 *    resume is a **named exclusion**, because a machine-local session store is
 *    not the journal.
 *
 * The list is audited against the option surface of the pinned SDK, which is
 * what `a_reserved_list_is_audited_against_the_pinned_option_surface`
 * (`codegen/harness.rs`) holds it to: a version bump is where a new
 * reach-around arrives, so the pin is what re-opens the audit.
 */
const CC_RESERVED: readonly string[] = [
  // Options that spell a bound another key states.
  "abortController",
  "allowDangerouslySkipPermissions",
  "allowedTools",
  "canUseTool",
  "cwd",
  "env",
  "maxThinkingTokens",
  "model",
  "outputFormat",
  "permissionMode",
  "planModeInstructions",
  "systemPrompt",
  "thinking",
  "tools",
  // …and options that contain one without spelling it.
  "additionalDirectories",
  "agent",
  "agents",
  "continue",
  "executable",
  "executableArgs",
  "extraArgs",
  "fallbackModel",
  "forkSession",
  "hooks",
  "managedSettings",
  "mcpServers",
  "pathToClaudeCodeExecutable",
  "permissionPromptToolName",
  "permissionPrompts",
  "plugins",
  "resume",
  "resumeDropsTurn",
  "resumeSessionAt",
  "sandbox",
  "sessionId",
  "settingSources",
  "settings",
  "toolAliases",
];

/**
 * How `access:` reaches the Agent SDK (grammar 8.9, Decision D138).
 *
 * The SDK's containment primitive is a permission *mode* plus a working
 * directory, so the preset selects the mode and `cwd` carries the root. Which
 * primitive a preset lands on is **stated per harness and never implied
 * equivalent** (PRD resolved q57 ruling c): what `access:` means is the same
 * sentence under both harnesses, and what enforces it is each harness's own:
 *
 *  * `read_only` → `plan`, the mode whose system reminder the CLI wraps in its
 *    **read-only enforcement** preamble: the tree is readable and nothing under
 *    it is written. It is the one containment statement the SDK's own surface
 *    makes without a tool taxonomy this adapter would have had to invent, and
 *    the node's own instructions replace the mode's default
 *    code-implementation workflow body (`planModeInstructions`) so the run is
 *    asked for what the node asked for rather than for a plan to implement
 *    something;
 *  * `workspace_write` → `acceptEdits`, which auto-accepts edits under the
 *    working directory and still asks about everything else;
 *  * `full_access` → `bypassPermissions`, with the flag the SDK requires beside
 *    it, which is the preset that asks for no containment at all.
 */
const CC_PERMISSION: Readonly<Record<runtime.WorkspaceAccess, PermissionMode>> = {
  read_only: "plan",
  workspace_write: "acceptEdits",
  full_access: "bypassPermissions",
};

/**
 * The `cc` driver: a thin mapping over the Claude Agent SDK.
 *
 * # Where the top level is
 *
 * `parent_tool_use_id === null`. Every message the SDK yields carries it, and a
 * message produced *inside* a subagent carries that subagent's tool-use id — so
 * the one predicate is the whole of PRD resolved q57 ruling a's depth rule: a
 * subagent's turns and tool calls are yielded with no `tap` and reach the
 * journal payload only.
 *
 * # What enforces the allowlist
 *
 * **Two options, because one of them has a hole.** `tools` is the SDK's own
 * "base set of available built-in tools", so a list written there is a tool set
 * the model is never offered — a bound that holds whatever the permission mode
 * is. `canUseTool` denies, per call, anything outside the list that reached the
 * loop anyway, and tapes the denial as a `"refused"` tool event so the trace
 * says the bound bit; `allowedTools` auto-allows the ones inside it so a bounded
 * run is not also a prompting one.
 *
 * The hole is `access: full_access`, and it is why `tools` carries the bound
 * rather than the callback: that preset is `permissionMode: "bypassPermissions"`,
 * which the SDK documents as bypassing **all** permission checks — so
 * `canUseTool` does not run, and a node whose `enforcesTools` says its list is
 * enforced would be asserting a bound nothing held (grammar 8.9, PRD resolved
 * q57 ruling c). Narrowing the available set is the answer the SDK's own
 * documentation gives for `allowedTools`: "to restrict which tools are
 * available, use the `tools` option instead".
 *
 * # Whose system prompt a run has
 *
 * The harness's, with the node's `prompt:` **appended** to it
 * (`{ type: "preset", preset: "claude_code", append }`). A bare string there is
 * the SDK's custom-prompt form and *replaces* the preset — and the preset is
 * the vendor's own agent instructions, which is half of what PRD resolved q57
 * makes this a kind for: the loop, the tool shapes and the prompt the model was
 * post-trained against are the thing that cannot be reassembled from parts. So
 * the node's prompt is what it reads like in the grammar — instructions *for*
 * the harness — rather than a replacement of the harness.
 *
 * # What is asked for, and what is parsed
 *
 * `outputFormat: { type: "json_schema" }` carrying the schema the adapter
 * already projected through this harness's lowering table, and the answer
 * arrives as the result message's `structured_output`. The adapter parses it
 * against the **full** declared schema, which is where PRD resolved q55's
 * amendment to q16 lives.
 */
const CC_DRIVER: runtime.HarnessDriver = {
  sdk: "@anthropic-ai/claude-agent-sdk",
  version: CC_SDK_VERSION,
  enforcesTools: true,
  run(run: runtime.HarnessRun): AsyncIterable<runtime.HarnessEvent> {
    return (async function* driven(): AsyncGenerator<runtime.HarnessEvent> {
      // What the permission callback refused, drained into the stream between
      // messages: `canUseTool` answers the SDK rather than this generator, so a
      // queue is how a denial becomes a tool event in the order it happened.
      const refusals: runtime.HarnessEvent[] = [];
      // The name each top-level `tool_use` went out under, so the `tool_result`
      // that answers it can be taped under the same name.
      const calls = new Map<string, string>();
      // The `tool_use` ids the callback below denied. A denial is taped once,
      // from the callback; the `tool_result` the SDK then hands the model is
      // that same denial travelling back, and taping it again would claim two
      // tool events where the run made one (`docs/trace.md` 7.6.3, whose
      // `completed` and `failed` describe a call that executed).
      const refused = new Set<string>();

      const config = ccOptions(run, refusals, refused);

      for await (const message of query({ prompt: run.input, options: config })) {
        while (refusals.length > 0) yield refusals.shift() as runtime.HarnessEvent;
        yield* ccEvents(message, calls, refused);
      }
      while (refusals.length > 0) yield refusals.shift() as runtime.HarnessEvent;
    })();
  },
};

/**
 * The `Options` one `cc` run is made of — the config map of grammar 8.9, as one
 * value (see [`CC_DRIVER`]).
 *
 * **Exported for the reason [`scriptedDriver`] is**: what this object holds is
 * the whole of the node's containment, and none of it is visible from a run's
 * answer or from the events a driver yields. A test that could only drive the
 * seam could say nothing about the bound at all — and the bound that matters
 * most, the [`CC_RESERVED`] drop, is a *subtraction*: it is what a settings key
 * did **not** put here. So the options are built by a function a test can call
 * and then read, against a `run` whose `settings` spell every bound this node
 * states, rather than assembled inline where only the vendor's SDK ever sees
 * them.
 *
 * `refusals` and `refused` are the driver's own two queues, handed in because
 * the permission callback below writes to both: a denial is an event the
 * generator drains between messages, and an id the `tool_result` that answers
 * it is told apart by.
 */
export function ccOptions(
  run: runtime.HarnessRun,
  refusals: runtime.HarnessEvent[],
  refused: Set<string>,
): Options {
  const options: Options = {
    cwd: run.workspace,
    systemPrompt: { type: "preset", preset: "claude_code", append: run.instructions },
    model: run.model,
    permissionMode: CC_PERMISSION[run.access],
    abortController: controllerFor(run.signal),
    env: { ...run.env },
    outputFormat: { type: "json_schema", schema: { ...run.schema } },
    ...passthrough(run, CC_SETTINGS, CC_RESERVED),
  };
  if (run.access === "full_access") options.allowDangerouslySkipPermissions = true;
  // Plan mode's body, replaced by what this node asked for: the mode's
  // default body is a code-implementation workflow, and a `read_only` node
  // is a run that reads and reports (see [`CC_PERMISSION`]).
  if (run.access === "read_only") options.planModeInstructions = run.instructions;
  // The one `model.*` setting this harness has a place for: the `anthropic`
  // plugin's `thinking: { budget_tokens: … }` is the SDK's
  // `maxThinkingTokens` (Decision D141).
  const thinking = run.modelSettings["thinking"];
  const budgetTokens =
    typeof thinking === "object" && thinking !== null
      ? settingNumber((thinking as Record<string, unknown>)["budget_tokens"])
      : undefined;
  if (budgetTokens !== undefined) options.maxThinkingTokens = budgetTokens;
  const maxTurns = settingNumber(run.settings["max_turns"]);
  if (maxTurns !== undefined) options.maxTurns = maxTurns;
  const budget = settingNumber(run.settings["max_budget_usd"]);
  if (budget !== undefined) options.maxBudgetUsd = budget;
  const forward = settingFlag(run.settings["forward_subagent_text"]);
  if (forward !== undefined) options.forwardSubagentText = forward;
  const disallowed = settingList(run.settings["disallowed_tools"]);
  if (disallowed !== undefined) options.disallowedTools = disallowed;
  const allowed = run.allowTools;
  if (allowed !== undefined) {
    // The bound itself: a tool outside the list is not in the set the loop
    // can reach, which is true under every one of the three `access:`
    // presets — `bypassPermissions` included.
    options.tools = [...allowed];
    options.allowedTools = [...allowed];
    options.canUseTool = (name, _input, ask) => {
      if (allowed.includes(name)) return Promise.resolve({ behavior: "allow" as const });
      const message = `\`${run.node}\` allows ${allowed.map((tool) => `\`${tool}\``).join(", ")}, and \`${name}\` is not one of them`;
      // The call this denial answers, remembered by its id: the SDK hands
      // the model the denial as the `tool_result` for that `tool_use`, and
      // one call is one tool event.
      refused.add(ask.toolUseID);
      refusals.push({
        source: { type: "agent-compose.permission_denied", tool: name, message },
        // A denial inside a **subagent** is that subagent's, and the
        // envelope carries the top level only — the same depth rule
        // [`ccEvents`] reads off `parent_tool_use_id`, read here off the
        // sub-agent id the SDK passes the callback (PRD resolved q57
        // ruling a).
        ...(ask.agentID === undefined
          ? { tap: { kind: "tool" as const, name, outcome: "refused" as const, error: message } }
          : {}),
      });
      return Promise.resolve({ behavior: "deny" as const, message });
    };
  }
  return options;
}

/** One SDK message, as the events the tap reads (see [`CC_DRIVER`]). */
function* ccEvents(
  message: SDKMessage,
  calls: Map<string, string>,
  refused: Set<string>,
): Generator<runtime.HarnessEvent> {
  // A message from inside a subagent is payload and nothing else — the depth
  // rule PRD resolved q57 ruling a fixes, read off the one field that carries
  // it.
  const nested =
    "parent_tool_use_id" in message && (message as { parent_tool_use_id: unknown }).parent_tool_use_id !== null;
  if (nested) {
    yield { source: message };
    return;
  }
  if (message.type === "assistant") {
    const blocks = Array.isArray(message.message.content) ? message.message.content : [];
    for (const block of blocks) {
      if (block.type === "tool_use") calls.set(block.id, block.name);
    }
    yield { source: message, tap: { kind: "turn" } };
    return;
  }
  if (message.type === "user") {
    const blocks = Array.isArray(message.message.content) ? message.message.content : [];
    yield { source: message };
    for (const block of blocks) {
      if (block.type !== "tool_result") continue;
      const name = calls.get(block.tool_use_id);
      if (name === undefined) continue;
      // A call the permission callback denied was taped `refused` there, and
      // this result is that denial on its way to the model rather than a second
      // event: the call never executed, so neither `completed` nor `failed`
      // describes it (`docs/trace.md` 7.6.3). The message still reaches the
      // journal's payload above, untapped, like every other result.
      if (refused.delete(block.tool_use_id)) {
        calls.delete(block.tool_use_id);
        continue;
      }
      const failed = block.is_error === true;
      yield {
        source: { type: "agent-compose.tool_result", tool: name, is_error: failed },
        tap: {
          kind: "tool",
          name,
          outcome: failed ? "failed" : "completed",
          ...(failed ? { error: `the tool result came back as an error` } : {}),
        },
      };
    }
    return;
  }
  if (message.type === "result") {
    const usage = message.usage as { input_tokens?: number; output_tokens?: number };
    yield {
      source: message,
      tap: {
        kind: "settled",
        cost: {
          ...(typeof usage.input_tokens === "number" ? { inputTokens: usage.input_tokens } : {}),
          ...(typeof usage.output_tokens === "number" ? { outputTokens: usage.output_tokens } : {}),
          ...(typeof message.total_cost_usd === "number" ? { usd: message.total_cost_usd } : {}),
        },
        // The two facts `codex` has no shape for: which of the SDK's result
        // subtypes this was, and the stop reason underneath it.
        extra: { subtype: message.subtype, stopReason: message.stop_reason },
      },
    };
    if (message.subtype !== "success") {
      yield {
        source: { type: "agent-compose.result_error", subtype: message.subtype },
        tap: { kind: "error", message: `the run ended \`${message.subtype}\`` },
      };
      return;
    }
    if (message.structured_output !== undefined) {
      yield {
        source: { type: "agent-compose.structured_output" },
        tap: { kind: "output", value: message.structured_output },
      };
    }
    return;
  }
  yield { source: message };
}
