/**
 * The keys of a `codex` node's `settings:` this compiler release maps by name
 * (Decision D140's first tier).
 *
 * Everything else travels to the thread's options unchanged, under the warning
 * `validate` printed for it — PRD resolved q30's terms, one construct along.
 */
const CODEX_SETTINGS: readonly string[] = [
  "network_access",
  "web_search",
  "skip_git_repo_check",
];

/**
 * The keys of a thread's options the **adapter** owns, and which a `settings:`
 * key therefore never reaches (see [`passthrough`]).
 *
 * Two kinds of name are on the list, exactly as on `cc`'s:
 *
 *  * an option that **spells** a bound some other part of the node states:
 *    `workspace:` is `workingDirectory`, `access:` is `sandboxMode` and the
 *    approval policy beside it, and `model:` is the model and the one reasoning
 *    setting Decision D141 maps into it. A composition that spelled one of them
 *    here would be reaching around the construct that states it, through the
 *    surface this grammar deliberately leaves open — so these are dropped
 *    rather than passed. The environment is not on the list because it is not a
 *    thread option at all: it is handed to the `Codex` constructor below;
 *  * an option that **contains** one without spelling it, which is the wider
 *    half of the dropped set grammar 8.9 states: `additionalDirectories` is
 *    additional sandbox roots beside `workspace:`, so a run under it is written
 *    where the node's own workspace never reached. It is the same option `cc`
 *    drops by the same name, and the two harnesses answer that key the same way
 *    or `access:` means one thing under one harness and another under the
 *    other.
 *
 * `approvalPolicy` is the one that is neither: it is the **per-call approval
 * tier** PRD resolved q57 ruling c says this release does not adopt, and a run
 * that escalated out of its sandbox to an approver nothing answers for would be
 * `access:` saying one thing and the run doing another. The thread's other
 * options are the vendor's own vocabulary and travel unchanged, which is
 * Decision D140's whole point.
 *
 * The list is audited against the pinned SDK's own `ThreadOptions`, which is
 * what `a_reserved_list_is_audited_against_the_pinned_option_surface`
 * (`codegen/harness.rs`) holds it to.
 */
const CODEX_RESERVED: readonly string[] = [
  // Options that spell a bound another key states.
  "approvalPolicy",
  "model",
  "modelReasoningEffort",
  "sandboxMode",
  "workingDirectory",
  // …and the option that contains one without spelling it.
  "additionalDirectories",
];

/**
 * How `access:` reaches the Codex SDK (grammar 8.9, Decision D138).
 *
 * One-to-one with its own sandbox presets, which is why this grammar's three
 * names are the three they are: `codex` is the harness whose containment
 * primitive is named, and a vocabulary that did not line up with it would be a
 * translation nobody could check.
 */
const CODEX_SANDBOX: Readonly<Record<runtime.WorkspaceAccess, SandboxMode>> = {
  read_only: "read-only",
  workspace_write: "workspace-write",
  full_access: "danger-full-access",
};

/**
 * The `codex` driver: a thin mapping over the Codex SDK.
 *
 * # Where the top level is
 *
 * Everywhere. `runStreamed` yields one thread's events, and a Codex thread has
 * no nesting for PRD resolved q57 ruling a's depth rule to cut at: what its
 * `mcp_tool_call` items reach is another process's, not a subagent of this run.
 * So every turn and every tool item this stream carries is a top-level one, and
 * the ruling's boundary is satisfied by there being nothing below it.
 *
 * # What bounds a run, and what does not
 *
 * The **sandbox**, and nothing else. The Codex SDK's per-call approval tier is
 * their app server's, which this release does not adopt, so `enforcesTools` is
 * `false` and `docs/grammar.md` 8.9 states the asymmetry where an author writes
 * the list. The `codex` half of PRD resolved q57 ruling c is this constant.
 *
 * `allow_tools:` is therefore what the harness is **offered**, and offering it
 * is something a driver has to do rather than something that happens: a thread
 * has no tool-set option to put it in, so the list reaches the run the one way
 * anything reaches it — as a line of the instructions the turn carries
 * ([`codexTurn`]). A list that reached nothing at all would make grammar 8.9's
 * `offered` a word for a key with no effect.
 *
 * # What is asked for, and what is parsed
 *
 * `outputSchema`, carrying the schema the adapter already projected through this
 * harness's lowering table; the answer arrives as the last `agent_message`
 * item's text, which the SDK documents as JSON when a schema was asked for. The
 * adapter parses it against the **full** declared schema (PRD resolved q55).
 */
