// Drives a generated project's `src/stores.ts` — the local `kv`, `vector` and
// `blob` backends PRD 5.8's zero-infra guarantee promises — and reports what
// each op answered.
//
// The bindings are written here rather than read out of `src/graph.ts`, and that
// is deliberate: what `graph.ts` emits for a `store.*` is already held by the
// golden corpus and by the acceptance suite, while what the *backends* do with
// one — the scope partitions, the idempotency ledger, the key encoding — is
// behaviour no committed diff can show. So this constructs the bindings and
// exercises the module directly, which is also what lets one runner answer for
// both supported runtimes (PRD §9.18).
//
// `vector` is not here: its ops embed through a real provider connection
// (grammar 11.2), which is a network round trip and belongs in the acceptance
// suite, where there is a scripted provider to answer it.
//
// Usage: node store-backends.mjs <generated project directory> <data directory>
// Output: one JSON object of everything the gate asserts about.

import fs from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project, data] = process.argv;
if (project === undefined || data === undefined) {
  throw new Error("usage: node store-backends.mjs <project> <data directory>");
}

// Read by `dataRoot()` at every call, so setting it before the import is not
// required — but setting it here keeps the whole run inside one directory the
// caller can throw away.
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/** A store-op node's context, with the record channel a trace entry reads. */
const context = (id, session = "") => ({
  execution: { id, session_key: session },
  signal: new AbortController().signal,
  node: "probe",
  storeRecords: [],
});

/** One binding, with the local backend the `local` target resolves. */
const binding = (name, kind, scope, extra = {}) => ({
  address: `store.${name}`,
  name,
  kind,
  scope,
  metadata: false,
  backend: {
    provider: kind === "kv" ? "sqlite" : kind === "vector" ? "sqlite_vec" : "local_fs",
    from: "the `local` target substitutes local storage for every store unconditionally",
  },
  ...extra,
});

const node = (key) => ({ via: "node", ...(key === undefined ? {} : { idempotencyKey: key }) });

