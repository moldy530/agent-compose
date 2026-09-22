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
 * on the list, and the second kind — with one name of the first — is why it is
 * a list rather than a reading of what the driver below assigns:
 *
 *  * an option that **spells** a bound another key states. `workspace:` is
 *    `cwd`; `permission_mode:` is `permissionMode`, the plan-mode body beside it
 *    and the flag `bypassPermissions` requires, with `access:` the key that
 *    derives that mode where the node states none (Decision D146); `env:` is
 *    `env` — which is also where
 *    the model's **connection** lands, because this SDK's endpoint, credential
 *    and custom headers are variables of the process it spawns (Decision D143),
 *    so one reserved name holds both bounds — `output:` is
 *    `outputFormat`, `prompt:` is `systemPrompt`, `allow_tools:` is the
 *    available tool set and the callback over it, `timeout:` is the abort
 *    controller, and `model:` is the model and the one thinking budget
 *    Decision D141 maps into it (`thinking` is here for that last reason: the
 *    SDK documents it as taking precedence over the `maxThinkingTokens` D141
 *    writes). `allowedTools` spells `allow_tools:` too, and it is the one name
 *    of this kind the driver below **never** writes: a bare entry there
 *    approves a whole tool before `canUseTool` is consulted, which the pinned
 *    SDK flags as a shadowed callback (see [`CC_DRIVER`]) — so a key spelling
 *    it would put back the very pairing the driver leaves out;
 *  * an option that **contains** one without spelling it, which a driver never
 *    assigns and a list keyed off the driver could therefore never hold.
 *    `extraArgs` is an arbitrary CLI flag — `dangerously-skip-permissions` and
 *    `add-dir` among them — so it is every bound at once. `settings`,
 *    `managedSettings` and `settingSources` carry permission rules;
 *    `additionalDirectories` carries roots beside `workspace:`; `sandbox`
 *    carries containment; `mcpServers`, `agents`, `agent`, `skills` and
 *    `toolAliases` each put a tool or a whole loop outside `allow_tools:`
 *    within reach — `agent` also carrying its own model and prompt, and
 *    `skills` being the SDK's own single switch for turning skills on, which
 *    its documentation says needs no `'Skill'` entry in `allowedTools` beside
 *    it — and `plugins` carries hooks, agents and skills together; `hooks`,
 *    `permissionPrompts` and
 *    `permissionPromptToolName` each move or silence the decision `canUseTool`
 *    makes; and `fallbackModel` is the failover ladder D141 stops at the
 *    boundary. The **process-spawn family** — `pathToClaudeCodeExecutable`,
 *    `executable`, `executableArgs` and `spawnClaudeCodeProcess` — contains
 *    *every* bound at once and for the worst reason: those four choose which
 *    program runs and what the runtime loads before it, so a key among them
 *    replaces or re-arms the very
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
  "skills",
  "spawnClaudeCodeProcess",
  "toolAliases",
];

/**
 * How `access:` reaches the Agent SDK (grammar 8.9, Decisions D138, D146).
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
 *
 * This is the **derived** mode: what a node that states no `permission_mode:`
 * runs under, which is every node written before that key existed (Decision
 * D146). A node that states one supersedes the map here and nowhere else — the
 * compiler has already refused any mode the level does not admit, so the value
 * arriving at [`ccPermissionMode`] is never wider than this one.
 */
const CC_PERMISSION: Readonly<Record<runtime.WorkspaceAccess, PermissionMode>> = {
  read_only: "plan",
  workspace_write: "acceptEdits",
  full_access: "bypassPermissions",
};