const CODEX_DRIVER: runtime.HarnessDriver = {
  sdk: "@openai/codex-sdk",
  version: CODEX_SDK_VERSION,
  enforcesTools: false,
  run(run: runtime.HarnessRun): AsyncIterable<runtime.HarnessEvent> {
    return (async function* driven(): AsyncGenerator<runtime.HarnessEvent> {
      // The environment is handed to the SDK rather than inherited by it: the
      // Codex SDK documents that a provided `env` replaces `process.env` for the
      // CLI it spawns, which is exactly the scrubbed child PRD resolved q54
      // ruling b asks for (Decision D139).
      const codex = new Codex({ env: { ...run.env } });
      const thread = codex.startThread(codexOptions(run));
      const streamed = await thread.runStreamed(codexTurn(run), {
        outputSchema: { ...run.schema },
        signal: run.signal,
      });

      // The totals, accumulated: `codex` reports usage per turn and no money at
      // all, so the rollup is a sum of what the turns said rather than a number
      // the SDK hands over (PRD resolved q57 ruling a).
      let inputTokens = 0;
      let outputTokens = 0;
      let cached = 0;
      let cacheWrites = 0;
      let answer: string | undefined;

      for await (const event of streamed.events) {
        switch (event.type) {
          case "turn.completed": {
            const usage = event.usage;
            inputTokens += usage.input_tokens;
            outputTokens += usage.output_tokens;
            cached += usage.cached_input_tokens;
            cacheWrites += usage.cache_write_input_tokens;
            yield {
              source: event,
              tap: {
                kind: "turn",
                usage: {
                  inputTokens: usage.input_tokens,
                  cachedInputTokens: usage.cached_input_tokens,
                  outputTokens: usage.output_tokens,
                  reasoningTokens: usage.reasoning_output_tokens,
                },
              },
            };
            break;
          }
          case "turn.failed":
            yield { source: event, tap: { kind: "error", message: event.error.message } };
            break;
          case "error":
            yield { source: event, tap: { kind: "error", message: event.message } };
            break;
          case "item.completed": {
            const tap = codexToolTap(event.item);
            if (event.item.type === "agent_message") answer = event.item.text;
            yield tap === undefined ? { source: event } : { source: event, tap };
            break;
          }
          default:
            yield { source: event };
            break;
        }
      }

      yield {
        source: { type: "agent-compose.settled" },
        tap: {
          kind: "settled",
          cost: { inputTokens, outputTokens },
          // The two counters `cc` has no shape for.
          extra: { cachedInputTokens: cached, cacheWriteInputTokens: cacheWrites },
        },
      };
      if (answer !== undefined) {
        yield {
          source: { type: "agent-compose.agent_message" },
          tap: { kind: "output", value: codexAnswer(answer) },
        };
      }
    })();
  },
};

/**
 * The `ThreadOptions` one `codex` run is made of — the config map of grammar
 * 8.9, as one value (see [`CODEX_DRIVER`]).
 *
 * **Exported for the reason [`scriptedDriver`] is**, and the same reason
 * [`ccOptions`] is under the other harness: this object is where `workspace:`
 * and `access:` become this harness's own containment primitive, and none of it
 * is visible from a run's answer or from the events the driver yields. The
 * bound worth a test most of all is the [`CODEX_RESERVED`] drop, which is a
 * *subtraction* — what a `settings:` key did **not** put here — and a
 * subtraction can only be read off the object itself.
 */
