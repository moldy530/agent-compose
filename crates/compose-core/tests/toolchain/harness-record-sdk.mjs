// What a `HarnessRecord.sdk` names: the SDK package and the version of it that
// **ran**, read off the installed package's own manifest — not the number this
// compiler release pinned (`docs/trace.md` §7.6).
//
// The two are one number in a project installed from the `package.json` that
// `build` wrote, which is why no other gate can tell them apart: every golden
// here is installed from exactly those pins. What differs is the project whose
// manifest was edited after `build` — the case `build --check` reports — and a
// record that named the pin there would say a release ran that did not. So this
// installs **something else** under a staged golden and runs the golden's own
// emitted drivers over it:
//
//   * `pinned` — nothing staged over the shared install, which `bun install`
//     made from the pins, so the drivers must report exactly the pins: the
//     conformant case, read through the real packages' layouts (neither
//     publishes `./package.json` in its `exports`). A driver that *fell back*
//     would report the pins too, so this copy's compiled-in pins are replaced
//     with a sentinel first — the pin a driver reports here can only have come
//     off the installed package's manifest;
//   * `installed` — a stand-in for each SDK is installed into the project's own
//     `node_modules/`, at a version the compiler never pinned, and `flow.patch`
//     runs end to end through `src/graph.ts`'s emitted bindings. Nothing is
//     registered over the drivers: the `cc` driver's `ccOptions`/`ccEvents` and
//     the `codex` driver's own event mapping are the code under test, and the
//     stand-ins answer the way the vendors' SDKs do. Every record names the
//     stand-in's version;
//   * `unreadable` — the same stand-ins with no `version` in their manifests.
//     The drivers cannot read what ran, and the records fall back to the pin
//     rather than failing the run or naming nothing;
//   * `renamed` — stand-ins whose root manifests do not carry the name the
//     project imports them by. The `cc` one names a fork, at a version no
//     release pins, which is the manifest `npm i <sdk>@npm:<fork>@<version>`
//     installs: the fork ran, so its record names the fork's version under the
//     SDK's own name. The `codex` one names nothing and carries no version, so
//     the only version walking up from it is the project's own `package.json`:
//     its record falls back to the pin rather than borrowing that.
//
// Every stand-in keeps its entry point in `dist/`, below a nested
// `dist/package.json` carrying only a `type`, so a lookup that took the first
// manifest it met rather than the package root's would read no version in any
// mode.
//
// Usage: bun harness-record-sdk.mjs <generated project directory> <scratch dir> <pinned|installed|unreadable|renamed>
// Output: one JSON object, read by `generated_code_gates.rs`.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch, mode] = process.argv;
const MODES = ["pinned", "installed", "unreadable", "renamed"];
if (project === undefined || scratch === undefined || !MODES.includes(mode)) {
  throw new Error(
    `usage: bun harness-record-sdk.mjs <generated project directory> <scratch dir> <${MODES.join("|")}>`,
  );
}

/** The two packages the emitted drivers import. */
const CC_SDK = "@anthropic-ai/claude-agent-sdk";
const CODEX_SDK = "@openai/codex-sdk";

/** The version the stand-ins are installed at — one no compiler release pins. */
const INSTALLED = "9.9.9-edited";

/** The version the `renamed` fork of the Agent SDK is installed at. */
const FORKED = "9.9.9-forked";

/** The name the `renamed` fork's own manifest carries. */
const FORK = "@someone/claude-agent-sdk-fork";

/**
 * Install one stand-in package into the project's own `node_modules/`, where
 * resolution from `src/` finds it ahead of the shared install above.
 * `identity` is the `name` and `version` its root manifest carries, either of
 * them absent where the mode says so. The entry point sits in `dist/`, below a
 * nested manifest carrying only a `type`.
 */
function standIn(name, source, identity) {
  const directory = path.join(project, "node_modules", ...name.split("/"));
  fs.mkdirSync(path.join(directory, "dist"), { recursive: true });
  const manifest = {
    ...identity,
    type: "module",
    main: "./dist/index.mjs",
    exports: { ".": "./dist/index.mjs" },
  };
  fs.writeFileSync(path.join(directory, "package.json"), JSON.stringify(manifest, null, 2));
  fs.writeFileSync(path.join(directory, "dist", "package.json"), JSON.stringify({ type: "module" }, null, 2));
  fs.writeFileSync(path.join(directory, "dist", "index.mjs"), source);
  return pathToFileURL(path.join(directory, "dist", "index.mjs")).href;
}

/** What each stand-in's root manifest carries, per mode. */
function identityOf(name) {
  switch (mode) {
    case "installed":
      return { name, version: INSTALLED };
    case "unreadable":
      return { name };
    case "renamed":
      return name === CC_SDK ? { name: FORK, version: FORKED } : {};
    default:
      throw new Error(`no stand-ins under \`${mode}\``);
  }
}