/**
 * The mode one run works under: the one the node stated, or the one its
 * `access:` level derives (grammar 8.9, Decision D146, PRD resolved q60 ruling
 * a).
 *
 * **One function, because three options answer to this value** and a second
 * reading of it would be a second answer. `permissionMode` is the mode itself;
 * `allowDangerouslySkipPermissions` is the flag the SDK *requires* beside
 * `bypassPermissions` and beside nothing else; and `planModeInstructions` is
 * plan mode's body, which exists only when the mode is `plan`. Keying those two
 * off `run.access` instead — which is what this driver did while `access:` was
 * the only axis — would arm the skip flag on a `full_access` node that asked for
 * `plan`, and leave a `full_access` node that asked for `plan` running the
 * vendor's default code-implementation body instead of the node's own prompt.
 * Reading the resolved mode is the same answer in every case the two used to
 * agree on, which is every node that states no mode at all.
 */
function ccPermissionMode(run: runtime.HarnessRun): PermissionMode {
  return run.permissionMode ?? CC_PERMISSION[run.access];
}

/**
 * Whether one resolved header value would forge a second header field.
 *
 * The compiler asks this of the text a composition **wrote** (`validate`
 * refuses it there, `invalid-value`); this asks it of what a `${ENV}` resolved
 * to, which is text no build ever saw and precisely the shape a gateway
 * deployment produces — an operator sets the variable, not the author.
 *
 * Every control character **but a tab**, not only the two that split a line:
 * `\r` on its own is a byte no header value carries either, `\r\n` splits on the
 * newline and leaves the carriage return glued to the line before it, and `\0`
 * truncates the environment variable this encoding writes. A tab passes because
 * RFC 9110 5.5's `field-content` admits `SP` and `HTAB` between visible
 * characters, so a tab is content a header value really carries and no split
 * this variable performs — on newlines, then on each line's first colon —
 * notices one.
 *
 * The set is the parser's set, character for character (grammar Decision D144,
 * `parse::binding::header_value_shape`), including the C1 range Rust's
 * `char::is_control` covers: the two halves of one rule ask the same question of
 * written text and of resolved text, and a value this refuses at run time is one
 * `validate` would have refused had the composition written it out.
 */
function forgesAHeaderField(value: string): boolean {
  for (let index = 0; index < value.length; index += 1) {
    const code = value.charCodeAt(index);
    if (code === 0x09) continue;
    if (code < 0x20 || (code >= 0x7f && code <= 0x9f)) return true;
  }
  return false;
}

/**
 * How a provider connection reaches this harness (grammar 8.9, Decision D143,
 * PRD resolved q58 ruling a).
 *
 * The SDK's own `Options` carry no endpoint and no credential — what they carry
 * is `env`, documented as **replacing** the subprocess environment outright —
 * and the Claude Code runtime that subprocess runs reads its connection out of
 * that environment. So all three facts are variables, merged into the `env` the
 * adapter already built from the node's own `env:`:
 *
 *  * `ANTHROPIC_BASE_URL` — the endpoint the bundled client is constructed with;
 *  * `ANTHROPIC_API_KEY` — the credential, read the same way. The **key** rather
 *    than the auth-token variable beside it, because what grammar 12.1 spells
 *    `api_key:` is a vendor API key; a gateway wanting a bearer token of its own
 *    writes it into `headers:`, which is resolved q25's own answer and arrives
 *    through the variable below;
 *  * `ANTHROPIC_CUSTOM_HEADERS` — one `Name: value` per line, which is how that
 *    runtime parses it.
 *
 * Three variables written, and **more than three owned** — which is
 * [`CC_CONNECTION_VARIABLES`], and why the merge that uses this function is
 * [`ccEnvironment`] rather than a spread. That runtime reads a whole endpoint
 * table and a whole credential list — `ANTHROPIC_AUTH_TOKEN` is sent as a bearer
 * header beside the `X-Api-Key` the second variable sets, and
 * `CLAUDE_CODE_USE_BEDROCK` with its `ANTHROPIC_BEDROCK_BASE_URL` companion
 * chooses an endpoint instead of the first — so `validate` refuses a node `env:`
 * entry naming any of them for a fact this connection declares, not only the
 * three below (grammar Decision D143, PRD resolved q58 rulings a and c). The
 * node's own `env:` therefore needs no filtering here; the **inherited** half
 * does, and that is what [`ccEnvironment`] removes before this map goes over the
 * top.
 *
 * **An absent fact sets no variable**, which is the half a one-token slip would
 * turn into the opposite of q25's ruling: a keyless gateway connection must
 * leave the credential variable *unset*, not set to the empty string, or the
 * harness would authenticate as nobody instead of letting the gateway do it.
 * `validate` has already refused a node whose provider declares a fact this
 * harness has no slot for, so there is nothing here to drop.
 *
 * **The header encoding is the one slot that can be handed a value it cannot
 * carry**, and this is where that is caught. `ANTHROPIC_CUSTOM_HEADERS` is
 * newline-delimited and each line is split on its first colon, so a value
 * carrying a line break would declare one header and send two — the second one
 * spelled by the value, which is `x-api-key` as easily as anything else.
 * `validate` refuses such a value where the composition *wrote* it, but a
 * `${ENV}` is text this build never saw and a gateway deployment is exactly
 * where operators set those variables (PRD resolved q58 ruling a). So the
 * resolved value is asked the same question here, and a run is failed rather
 * than quietly sent with a header nobody declared. The message names the node
 * and the header and **never the value**, which is `docs/trace.md` §11.1's rule
 * about every artifact this project writes: a connection header is where a
 * gateway credential lives.
 */
