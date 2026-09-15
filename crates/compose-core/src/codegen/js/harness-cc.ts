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
 * How `access:` reaches the Agent SDK (grammar 8.9, Decision D138).
 *
 * The SDK's containment primitive is a permission *mode* plus a working
 * directory, so the preset selects the mode and `cwd` carries the root:
 *
 *  * `read_only` → `plan`, which the SDK documents as planning mode with no
 *    execution of tools — the strongest read-only statement its own surface
 *    makes;
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
 * `canUseTool`, per call, inside the loop. `allowedTools` is set beside it
 * because it is what the SDK offers the model in the first place, and the
 * callback is what makes the list a **bound** rather than an offer: a call
 * outside it is denied with a message the model reads, and the denial is taped
 * as a `"refused"` tool event so the trace says the bound bit.
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
      const controller = controllerFor(run.signal);
      // What the permission callback refused, drained into the stream between
      // messages: `canUseTool` answers the SDK rather than this generator, so a
      // queue is how a denial becomes a tool event in the order it happened.
      const refusals: runtime.HarnessEvent[] = [];
      // The name each top-level `tool_use` went out under, so the `tool_result`
      // that answers it can be taped under the same name.
      const calls = new Map<string, string>();

      const options: Options = {
        cwd: run.workspace,
        systemPrompt: run.instructions,
        model: run.model,
        permissionMode: CC_PERMISSION[run.access],
        abortController: controller,
        env: { ...run.env },
        outputFormat: { type: "json_schema", schema: { ...run.schema } },
        ...passthrough(run, CC_SETTINGS),
      };
      if (run.access === "full_access") options.allowDangerouslySkipPermissions = true;
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
        options.allowedTools = [...allowed];
        options.canUseTool = (name: string) => {
          if (allowed.includes(name)) return Promise.resolve({ behavior: "allow" as const });
          const message = `\`${run.node}\` allows ${allowed.map((tool) => `\`${tool}\``).join(", ")}, and \`${name}\` is not one of them`;
          refusals.push({
            source: { type: "agent-compose.permission_denied", tool: name, message },
            tap: { kind: "tool", name, outcome: "refused", error: message },
          });
          return Promise.resolve({ behavior: "deny" as const, message });
        };
      }

      for await (const message of query({ prompt: run.input, options })) {
        while (refusals.length > 0) yield refusals.shift() as runtime.HarnessEvent;
        yield* ccEvents(message, calls);
      }
      while (refusals.length > 0) yield refusals.shift() as runtime.HarnessEvent;
    })();
  },
};

/** One SDK message, as the events the tap reads (see [`CC_DRIVER`]). */
function* ccEvents(
  message: SDKMessage,
  calls: Map<string, string>,
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