export function codexOptions(run: runtime.HarnessRun): ThreadOptions {
  const options: ThreadOptions = {
    model: run.model,
    workingDirectory: run.workspace,
    sandboxMode: CODEX_SANDBOX[run.access],
    ...passthrough(run, CODEX_SETTINGS, CODEX_RESERVED),
  };
  const effort = settingText(run.modelSettings["reasoning_effort"]);
  if (effort !== undefined) {
    options.modelReasoningEffort = effort as ThreadOptions["modelReasoningEffort"];
  }
  const network = settingFlag(run.settings["network_access"]);
  if (network !== undefined) options.networkAccessEnabled = network;
  const search = settingText(run.settings["web_search"]);
  if (search !== undefined) options.webSearchMode = search as ThreadOptions["webSearchMode"];
  const skipGit = settingFlag(run.settings["skip_git_repo_check"]);
  if (skipGit !== undefined) options.skipGitRepoCheck = skipGit;
  return options;
}

/**
 * The turn one run sends: the node's instructions, the list it offers the
 * harness where it wrote one, and the bound input (see [`CODEX_DRIVER`]).
 *
 * The instructions lead, the way they do under the other harness, and the
 * vendor's own base instructions are untouched beneath them — a thread keeps
 * the agent loop it was post-trained with, which is why this is a kind at all
 * (PRD resolved q57).
 *
 * The offered list is stated as what it is: this harness bounds at the sandbox,
 * so the sentence is an instruction the model reads and not a bound. `cc` puts
 * the same list somewhere that holds; `docs/grammar.md` 8.9 and the graph
 * document's `tools_enforced` are where the asymmetry is written down.
 */
function codexTurn(run: runtime.HarnessRun): string {
  const offered =
    run.allowTools === undefined
      ? ""
      : `\n\nThe tools this run is offered: ${run.allowTools.join(", ")}. Use no others.`;
  return `${run.instructions}${offered}\n\n${run.input}`;
}

/**
 * One completed thread item, as a tool event — or `undefined` where the item is
 * not a tool at all.
 *
 * Four of the SDK's eight item types are tool events; the other four are the
 * agent's own message, its reasoning, its to-do list and a non-fatal error,
 * none of which is a call to anything. Their events still reach the payload:
 * this decides what the **trace** carries, not what the journal holds.
 */
function codexToolTap(item: ThreadItem): runtime.HarnessTap | undefined {
  switch (item.type) {
    case "command_execution":
      return {
        kind: "tool",
        name: "command_execution",
        outcome: item.status === "failed" ? "failed" : "completed",
        ...(item.status === "failed"
          ? { error: `the command exited ${item.exit_code ?? "without a status"}` }
          : {}),
      };
    case "file_change":
      return {
        kind: "tool",
        name: "file_change",
        outcome: item.status === "failed" ? "failed" : "completed",
        ...(item.status === "failed" ? { error: "the patch did not apply" } : {}),
      };
    case "mcp_tool_call":
      return {
        kind: "tool",
        name: `${item.server}/${item.tool}`,
        outcome: item.status === "failed" ? "failed" : "completed",
        ...(item.error === undefined ? {} : { error: item.error.message }),
      };
    case "web_search":
      return { kind: "tool", name: "web_search", outcome: "completed" };
    default:
      return undefined;
  }
}

/**
 * The agent's last message, as the value the gate is handed.
 *
 * The SDK documents an `agent_message`'s `text` as JSON when `outputSchema` was
 * asked for, so the text is decoded here. A text that is **not** JSON is handed
 * on as the string it is rather than being refused here: what an answer has to
 * be is the node's `output:` schema, and the gate is the one place that decides
 * it — a driver that refused first would report a decoding failure where the
 * composition's own contract has a message to give (PRD resolved q55 ruling c).
 */
function codexAnswer(text: string): unknown {
  try {
    return JSON.parse(text) as unknown;
  } catch {
    return text;
  }
}
