// What `builtin.list` costs and what stops it, run against a generated
// project's own runtime (grammar 5.5, PRD resolved q31).
//
// A listing is the one built-in whose work happens **in this process**: `bash`
// is a child the runtime kills and the file tools touch one path each, but a
// `list` walks a directory a model named, matching every entry against a glob a
// model wrote. Both of those are the model's to size, so the two questions here
// are the ones a happy-path listing cannot answer:
//
//   * **What a pattern costs.** `**` is zero or more segments, and a matcher
//     that tried every split of the candidate at every `**` is exponential in
//     how many of them the glob holds — a one-line argument and a directory a
//     dozen deep, spending a core for as long as it takes anyone to notice. No
//     bound in the design stops it: grammar 5.5 gives `list` no `timeout:`
//     ("a file tool has no command to bound"), so the only thing between that
//     pattern and the machine is the matcher's own complexity.
//   * **What stops one.** The node's `timeout:` (grammar 9.2) and a cancelled
//     run abort `context.signal`, and `runActivity` stops *waiting* on an
//     activity either way — an activity that goes on working is invisible from
//     there, because the loser of that race is swallowed. So the walk has to
//     observe the signal itself, and the observation is only checkable from
//     inside.
//
// A third block asks the thing those two could quietly break: that the matcher
// still matches what it did. Its answers are the same before and after any
// change to how the search is run, which is what makes it a table rather than a
// pair of examples.
//
// Usage: node builtin-listing.mjs <generated project directory> <scratch dir>
// Output: { "deep": …, "wide": …, "shapes": … } as JSON.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch] = process.argv;
if (project === undefined || scratch === undefined) {
  throw new Error("usage: node builtin-listing.mjs <generated project directory> <scratch dir>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

/** A context with a signal nothing aborts — every call but the one below. */
function contextWith(signal) {
  return {
    execution: { id: "exec_builtin_listing", session_key: "" },
    signal,
    node: "probe",
  };
}

/** One `list` call, as the emitted `graph.ts` makes it. */
async function listing(root, args, signal) {
  return await runtime.runBuiltin({ tool: "list", root: [root] }, args, contextWith(signal));
}

function tree(name) {
  const root = path.join(path.resolve(scratch), name);
  fs.rmSync(root, { recursive: true, force: true });
  fs.mkdirSync(root, { recursive: true });
  return root;
}

// ---------------------------------------------------------------------------
// What a pattern costs.
//
// Sixteen `**` over a chain twelve deep. Sixteen rather than the thirty a model
// could as easily write, because a **regression here has to fail rather than
// hang the suite**: the search is synchronous, so no timer in this process can
// cut it short, and the number is chosen to keep the old cost at tens of
// seconds — measurably over the budget below and still a test that ends. The
// answer is asserted beside the clock, because a matcher that got fast by
// matching less would otherwise pass.
const deepRoot = tree("deep");
let at = deepRoot;
const depth = 12;
for (let level = 0; level < depth; level += 1) {
  at = path.join(at, `d${level}`);
  fs.mkdirSync(at);
}
fs.writeFileSync(path.join(at, "zzz.txt"), "the file at the bottom");
const stars = 16;
const startedDeep = Date.now();
const deepAnswer = await listing(
  deepRoot,
  { path: ".", glob: `${"**/".repeat(stars)}zzz.txt` },
  new AbortController().signal,
);
const deep = { stars, depth, elapsedMs: Date.now() - startedDeep, ...deepAnswer };

// ---------------------------------------------------------------------------
// What stops one.
//
// A tree wide enough that walking it is many turns of the event loop, listed
// twice: once under a signal nothing aborts, and once under one aborted on the
// first timer turn after the call. The first is what says the tree is really
// there to be walked — that the second stopped because it was stopped, and not
// because there was nothing to do — and the second carries the identity of what
// it raised, because a walk that let the abort become a `could not list …`
// would be reporting the node's deadline as a file-system fault.
const wideRoot = tree("wide");
for (let index = 0; index < 400; index += 1) {
  const directory = path.join(wideRoot, `dir${String(index).padStart(4, "0")}`);
  fs.mkdirSync(directory);
  for (let file = 0; file < 20; file += 1) {
    fs.writeFileSync(path.join(directory, `f${file}.txt`), "x");
  }
}
const startedWhole = Date.now();
const whole = await listing(wideRoot, { path: ".", glob: "**/*.txt" }, new AbortController().signal);
const wholeMs = Date.now() - startedWhole;

const controller = new AbortController();
const deadline = new Error("the deadline this probe stood in for");
setTimeout(() => controller.abort(deadline), 0);
const startedStopped = Date.now();
let wide;
try {
  const answered = await listing(wideRoot, { path: ".", glob: "**/*.txt" }, controller.signal);
  wide = { completed: true, entries: answered.entries.length };
} catch (error) {
  wide = {
    completed: false,
    // The abort's own reason, raised as it came: identity rather than wording,
    // so no restatement can pass this.
    raisedTheAbort: error === deadline,
    message: error instanceof Error ? error.message : String(error),
    stoppedMs: Date.now() - startedStopped,
  };
}
// What the same call answers when nothing stops it, beside how long that took.
wide.whole = whole.entries.length;
wide.wholeTruncated = whole.truncated;
wide.wholeMs = wholeMs;

// ---------------------------------------------------------------------------
// What the matcher matches.
//
// One tree, every shape a glob can take against it. `**` spans segments and `*`
// and `?` do not; `**` spans *zero* segments as readily as several, which is
// what makes a run of them mean exactly what one of them means; and a pattern
// that runs out while the candidate has not is a mismatch rather than a prefix
// match.
const shapeRoot = tree("shapes");
fs.mkdirSync(path.join(shapeRoot, "docs/deep/deeper"), { recursive: true });
for (const file of [
  "a.md",
  "docs/one.md",
  "docs/two.txt",
  "docs/deep/three.md",
  "docs/deep/deeper/four.md",
]) {
  fs.writeFileSync(path.join(shapeRoot, file), "x");
}
const shapes = {};
for (const glob of [
  "*.md",
  "?.md",
  "docs/*.md",
  "docs",
  "**/*.md",
  "**/**/**/*.md",
  "docs/**/*.md",
  "docs/**",
  "**/deep/**/*.md",
  "**",
]) {
  shapes[glob] = (await listing(shapeRoot, { path: ".", glob }, new AbortController().signal))
    .entries;
}

process.stdout.write(`${JSON.stringify({ deep, wide, shapes })}\n`);
