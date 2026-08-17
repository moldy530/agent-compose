// What the single string-typed property of a `tool.*` takes from each surface.
//
// Grammar 6.1 states the exception twice, once per binding, and the two
// sentences differ by one word: the `exec` binding says **trimmed** raw stdout,
// the `http` binding says the raw response **text**. This runs one payload —
// whitespace at both ends — through both surfaces of a generated project's own
// runtime and reports what each bound, so which reading this compiler took is a
// committed fact rather than an accident of a shared helper.
//
// The HTTP half serves the payload from a server this script starts on
// loopback, because the claim is about a response body rather than about any
// particular server.
//
// Usage: node raw-decoding.mjs <generated project directory>
// Output: { "sent": …, "exec": …, "http": … } as JSON, where the two values are
// what each binding bound to the property.

import http from "node:http";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node raw-decoding.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

// Leading and trailing whitespace, so trimming is visible from either end.
const PAYLOAD = "  a body\n";

// One `tool.*` result with exactly one string-typed property, which is the
// shape the exception is about (grammar 6.1, Decision D109).
const decoding = { envelope: [], decoded: [], raw: "text", empty: false };
const context = {
  execution: { id: "exec_raw_decoding", session_key: "" },
  signal: new AbortController().signal,
  node: "probe",
};

const server = http.createServer((_request, response) => {
  response.writeHead(200, { "content-type": "text/plain" });
  response.end(PAYLOAD);
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const { port } = server.address();

try {
  const fromExec = await runtime.runExec(
    {
      command: ["printf"],
      args: [["%s"], [PAYLOAD]],
      env: [],
      expectExit: [0],
      decoding,
    },
    {},
    context,
  );
  const fromHttp = await runtime.runHttp(
    {
      method: "GET",
      url: [`http://127.0.0.1:${port}/`],
      headers: [],
      expectStatus: "2xx",
      decoding,
    },
    {},
    context,
  );
  process.stdout.write(JSON.stringify({ sent: PAYLOAD, exec: fromExec, http: fromHttp }));
} finally {
  server.close();
}
