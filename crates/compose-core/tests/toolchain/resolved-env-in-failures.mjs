// The three failures the **platform** words, run against a generated project's
// own runtime (`docs/trace.md` §11.2).
//
// §11.1 promises a public surface: no resolved `${ENV}` value appears in a
// trace. The messages this runtime composes itself keep that promise by
// construction — they quote `asWritten`, and a source-level check holds them to
// it. These three do not go through a message this runtime composed at all:
//
//   * a `command:` (or `cwd:`) the OS refused to spawn. Node reports
//     `spawn /opt/secret/rg ENOENT`, Bun `ENOENT: no such file or directory,
//     posix_spawn '/opt/secret/rg'` — the resolved path, in both;
//   * an `http:` `url` that is not a URL once its references resolve. Bun
//     quotes the resolved string, Node does not;
//   * a provider whose resolved `base_url:` `fetch` cannot parse. Node quotes
//     the resolved string, Bun does not.
//
// Which engine quotes which is exactly why this is a runner rather than a unit
// test: the wording is the platform's, it differs between the two supported
// runtimes (PRD §9.18), and a check that asked only one would be evidence for
// whichever half it happened to run. What is asserted is the same for both — the
// secret is not in the message, and the reference is.
//
// A fourth message is probed for the opposite reason: a reference that is not
// set at all is worded by this runtime, precisely, and the three restatements
// above must not stand in front of it. See `unsetUrl`/`unsetCwd` below.
//
// Usage: node resolved-env-in-failures.mjs <generated project directory>
// Output: { "secret": …, "exec": …, "http": …, "provider": …, "unsetUrl": …,
// "unsetCwd": … } as JSON, each message being the one a node's `error` would
// have carried.

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node resolved-env-in-failures.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

// One value, referenced from all three surfaces, so a single search over every
// message answers the whole question. It is not a path that exists and not a
// URL that parses, which is what makes each of the three fail where it does.
const SECRET = "/opt/secret-token-abc";
process.env["OPS_BIN"] = SECRET;
process.env["SIGNED_ENDPOINT"] = SECRET;

const context = {
  execution: { id: "exec_resolved_env", session_key: "" },
  signal: new AbortController().signal,
  node: "probe",
};

/** What `await run()` failed with, as a node's `error` would spell it. */
async function refusal(run) {
  try {
    await run();
  } catch (error) {
    return error instanceof Error ? `${error.name}: ${error.message}` : String(error);
  }
  throw new Error("the probe was expected to fail and did not");
}

const exec = await refusal(() =>
  runtime.runExec(
    {
      command: [{ env: "OPS_BIN", site: "`tool.probe` `exec.command`" }, "/nothing-here"],
      args: [],
      env: [],
      expectExit: [0],
      decoding: { envelope: ["exit_code", "stdout"], decoded: [], empty: false },
    },
    undefined,
    context,
  ),
);

const http = await refusal(() =>
  runtime.runHttp(
    {
      method: "GET",
      url: [{ env: "SIGNED_ENDPOINT", site: "`tool.probe` `http.url`" }, "/reports"],
      headers: [],
      expectStatus: "2xx",
      decoding: { envelope: [], decoded: ["ok"], empty: false },
    },
    {},
    context,
  ),
);

// A direct binding (grammar 12.2), which is a ladder of one: the refusal it ends
// on is the message a `Refusal.detail` carries and, through it, the node's own
// `error`.
const provider = await refusal(() =>
  runtime.callModel(
    {
      address: "model.smart",
      id: "claude-sonnet-4-6",
      provider: {
        address: "provider.acme",
        kind: "anthropic",
        apiKey: "unused",
        baseUrl: SECRET,
      },
      settings: {},
    },
    { system: "", turns: [{ role: "user", text: "hello" }], tools: [] },
    { signal: new AbortController().signal },
  ),
);

// The other fault at the same two sites, and the reason both are probed here:
// a reference that is **not set** at all. `environmentValue` words that one
// itself — `` `REPO_ROOT` is not set (referenced by …) `` — and it is the whole
// diagnosis, so the restatements above must not swallow it. They would if the
// interpolation happened inside the guarded region: an unset `url:` reference
// throwing inside `runHttp`'s `try` reappears as "is not a URL once its
// `${ENV}` references are resolved", which is false — nothing resolved — and
// identical to what a reference *set* to a non-URL produces, so `TraceEntry.error`
// could no longer tell the two apart. An unset `cwd:` reference throwing inside
// `runExec`'s `new Promise` executor reappears as "`<command>` could not be
// run", which names the command for a fault in the working directory.
//
// `cwd:` rather than `command:` for the `exec:` half on purpose: it is the
// interpolation furthest from the resolved ones, and the goldens' `env.ts` puts
// real references there (`tool.repo_grep.exec.cwd`).
delete process.env["UNSET_ENDPOINT"];
delete process.env["UNSET_ROOT"];

const unsetUrl = await refusal(() =>
  runtime.runHttp(
    {
      method: "GET",
      url: [{ env: "UNSET_ENDPOINT", site: "`tool.probe` `http.url`" }, "/reports"],
      headers: [],
      expectStatus: "2xx",
      decoding: { envelope: [], decoded: ["ok"], empty: false },
    },
    {},
    context,
  ),
);

const unsetCwd = await refusal(() =>
  runtime.runExec(
    {
      // Resolvable, and a command that exists: the only thing wrong here is the
      // working directory, so a message about anything else is a misattribution.
      command: ["echo"],
      args: [],
      cwd: [{ env: "UNSET_ROOT", site: "`tool.probe` `exec.cwd`" }],
      env: [],
      expectExit: [0],
      decoding: { envelope: ["exit_code", "stdout"], decoded: [], empty: false },
    },
    undefined,
    context,
  ),
);

process.stdout.write(
  JSON.stringify({ secret: SECRET, exec, http, provider, unsetUrl, unsetCwd }),
);