/** What a call threw, or `null` when it did not throw. */
const refusal = async (work) => {
  try {
    await work();
    return null;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
};

const answer = {};

// --- `kv`, at `global` scope: the whole catalogue, and the ledger ------------

{
  const store = binding("prefs", "kv", "global");
  const run = context("exec_kv");
  await stores.runStoreOp(store, "set", { key: "a", value: { theme: "dark" } }, run, node("k/1"));
  await stores.runStoreOp(store, "set", { key: "b", value: { theme: "light" } }, run, node("k/2"));
  answer.kvHit = await stores.runStoreOp(store, "get", { key: "a" }, run, node());
  answer.kvMiss = await stores.runStoreOp(store, "get", { key: "z" }, run, node());
  answer.kvList = await stores.runStoreOp(store, "list", { limit: 10 }, run, node());
  answer.kvPrefixed = await stores.runStoreOp(store, "list", { prefix: "b", limit: 10 }, run, node());
  answer.kvDeleted = await stores.runStoreOp(store, "delete", { key: "b" }, run, node("k/3"));
  answer.kvDeletedAgain = await stores.runStoreOp(store, "delete", { key: "b" }, run, node("k/4"));

  // At-least-once, deduped on the key of grammar 9.4: the second write carries
  // the key the first did, so the effect does not happen twice and the answer is
  // the one the first attempt gave.
  const repeated = await stores.runStoreOp(
    store,
    "set",
    { key: "a", value: { theme: "REWRITTEN" } },
    run,
    node("k/1"),
  );
  answer.dedupedWrite = repeated;
  answer.afterDedupedWrite = await stores.runStoreOp(store, "get", { key: "a" }, run, node());

  // …and a write under a *different* key is a different effect.
  await stores.runStoreOp(store, "set", { key: "a", value: { theme: "changed" } }, run, node("k/5"));
  answer.afterSecondWrite = await stores.runStoreOp(store, "get", { key: "a" }, run, node());

  answer.records = run.storeRecords;
}

// --- A `prefix:` outside the BMP, which two string lengths disagree about -----

{
  // Grammar 11.4 puts no character restriction on `prefix:`, and the two sides
  // of the comparison count characters differently: SQLite's `substr` counts
  // **characters**, a JavaScript `.length` counts UTF-16 code units, and an
  // astral character is one of the first and two of the second. A prefix filter
  // that mixed the two answers a legal read with the wrong keys — silently, and
  // the answer goes into the trace as history.
  const store = binding("emoji", "kv", "global");
  const run = context("exec_astral");
  await stores.runStoreOp(store, "set", { key: "😀alpha", value: { n: 1 } }, run, node("a/1"));
  await stores.runStoreOp(store, "set", { key: "😀beta", value: { n: 2 } }, run, node("a/2"));
  await stores.runStoreOp(store, "set", { key: "zzz", value: { n: 3 } }, run, node("a/3"));
  answer.astralAll = await stores.runStoreOp(store, "list", { limit: 10 }, run, node());
  answer.astralPrefix = await stores.runStoreOp(
    store,
    "list",
    { prefix: "😀", limit: 10 },
    run,
    node(),
  );
  answer.astralDeeper = await stores.runStoreOp(
    store,
    "list",
    { prefix: "😀al", limit: 10 },
    run,
    node(),
  );
  answer.astralMiss = await stores.runStoreOp(
    store,
    "list",
    { prefix: "😀gamma", limit: 10 },
    run,
    node(),
  );

  // The same three prefixes against a `blob` store, whose `list` is JavaScript
  // rather than SQL: the two backends answer one op of grammar 11.4's catalogue,
  // so they have to answer it the same way.
  const blobs = binding("emoji_blobs", "blob", "global");
  await stores.runStoreOp(blobs, "put", { key: "😀alpha", value: "one" }, run, node("a/4"));
  await stores.runStoreOp(blobs, "put", { key: "😀beta", value: "two" }, run, node("a/5"));
  await stores.runStoreOp(blobs, "put", { key: "zzz", value: "three" }, run, node("a/6"));
  answer.astralBlobPrefix = await stores.runStoreOp(
    blobs,
    "list",
    { prefix: "😀", limit: 10 },
    run,
    node(),
  );
}

// --- `list` answers one key order, whichever backend holds the keys ----------

{
  // Grammar 11.4 fixes no order for `list`, but it is one row of one catalogue
  // and the two backends answer it from different machinery: SQLite's
  // `ORDER BY key`, which compares UTF-8 bytes, and a sort of file names, which
  // in JavaScript compares UTF-16 code units. The two disagree above the BMP —
  // U+FF00 is `EF BC 80` and U+1F600 is `F0 9F 98 80`, so bytes put U+FF00
  // first, while the surrogate `D83D` puts U+1F600 first — and with the
  // `limit:` grammar 11.4 requires, that is not a different order but a
  // different answer.
  const written = ["zz", "＀", "\u{1F600}"];
  const kv = binding("ordered", "kv", "global");
  const blobs = binding("ordered_blobs", "blob", "global");
  const run = context("exec_order");
  for (const [index, key] of written.entries()) {
    await stores.runStoreOp(kv, "set", { key, value: { n: index } }, run, node(`o/${index}`));
    await stores.runStoreOp(blobs, "put", { key, value: String(index) }, run, node(`ob/${index}`));
  }
  answer.kvOrder = (await stores.runStoreOp(kv, "list", { limit: 10 }, run, node())).keys;
  answer.blobOrder = (await stores.runStoreOp(blobs, "list", { limit: 10 }, run, node())).keys;
  answer.kvOrderLimited = (await stores.runStoreOp(kv, "list", { limit: 2 }, run, node())).keys;
  answer.blobOrderLimited = (await stores.runStoreOp(blobs, "list", { limit: 2 }, run, node())).keys;
}

// --- a keyed `blob` write whose idempotency key is a deep instance path -------

{
  // Grammar 9.4's key is **composed** — the execution, the node, the attempt,
  // and one frame per enclosing `map` — so its length is a property of the graph
  // rather than of anything an author typed. The `blob` backend records it in a
  // ledger of one file per key, and a name is capped at 255 bytes, so a key long
  // enough to exceed that has to be recorded some other way: the alternative is
  // an `ENAMETOOLONG` raised **after** the effect landed and before the marker
  // that dedupes it, which turns the next attempt into a second write.
  const store = binding("deep", "blob", "global");
  const run = context("exec_deep");
  const frames = Array.from({ length: 12 }, (_, index) => `node_with_a_fairly_long_name_${index}/0`);
  const key = `exec_deep/${frames.join("/")}`;
  answer.deepKeyLength = key.length;
  answer.deepWrite = await stores.runStoreOp(store, "put", { key: "deep.txt", value: "first" }, run, node(key));
  // The retry of that same effect site: deduped rather than applied twice, which
  // is only possible if the first attempt's ledger entry was written.
  answer.deepRewrite = await stores.runStoreOp(
    store,
    "put",
    { key: "deep.txt", value: "REWRITTEN" },
    run,
    node(key),
  );
  answer.deepValue = await stores.runStoreOp(store, "get", { key: "deep.txt" }, run, node());
  answer.deepDeduped = run.storeRecords
    .filter((record) => record.idempotencyKey === key)
    .map((record) => record.deduped);
}

// --- `session`: one file, one partition per key ------------------------------

{
  const store = binding("memory", "kv", "session");
  const first = context("exec_s1", "session-a");
  const second = context("exec_s2", "session-a");
  const other = context("exec_s3", "session-b");
  await stores.runStoreOp(store, "set", { key: "note", value: { text: "mine" } }, first, node("s/1"));
  answer.sessionSame = await stores.runStoreOp(store, "get", { key: "note" }, second, node());
  answer.sessionOther = await stores.runStoreOp(store, "get", { key: "note" }, other, node());
  answer.sessionUnkeyed = await refusal(() =>
    stores.runStoreOp(store, "get", { key: "note" }, context("exec_s4"), node()),
  );
}

// --- `execution`: dies with the run -----------------------------------------

{
  const store = binding("scratch", "kv", "execution");
  const run = context("exec_e1");
  await stores.runStoreOp(store, "set", { key: "k", value: { text: "held" } }, run, node("e/1"));
  answer.executionHit = await stores.runStoreOp(store, "get", { key: "k" }, run, node());
  stores.releaseExecution("exec_e1");
  answer.executionAfterRelease = await stores.runStoreOp(store, "get", { key: "k" }, run, node());
  // A second execution never saw the first's, released or not.
  answer.executionOther = await stores.runStoreOp(
    store,
    "get",
    { key: "k" },
    context("exec_e2"),
    node(),
  );
}

// --- `blob`: a directory of files, keyed reversibly --------------------------

{
  const store = binding("artifacts", "blob", "global");
  const run = context("exec_blob");
  await stores.runStoreOp(
    store,
    "put",
    { key: "notes/one.txt", value: "first", contentType: "text/plain" },
    run,
    node("b/1"),
  );
  await stores.runStoreOp(store, "put", { key: "notes/two.txt", value: "second" }, run, node("b/2"));
  await stores.runStoreOp(store, "put", { key: "other", value: "third" }, run, node("b/3"));
  answer.blobHit = await stores.runStoreOp(store, "get", { key: "notes/one.txt" }, run, node());
  answer.blobMiss = await stores.runStoreOp(store, "get", { key: "notes/none" }, run, node());
  // The keys come back as they were written, which is what makes the file-name
  // encoding reversible rather than merely safe.
  answer.blobList = await stores.runStoreOp(store, "list", { limit: 10 }, run, node());
  answer.blobPrefixed = await stores.runStoreOp(
    store,
    "list",
    { prefix: "notes/", limit: 10 },
    run,
    node(),
  );
  answer.blobDeleted = await stores.runStoreOp(store, "delete", { key: "other" }, run, node("b/4"));
  answer.blobLimited = await stores.runStoreOp(store, "list", { limit: 1 }, run, node());
  answer.blobEmptyKey = await refusal(() =>
    stores.runStoreOp(store, "get", { key: "" }, run, node()),
  );
  answer.blobLongKey = await refusal(() =>
    stores.runStoreOp(store, "get", { key: "x".repeat(300) }, run, node()),
  );
  // A key that percent-encodes to nothing a path can climb out of.
  answer.blobEncoded = stores.encodeKey("../escape me");
}

// --- The lock a killed writer left behind ------------------------------------

// This driver's virtual file system takes SQLite's lock by creating
// `<file>.lock` as a **directory** and gives it back by removing it, so a `run`
// killed inside a store write never gives it back and nothing else ever will.
// Left standing it refuses every later open of that store — not only the resume
// of the execution the crash interrupted, whose live ops past the frontier are
// promised the world the recorded prefix left behind (`docs/durability.md` §5),
// but every future run of the project. `./journal.ts` breaks such a lock for the
// other file in this directory; a store that did not would be a second artifact
// one crash can permanently seal.
//
// The lock is planted rather than raced for, for the reason the journal's own
// test plants one: the window a real crash has to land in is one statement wide.

{
  const store = binding("sealed", "kv", "global");
  const run = context("exec_sealed");
  const file = path.join(data, "stores", `${stores.encodeKey("sealed")}.sqlite`);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.mkdirSync(`${file}.lock`, { recursive: true });

  answer.sealedWrite = await refusal(() =>
    stores.runStoreOp(store, "set", { key: "a", value: { theme: "dark" } }, run, node("s/1")),
  );
  answer.sealedRead = await stores.runStoreOp(store, "get", { key: "a" }, run, node());
  answer.sealedLockGone = !fs.existsSync(`${file}.lock`);
}

// --- A backend this compiler release does not implement ----------------------

{
  const store = binding("remote", "kv", "global", {
    backend: { provider: "redis", from: "the `kv` default of the `staging` target" },
  });
  answer.productionBackend = await refusal(() =>
    stores.runStoreOp(store, "get", { key: "a" }, context("exec_r"), node()),
  );
}

// --- …and the two the release does, reached out of a build that has neither ---

// This golden's target is `local`, so `src/stores.ts` here is the invariant half
// alone: no arm, no driver, and no `package.json` entry for one (PRD resolved
// q63). Both of the ways a binding can arrive at that half are refused by name
// rather than by an `undefined`, because both are a deploy file's mistake or a
// build's and an author has to be able to tell which.

{
  // A dialled backend whose entry declared no `url:`: there is nowhere to go.
  const store = binding("addressless", "kv", "global", {
    backend: { provider: "postgres", from: "the alias `prefs_db`, defined by the `staging` target" },
  });
  answer.dialledWithoutAnAddress = await refusal(() =>
    stores.runStoreOp(store, "get", { key: "a" }, context("exec_a"), node()),
  );
}

{
  // …and one with an address, in a project `build` emitted no arm into.
  process.env["STORE_BACKENDS_PROBE_URL"] = "postgres://user:pass@127.0.0.1:5432/nothing";
  const store = binding("armless", "kv", "global", {
    backend: {
      provider: "mysql",
      from: "the alias `notes_db`, defined by the `staging` target",
      urlEnv: "STORE_BACKENDS_PROBE_URL",
    },
  });
  answer.dialledWithoutADriver = await refusal(() =>
    stores.runStoreOp(store, "get", { key: "a" }, context("exec_d"), node()),
  );
}

process.stdout.write(JSON.stringify(answer));
