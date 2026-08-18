// What an `http:` binding actually puts on the wire.
//
// Two claims about one request, both invisible to `tsc` and to graph
// construction, and both driven through a generated project's own runtime
// against a server started here on loopback.
//
// **The media type.** Grammar 6.1 says header names are case-insensitive;
// `fetch` is not. It builds its `Headers` from the object it is handed by
// **appending**, so `Content-Type` declared beside the `content-type` `runHttp`
// sends with a JSON body arrives at the server as one field carrying both media
// types, comma-joined. A real API that dispatches on it answers 415 to a
// composition that reads correctly.
//
// **The query string.** Without `query:`/`body:`, grammar 6.1 sends the bound
// input object as the JSON body on a body-bearing method and as query
// **parameters** on `GET`/`HEAD` — so the object handed to `runHttp` as `query`
// has to become the URL's parameters, values that are not strings included.
//
// Usage: node http-request.mjs <generated project directory>
// Output: { "declared": …, "default": …, "query": … } as JSON, each recording
// what the server received.

import http from "node:http";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node http-request.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

const MEDIA_TYPE = "application/vnd.acme+json";

const received = [];
const server = http.createServer((request, response) => {
  let body = "";
  request.on("data", (chunk) => {
    body += chunk;
  });
  request.on("end", () => {
    received.push({
      header: request.headers["content-type"] ?? null,
      body,
      url: request.url,
    });
    response.writeHead(200, { "content-type": "application/json" });
    response.end(JSON.stringify({ ok: true }));
  });
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const { port } = server.address();

const context = {
  execution: { id: "exec_http_request", session_key: "" },
  signal: new AbortController().signal,
  node: "notify",
};

const binding = (method, headers) => ({
  method,
  url: [`http://127.0.0.1:${port}/tickets`],
  headers,
  expectStatus: "2xx",
  decoding: { envelope: [], decoded: ["ok"], empty: false },
});

try {
  // The author's spelling is the capitalised one on purpose: folding the name is
  // the whole of the fix, and a runtime that compared spellings would pass a
  // lower-case-only test.
  await runtime.runHttp(
    binding("POST", [{ name: "Content-Type", value: [MEDIA_TYPE] }]),
    { body: { goal: "g" } },
    context,
  );
  await runtime.runHttp(binding("POST", []), { body: { goal: "g" } }, context);
  // The `GET` half of the same convention: an input object, sent as parameters.
  // `limit` is a number, because a query string carries text and something has
  // to decide how a non-string is spelled.
  await runtime.runHttp(
    binding("GET", []),
    { query: { term: "a widget", limit: 3 } },
    context,
  );
  process.stdout.write(
    JSON.stringify({ declared: received[0], default: received[1], query: received[2] }),
  );
} finally {
  server.close();
}