function ccConnection(run: runtime.HarnessRun): Record<string, string> {
  const held: Record<string, string> = {};
  const connection = run.connection;
  if (connection.baseUrl !== undefined) held["ANTHROPIC_BASE_URL"] = connection.baseUrl;
  if (connection.credential !== undefined) held["ANTHROPIC_API_KEY"] = connection.credential;
  const headers = Object.entries(connection.headers ?? {});
  if (headers.length > 0) {
    for (const [name, value] of headers) {
      if (forgesAHeaderField(value)) {
        throw new Error(
          `\`${run.node}\`'s connection header \`${name}\` resolves to a value carrying a control character: ` +
            "`ANTHROPIC_CUSTOM_HEADERS` is one `Name: value` per line, so the run would send a header the composition never declared",
        );
      }
    }
    held["ANTHROPIC_CUSTOM_HEADERS"] = headers.map(([name, value]) => `${name}: ${value}`).join("\n");
  }
  return held;
}

/**
 * Every variable the pinned runtime reads for one connection fact — the slot
 * [`ccConnection`] writes **and** every other name that decides the same thing
 * (grammar 8.9, Decision D143, PRD resolved q58 rulings a and c).
 *
 * This is the compiler's own `cc` connection row, declared again for the
 * runtime, exactly as [`CC_SETTINGS`] is the curated settings table declared
 * again for [`passthrough`] — and held to it by
 * `the_cc_connection_variable_table_is_one_table` in `src/harness.rs`, because
 * two hand-maintained copies of one document drift in silence.
 *
 * It exists because a slot is not the only name that answers its question.
 * `ANTHROPIC_AUTH_TOKEN` is sent as `Authorization: Bearer …` *beside* the
 * `X-Api-Key` the credential slot sets; the `CLAUDE_CODE_USE_*` selectors each
 * choose an endpoint instead of `ANTHROPIC_BASE_URL`, and the
 * `*_FILE_DESCRIPTOR` names hand a credential over on a file descriptor rather
 * than in a value. Guarding only the three the map writes leaves every one of
 * those free to decide the fact the composition already decided.
 */