// The Agent SDK's `query`, answering with the two messages the `cc` driver
// reads a run off: a top-level assistant turn, and a successful result carrying
// the structured output the node's `output:` asked for.
const CC_STAND_IN = `
export const calls = [];
export function query({ prompt, options }) {
  calls.push({ prompt, cwd: options.cwd, permissionMode: options.permissionMode, tools: options.tools });
  return (async function* answered() {
    yield { type: "assistant", parent_tool_use_id: null, message: { content: [] } };
    yield {
      type: "result",
      subtype: "success",
      parent_tool_use_id: null,
      usage: { input_tokens: 10, output_tokens: 5 },
      total_cost_usd: 0.001,
      stop_reason: "end_turn",
      structured_output: { summary: "stood in", touched: ["src/lib.rs"] },
    };
  })();
}
`;

// The Codex SDK's client, answering one turn with an approving verdict as the
// JSON text of its last agent message.
const CODEX_STAND_IN = `
export const calls = [];
export class Codex {
  constructor(options) {
    calls.push({ baseUrl: options.baseUrl });
  }
  startThread(options) {
    return {
      async runStreamed(_turn, _options) {
        return {
          events: (async function* streamed() {
            yield {
              type: "turn.completed",
              usage: {
                input_tokens: 7,
                cached_input_tokens: 0,
                cache_write_input_tokens: 0,
                output_tokens: 3,
                reasoning_output_tokens: 0,
              },
            };
            yield {
              type: "item.completed",
              item: { id: "m1", type: "agent_message", text: JSON.stringify({ verdict: "approve", feedback: "" }) },
            };
          })(),
        };
      },
    };
  }
}
`;

/** What this copy's compiled-in pins read as under `pinned` — never a real version. */
const NOT_READ = "0.0.0-fell-back-to-the-compiled-pin";

const results = { mode };
let ccModule;
let codexModule;
if (mode === "pinned") {
  const file = path.join(project, "src", "harness.ts");
  const source = fs.readFileSync(file, "utf8");
  let replaced = 0;
  const rewritten = source.replace(
    /^const (CC|CODEX)_SDK_VERSION: string = "[^"]*";$/gm,
    (_line, harness) => {
      replaced += 1;
      return `const ${harness}_SDK_VERSION: string = ${JSON.stringify(NOT_READ)};`;
    },
  );
  if (replaced !== 2) {
    throw new Error(`expected both pin constants in ${file}, replaced ${replaced}`);
  }
  fs.writeFileSync(file, rewritten);
} else {
  ccModule = standIn(CC_SDK, CC_STAND_IN, identityOf(CC_SDK));
  codexModule = standIn(CODEX_SDK, CODEX_STAND_IN, identityOf(CODEX_SDK));
}

fs.mkdirSync(scratch, { recursive: true });
const workspace = path.join(scratch, "checkout");
fs.mkdirSync(workspace, { recursive: true });
// The composition's own `${ENV}` references, set before the barrel is loaded —
// the same set `coder-graph.mjs` gives this golden, for its reason.
process.env["REPO_ROOT"] = workspace;
process.env["ANTHROPIC_API_KEY"] = "harness-key-anthropic";
process.env["OPENAI_API_KEY"] = "harness-key-openai";
process.env["LLM_GATEWAY_URL"] = "https://gateway.invalid/v1";
process.env["TEAM_NAME"] = "platform";
process.env["AGENT_COMPOSE_DATA"] = path.join(scratch, "data");

const harness = await import(pathToFileURL(path.resolve(project, "src/harness.ts")).href);
results["drivers"] = {
  cc: { sdk: harness.DRIVERS.cc.sdk, version: harness.DRIVERS.cc.version },
  codex: { sdk: harness.DRIVERS.codex.sdk, version: harness.DRIVERS.codex.version },
};

if (mode !== "pinned") {
  const { runFlow } = await import(pathToFileURL(path.resolve(project, "src/index.ts")).href);
  const run = await runFlow("flow.patch", { goal: "make the failing test pass" });
  results["outputs"] = run.outputs;
  results["records"] = run.trace.flatMap((entry) =>
    (entry.harness ?? []).map((record) => ({
      node: entry.node,
      harness: record.harness,
      outcome: record.outcome,
      sdk: record.sdk,
    })),
  );
  // The stand-ins are what the emitted drivers really called — not a driver
  // registered over them — so each was reached exactly once, by the node that
  // binds its harness.
  results["standInCalls"] = {
    cc: (await import(ccModule)).calls.length,
    codex: (await import(codexModule)).calls.length,
  };
}

process.stdout.write(JSON.stringify(results, null, 1));