const CC_CONNECTION_VARIABLES: Readonly<Record<keyof runtime.HarnessConnection, readonly string[]>> =
  {
    baseUrl: [
      "ANTHROPIC_BASE_URL",
      "_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL",
      "ANTHROPIC_BEDROCK_BASE_URL",
      "CLAUDE_CODE_USE_BEDROCK",
      "ANTHROPIC_VERTEX_BASE_URL",
      "CLAUDE_CODE_USE_VERTEX",
      "ANTHROPIC_FOUNDRY_BASE_URL",
      "CLAUDE_CODE_USE_FOUNDRY",
      "ANTHROPIC_AWS_BASE_URL",
      "CLAUDE_CODE_USE_ANTHROPIC_AWS",
      "ANTHROPIC_GOOGLE_CLOUD_BASE_URL",
      "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
      "ANTHROPIC_BEDROCK_MANTLE_BASE_URL",
      "CLAUDE_CODE_USE_MANTLE",
      "CLAUDE_CODE_USE_GATEWAY",
    ],
    credential: [
      "ANTHROPIC_API_KEY",
      "ANTHROPIC_AUTH_TOKEN",
      "CLAUDE_CODE_OAUTH_TOKEN",
      "AWS_BEARER_TOKEN_BEDROCK",
      "ANTHROPIC_FOUNDRY_API_KEY",
      "ANTHROPIC_FOUNDRY_AUTH_TOKEN",
      "ANTHROPIC_AWS_API_KEY",
      "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
      "CLAUDE_CODE_SKIP_VERTEX_AUTH",
      "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
      "CLAUDE_CODE_SKIP_ANTHROPIC_AWS_AUTH",
      "CLAUDE_CODE_SKIP_ANTHROPIC_GOOGLE_CLOUD_AUTH",
      "CLAUDE_CODE_SKIP_MANTLE_AUTH",
      "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
      "CLAUDE_CODE_GATEWAY_TOKEN_FILE_DESCRIPTOR",
      "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
      "CLAUDE_CODE_WEBSOCKET_AUTH_FILE_DESCRIPTOR",
    ],
    headers: ["ANTHROPIC_CUSTOM_HEADERS"],
  };

/**
 * Whether the connection **declares** one fact, which is the same question
 * [`ccConnection`] asks before it writes that fact's slot.
 *
 * `headers:` is the one that is not a bare `!== undefined`: an empty header map
 * writes no variable, so it claims no name either — the two answers are one
 * answer, and the compiler's `declared` reads it the same way.
 */
function ccDeclares(
  connection: runtime.HarnessConnection,
  fact: keyof runtime.HarnessConnection,
): boolean {
  if (fact === "headers") return Object.keys(connection.headers ?? {}).length > 0;
  return connection[fact] !== undefined;
}

/**
 * The environment one `cc` run is handed: the node's own, **minus every name
 * this runtime reads for a fact the connection declares**, plus the connection
 * itself (PRD resolved q58 rulings a and c, q54 ruling b).
 *
 * The subtraction is the whole of this function, and `inherit_env: true` is why
 * it is not redundant. `validate` refuses a node `env:` entry spelling any name
 * [`CC_CONNECTION_VARIABLES`] claims for a declared fact, so the *declared* half
 * of the environment is already clean when it arrives. The **inherited** half
 * never passed through `validate` at all: it is whatever shell started this
 * process, and a machine that runs `claude` interactively is exactly the machine
 * that has `CLAUDE_CODE_USE_BEDROCK` and an `ANTHROPIC_AUTH_TOKEN` set. Merging
 * the map over the top only overwrites the three names it writes, so without
 * this a gateway-bound coder run would inherit a selector that repoints it, or a
 * bearer token sent beside the composition's own key — while `validate` stayed
 * clean and the graph document and the journal's `connection` both went on
 * reporting `ANTHROPIC_BASE_URL`. That is the failure the sibling list exists to
 * refuse, arriving through the one door ruling c cannot reach.
 *
 * **An undeclared fact claims nothing**, which is q25's keyless posture read
 * here rather than one surface along: a provider with no `api_key:` leaves the
 * whole credential family alone, so a node that inherits a shell holding
 * `ANTHROPIC_AUTH_TOKEN` keeps it — the connection says nothing about how this
 * run authenticates, and the compiler's own `variables_read` is computed from
 * the declared facts for the same reason.
 */
function ccEnvironment(run: runtime.HarnessRun): Record<string, string> {
  const held: Record<string, string> = { ...run.env };
  for (const [fact, names] of Object.entries(CC_CONNECTION_VARIABLES) as [
    keyof runtime.HarnessConnection,
    readonly string[],
  ][]) {
    if (!ccDeclares(run.connection, fact)) continue;
    for (const name of names) delete held[name];
  }
  return { ...held, ...ccConnection(run) };
}

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
 * the model is never offered — the availability bound, which holds whatever the
 * permission mode is. `canUseTool` is the per-call gate over it, consulted
 * where a call would otherwise stop to ask: it answers `allow` for a name
 * inside the list, so a bounded run is not also a prompting one, and it denies
 * anything outside the list that reached the loop anyway, taping the denial as
 * a `"refused"` tool event so the trace says the bound bit.
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
 * **And not three: there is no `allowedTools`**, though it reads like the way
 * to keep a bounded run from prompting. A bare name there approves the whole
 * tool *before* the callback is consulted, so beside `canUseTool` it shadows
 * the callback — and the pinned SDK does not leave that silent: `query()`
 * reports the pairing as a shadowed callback under
 * `CLAUDE_SDK_CAN_USE_TOOL_SHADOWED`, naming every bare entry the callback
 * will never be asked about. The callback already answers `allow` for exactly
 * those names, so the list's whole decision is made in one place — the place a
 * refusal is taped — rather than half of it ahead of the callback, where the
 * trace cannot see it. `allowedTools` stays on [`CC_RESERVED`] for the same
 * reason: a `settings:` key spelling it would put the shadow back.
 *
 * One shadow is accepted, and it is the hole above: under `bypassPermissions`
 * the SDK reports the callback as shadowed under the same code, because that
 * mode approves every call before any callback runs. The callback is kept there
 * all the same — it is one options object for every mode, and under that mode
 * the bound was never the callback's to hold: `tools` holds it.
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
  // The node's approval mode, resolved once: the three options below that
  // answer to it read this value rather than `run.access`, so a stated mode and
  // the flags its SDK requires can never disagree (see [`ccPermissionMode`]).
  const mode = ccPermissionMode(run);
  const options: Options = {
    cwd: run.workspace,
    systemPrompt: { type: "preset", preset: "claude_code", append: run.instructions },
    model: run.model,
    permissionMode: mode,
    abortController: controllerFor(run.signal),
    // The node's environment with the model's connection mapped over the top —
    // and, first, with every other name this runtime reads for a declared fact
    // taken out of it. `validate` refuses an `env:` entry spelling one of those,
    // so the *declared* half needs no filtering; `inherit_env: true` is the half
    // no compile step ever saw, and a merge order alone could not have fixed it
    // (see [`ccEnvironment`], PRD resolved q58 ruling c).
    env: ccEnvironment(run),
    outputFormat: { type: "json_schema", schema: { ...run.schema } },
    ...passthrough(run, CC_SETTINGS, CC_RESERVED),
  };
  // The flag the SDK documents `bypassPermissions` as requiring, and nothing
  // else does — so it is armed by the mode rather than by the preset that
  // usually derives it (see [`ccPermissionMode`]).
  if (mode === "bypassPermissions") options.allowDangerouslySkipPermissions = true;
  // Plan mode's body, replaced by what this node asked for: the mode's
  // default body is a code-implementation workflow, and a node under `plan`
  // is a run that reads and reports (see [`CC_PERMISSION`]).
  if (mode === "plan") options.planModeInstructions = run.instructions;
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
    // …and the per-call gate over it, which is also what keeps a bounded run
    // from prompting: an in-list call is answered `allow` here. Nothing
    // approves the list ahead of this callback — a bare `allowedTools` would,
    // and the pinned SDK flags that pairing as a shadowed callback (see
    // [`CC_DRIVER`]).
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
