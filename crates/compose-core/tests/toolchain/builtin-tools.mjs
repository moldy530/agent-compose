// What the two built-in tools do, run against a generated project's own runtime
// (grammar 5.5, 6.1, PRD resolved q54).
//
// The acceptance suite drives these through a model loop, which is where the
// wire, the trace and the bounces are decided. What it cannot reach is the
// inside of one call, and that is what this runner is for:
//
//   * **containment** — a `..` that climbs out, an absolute path, a symlink
//     pointing outside the workspace, a *dangling* symlink pointing outside it,
//     a symlinked **directory** written through at a path whose own parents
//     do not exist yet (which `create` would otherwise make, following the link
//     on the way), and a **sibling whose name extends the workspace's**, which
//     a prefix comparison that does not count the separator calls contained.
//     Each has to come back as a refusal and, more importantly, has to leave
//     everything outside the workspace untouched — nothing changed and nothing
//     new. A test that only read the message would pass against a runtime that
//     refused *and* wrote;
//   * **the empty file** — `file_text` is a defaulted parameter, so a `""` sent
//     deliberately and one left out reach the handler the same way. `create`
//     writes the empty file for both, because refusing them would leave
//     `.gitkeep` with no spelling that works; `insert` keeps its refusal for an
//     empty `new_str`, because a lone newline is that spelling there;
//   * **the view window** — `view_range`, the provider-defined editor's own
//     parameter and the way a file longer than one answer is read: the window
//     keeps the *file's* line numbers, `-1` and a last line past the end both
//     read to the end, and the four ways a range can be wrong come back as
//     refusals rather than as a view of the wrong lines;
//   * **the read bound** — a file larger than the runtime reads is *viewed*
//     from the front with the stop said, and *edited* not at all: an edit
//     rewrites what it read, so a truncated read would truncate the file;
//   * **the session** — `builtin.bash` keeps one shell per node activity, so a
//     `cd` in one call is still in effect in the next, and a second activity
//     starts in the workspace. Two contexts is the whole of that claim, and it
//     is invisible from outside the process;
//   * **the restart the model asks for** — the one way a session ends that the
//     *model* drives: alone it runs nothing and says so with no status, and a
//     `command` sent with it runs in the fresh session rather than being
//     dropped, which is a claim only the file system afterwards can settle;
//   * **the scrubbed environment** — a variable this process holds is *not* in
//     the child unless the binding declared it or opted into inheritance. Both
//     directions, because a runtime that passed everything and one that passed
//     nothing each satisfy half of it;
//   * **the command deadline** — a command that outruns its bound comes back as
//     a tool result rather than as a failure, and the session it killed is
//     replaced by the next call;
//   * **the default workspace** — one directory per execution under the
//     project's data directory, shared by two bindings that took the default,
//     removed when the execution settles and kept when it parks.
//
// Usage: node builtin-tools.mjs <generated project directory> <scratch dir>
// Output: one JSON object, read by `generated_code_gates.rs`.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch] = process.argv;
if (project === undefined || scratch === undefined) {
  throw new Error("usage: node builtin-tools.mjs <generated project directory> <scratch dir>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

/** A context with a signal nothing aborts, and no journal behind it. */
function contextWith(execution) {
  return {
    execution: { id: execution, session_key: "" },
    signal: new AbortController().signal,
    node: "probe",
  };
}

/** The call site an agent's loop hands a tool, one per call. */
function site() {
  return { path: [], ordinal: 0, dispatches: [], builtin: [] };
}

/** One built-in call, as the emitted `graph.ts` makes it. */
async function call(binding, args, context) {
  const where = site();
  try {
    const result = await runtime.runBuiltin(binding, args, context, where);
    return { result, program: where.builtin[0] };
  } catch (error) {
    return {
      refused: error instanceof runtime.ToolCallRefused,
      message: error instanceof Error ? error.message : String(error),
      program: where.builtin[0],
    };
  }
}

/** A workspace of this run's own, emptied. */
function workspace(name) {
  const root = path.join(path.resolve(scratch), name);
  fs.rmSync(root, { recursive: true, force: true });
  fs.mkdirSync(root, { recursive: true });
  return root;
}

const files = (root) => ({ tool: "files", workspace: [root], env: [] });
const shell = (root, extra = {}) => ({
  tool: "bash",
  workspace: [root],
  timeout: { millis: 5_000, written: "5s" },
  env: [],
  ...extra,
});

// ---------------------------------------------------------------------------
// Containment.
//
// Nine ways out of a workspace, and two directories outside it that none of
// them may reach. The file in each is written with a sentinel and read back at
// the end, and the first directory's own entries are listed: what is being
// tested is the file system's state, not the wording of a refusal.
//
// `underLinkedDirectory` and `farUnderLinkedDirectory` are the ones a
// parent-only check gets wrong. `create` makes the directories above what it
// writes, so `out/deep/nested.txt` through a symlinked `out` has *no* parent to
// resolve — and a check that stopped there would hand back the lexical path,
// find it inside the workspace, and then let `mkdir -p` follow the link.
//
// `prefixSibling` is the one every *other* case here is blind to, and the only
// one that lands outside the workspace by a path with nothing wrong with it.
// Each of the eight above resolves under `outside/`, which fails even a bare
// `target.startsWith(workspace)` — so none of them can tell a comparison that
// counts the separator from one that does not. This one is a sibling whose name
// *extends* the workspace's own (`…/contained-evil` beside `…/contained`, the
// shape `/srv/work-backup` has beside `/srv/work`): a prefix test alone calls it
// contained, and a `create` through it overwrites a file the composition never
// offered.
const outside = path.join(path.resolve(scratch), "outside");
fs.rmSync(outside, { recursive: true, force: true });
fs.mkdirSync(outside, { recursive: true });
const secret = path.join(outside, "secret.txt");
fs.writeFileSync(secret, "the file outside the workspace");

const contained = workspace("contained");
fs.symlinkSync(secret, path.join(contained, "link.txt"));
fs.symlinkSync(path.join(outside, "not-there-yet.txt"), path.join(contained, "dangling.txt"));
fs.symlinkSync(outside, path.join(contained, "out"), "dir");

// The sibling, made after the workspace so `workspace()` cannot empty it: its
// path is the workspace's path with more characters after it and no separator
// between.
const sibling = `${contained}-evil`;
fs.rmSync(sibling, { recursive: true, force: true });
fs.mkdirSync(sibling, { recursive: true });
const siblingSecret = path.join(sibling, "secret.txt");
fs.writeFileSync(siblingSecret, "the file in the sibling directory");

const escapes = {};
for (const [name, args] of [
  ["climb", { command: "create", path: "../outside/secret.txt", file_text: "clobbered" }],
  ["absolute", { command: "create", path: secret, file_text: "clobbered" }],
  ["symlink", { command: "create", path: "link.txt", file_text: "clobbered" }],
  ["dangling", { command: "create", path: "dangling.txt", file_text: "clobbered" }],
  ["read", { command: "view", path: "../outside/secret.txt" }],
  ["linkedDirectory", { command: "create", path: "out/secret.txt", file_text: "clobbered" }],
  ["underLinkedDirectory", { command: "create", path: "out/deep/nested.txt", file_text: "leaked" }],
  ["farUnderLinkedDirectory", { command: "create", path: "out/a/b/c/file.txt", file_text: "leaked" }],
  [
    "prefixSibling",
    { command: "create", path: `../${path.basename(sibling)}/secret.txt`, file_text: "clobbered" },
  ],
]) {
  escapes[name] = await call(files(contained), args, contextWith("exec_contained"));
}
const containment = {
  refused: Object.fromEntries(Object.entries(escapes).map(([name, answer]) => [name, answer.refused === true])),
  // The whole point: nothing outside the workspace moved, and nothing new is
  // there either.
  outsideUnchanged: fs.readFileSync(secret, "utf8") === "the file outside the workspace",
  outsideNotCreated: !fs.existsSync(path.join(outside, "not-there-yet.txt")),
  outsideEntries: fs.readdirSync(outside).sort(),
  // …and the sibling the prefix test would have called contained still holds
  // what it held.
  siblingUnchanged: fs.readFileSync(siblingSecret, "utf8") === "the file in the sibling directory",
  siblingEntries: fs.readdirSync(sibling).sort(),
  // …and the record of a refused call still says what the model asked for
  // (`docs/trace.md` §7.4).
  program: escapes.climb.program,
  message: escapes.climb.message,
  throughLinkMessage: escapes.underLinkedDirectory.message,
};

// ---------------------------------------------------------------------------
// The file tool, end to end.
const editing = workspace("editing");
const editContext = contextWith("exec_editing");
const created = await call(
  files(editing),
  { command: "create", path: "notes/todo.md", file_text: "one\ntwo\nthree\n" },
  editContext,
);
const viewed = await call(files(editing), { command: "view", path: "notes/todo.md" }, editContext);
const replaced = await call(
  files(editing),
  { command: "str_replace", path: "notes/todo.md", old_str: "two", new_str: "TWO" },
  editContext,
);
const inserted = await call(
  files(editing),
  { command: "insert", path: "notes/todo.md", insert_line: 1, new_str: "one and a half" },
  editContext,
);
const listed = await call(files(editing), { command: "view", path: "notes" }, editContext);
// The window a `view` can be asked for — the provider-defined editor's own
// `view_range`, and the way a long file is read at all. On a file of its own,
// with a **blank line inside the window**, because a window re-split out of its
// own joined text would lose a blank line that landed last.
await call(
  files(editing),
  { command: "create", path: "ranged.txt", file_text: "a\nb\n\nd\ne\n" },
  editContext,
);
const ranged = (view_range) =>
  call(files(editing), { command: "view", path: "ranged.txt", view_range }, editContext);
const middle = await ranged([2, 4]);
const toTheEnd = await ranged([4, -1]);
const clamped = await ranged([4, 99]);
const emptyRange = await ranged([]);
const pastTheEnd = await ranged([9, 10]);
const backwards = await ranged([4, 2]);
const oneNumber = await ranged([4]);
const fromZero = await ranged([0, 2]);
const onDirectory = await call(
  files(editing),
  { command: "view", path: "notes", view_range: [1, 2] },
  editContext,
);
const viewWindow = {
  middle: middle.result,
  toTheEnd: toTheEnd.result,
  clamped: clamped.result,
  emptyRange: emptyRange.result,
  pastTheEndRefused: pastTheEnd.refused === true,
  pastTheEndMessage: pastTheEnd.message,
  backwardsRefused: backwards.refused === true,
  backwardsMessage: backwards.message,
  oneNumberRefused: oneNumber.refused === true,
  oneNumberMessage: oneNumber.message,
  fromZeroRefused: fromZero.refused === true,
  fromZeroMessage: fromZero.message,
  onDirectoryRefused: onDirectory.refused === true,
  onDirectoryMessage: onDirectory.message,
  // What the trace carries of a windowed read: the operation and the path, and
  // nothing about the window — `docs/trace.md` §7.4's fields, unchanged.
  program: middle.program,
};
const ambiguous = await call(
  files(editing),
  { command: "create", path: "twice.txt", file_text: "same\nsame\n" },
  editContext,
).then(() =>
  call(
    files(editing),
    { command: "str_replace", path: "twice.txt", old_str: "same", new_str: "other" },
    editContext,
  ),
);
const missing = await call(
  files(editing),
  { command: "str_replace", path: "notes/todo.md", old_str: "nowhere", new_str: "x" },
  editContext,
);
const absent = await call(files(editing), { command: "view", path: "nothing.txt" }, editContext);
// The file with nothing in it, both ways a model can ask for one. `file_text`
// carries `default: ""` so the operations that never read it are callable
// without it, and the parse fills that default in — so `""` sent deliberately
// and `file_text` left out arrive at the handler identically. A `create` that
// refused the pair would leave `.gitkeep` with no spelling that works, so both
// write the empty file and the answer says `wrote 0 bytes`.
const emptied = await call(
  files(editing),
  { command: "create", path: "keep/.gitkeep", file_text: "" },
  editContext,
);
const omitted = await call(files(editing), { command: "create", path: "keep/.keep" }, editContext);
// …and the operation that keeps its refusal, because there the empty argument
// has a spelling that works: a lone newline inserts the blank line. On a file of
// its own, so what the four operations above left on disk stays theirs.
await call(
  files(editing),
  { command: "create", path: "keep/blank.txt", file_text: "a\n" },
  editContext,
);
const blankLine = await call(
  files(editing),
  { command: "insert", path: "keep/blank.txt", insert_line: 0, new_str: "" },
  editContext,
);
await call(
  files(editing),
  { command: "insert", path: "keep/blank.txt", insert_line: 0, new_str: "\n" },
  editContext,
);
const emptyOf = (name) => {
  const at = path.join(editing, "keep", name);
  return fs.existsSync(at) && fs.readFileSync(at, "utf8") === "";
};
const editingTool = {
  created: created.result,
  createdProgram: created.program,
  viewed: viewed.result,
  replaced: replaced.result,
  replacedProgram: replaced.program,
  inserted: inserted.result,
  insertedProgram: inserted.program,
  listed: listed.result,
  ambiguousRefused: ambiguous.refused === true,
  ambiguousMessage: ambiguous.message,
  missingRefused: missing.refused === true,
  absentRefused: absent.refused === true,
  // What is on disk when the four operations have run.
  onDisk: fs.readFileSync(path.join(editing, "notes/todo.md"), "utf8"),
  // The empty file, asked for both ways, and the empty argument that keeps its
  // refusal because a lone newline says the same thing.
  emptyCreated: emptied.result,
  emptyProgram: emptied.program,
  emptyOnDisk: emptyOf(".gitkeep"),
  omittedCreated: omitted.result,
  omittedOnDisk: emptyOf(".keep"),
  blankLineRefused: blankLine.refused === true,
  blankLineMessage: blankLine.message,
  blankLineOnDisk: fs.readFileSync(path.join(editing, "keep/blank.txt"), "utf8"),
};

// ---------------------------------------------------------------------------
// A file larger than this runtime reads.
//
// The path is the model's, so the size of what a `files` call pulls into this
// process is the model's too unless the runtime bounds it. `view` answers with
// the front and says it stopped; an edit is refused outright, because
// `str_replace` writes back what it read and a truncated read would truncate the
// file — which the size on disk afterwards is what actually proves.
const heavyRoot = workspace("heavy");
const heavyContext = contextWith("exec_heavy");
const heavyPath = path.join(heavyRoot, "big.txt");
{
  const block = Buffer.from(`${"x".repeat(99)}\n`.repeat(1_000));
  const handle = fs.openSync(heavyPath, "w");
  for (let written = 0; written < 45; written += 1) fs.writeSync(handle, block);
  fs.closeSync(handle);
}
const heavyBytes = fs.statSync(heavyPath).size;
const heavyViewed = await call(files(heavyRoot), { command: "view", path: "big.txt" }, heavyContext);
// …and the reason `view_range` exists: a whole-file view is cut at the answer
// bound *from line 1*, so the far end of a long file is reachable only through a
// window. This one asks for it directly.
const heavyTail = await call(
  files(heavyRoot),
  { command: "view", path: "big.txt", view_range: [39_990, -1] },
  heavyContext,
);
const heavyEdited = await call(
  files(heavyRoot),
  { command: "str_replace", path: "big.txt", old_str: "xxxxx", new_str: "yyyyy" },
  heavyContext,
);
const readBound = {
  bytesOnDisk: heavyBytes,
  viewedLength: (heavyViewed.result?.content ?? "").length,
  saidItStopped: (heavyViewed.result?.content ?? "").includes("were read"),
  // The window into the same file: it starts where it was asked to, which is far
  // past where the whole-file view was cut, and still says the read stopped.
  tailFirstNumber: Number.parseInt((heavyTail.result?.content ?? "").trimStart(), 10),
  tailLength: (heavyTail.result?.content ?? "").length,
  tailSaidItStopped: (heavyTail.result?.content ?? "").includes("were read"),
  editRefused: heavyEdited.refused === true,
  editMessage: heavyEdited.message,
  stillWholeOnDisk: fs.statSync(heavyPath).size === heavyBytes,
};

// ---------------------------------------------------------------------------
// The shell session.
//
// One context is one node activity: the `cd` in the first call is still in
// effect in the second, and the variable set in the second is still set in the
// third. A *different* context is a different activity and starts over.
const shelled = workspace("shelled");
fs.mkdirSync(path.join(shelled, "inner"));
const first = contextWith("exec_shell");
const moved = await call(shell(shelled), { command: "cd inner && pwd" }, first);
const stayed = await call(shell(shelled), { command: "pwd" }, first);
const remembered = await call(shell(shelled), { command: "kept=42; echo set" }, first);
const recalled = await call(shell(shelled), { command: "echo \"[$kept]\"" }, first);
const second = contextWith("exec_shell");
const fresh = await call(shell(shelled), { command: "pwd; echo \"[$kept]\"" }, second);
// `(exit 3)` rather than `exit 3`: the command is typed into a shell that
// outlives it, so a bare `exit` would end the *session* — which is a thing this
// runtime handles (the next call opens a fresh one) and not the thing being
// asked here.
const failed = await call(shell(shelled), { command: "echo out; echo err >&2; (exit 3)" }, first);
const quit = await call(shell(shelled), { command: "exit 7" }, first);
const session = {
  moved: moved.result,
  stayedInInner: (stayed.result?.stdout ?? "").trim().endsWith("/inner"),
  remembered: (recalled.result?.stdout ?? "").trim(),
  freshActivity: (fresh.result?.stdout ?? "").trim(),
  // A nonzero exit is an answer, not a node failure.
  failed: failed.result,
  failedProgram: failed.program,
  // …and a model that ends its own shell is told so rather than left to wonder
  // why its state is gone.
  quitNotice: quit.result?.notice ?? null,
};
runtime.endShellSessions(first);
runtime.endShellSessions(second);

// ---------------------------------------------------------------------------
// The restart the *model* asks for.
//
// The other way a session ends, and the only one the model drives: `restart` is
// a parameter of the provider-defined tool, so it is a call this runtime has to
// answer whatever else it is doing. Three claims, and the third is the one a
// runtime gets wrong quietly:
//
//   * a restart **alone** runs nothing — so it comes back with a notice and no
//     `exit_code`, because a status invented for a call that ran nothing would
//     be copied into the trace as a command that completed (`docs/trace.md`
//     §7.4);
//   * it really ends the session — the `cd` before it is gone after it;
//   * a `command` sent **with** a restart *runs*, in the fresh session. A
//     runtime that dropped it would answer the model `exit_code: 0` for a write
//     that never happened and record that command in the trace as having run on
//     this host, which is worse than recording nothing. So what is asserted is
//     the file system afterwards, not the wording of the answer.
const restarted = workspace("restarted");
fs.mkdirSync(path.join(restarted, "inner"));
const restarting = contextWith("exec_restart");
await call(shell(restarted), { command: "cd inner; kept=42" }, restarting);
const restartAlone = await call(shell(restarted), { restart: true }, restarting);
const afterRestart = await call(shell(restarted), { command: 'pwd; echo "[${kept-unset}]"' }, restarting);
await call(shell(restarted), { command: "cd inner; kept=42" }, restarting);
const withCommand = await call(
  shell(restarted),
  {
    command: 'printf "ran\\n" > made-by-restart.txt; pwd; echo "[${kept-unset}]"; (exit 5)',
    restart: true,
  },
  restarting,
);
const restart = {
  alone: restartAlone.result,
  aloneProgram: restartAlone.program,
  // The state the restart threw away: back in the workspace, with the variable
  // it was holding gone.
  after: (afterRestart.result?.stdout ?? "").trim(),
  withCommand: withCommand.result,
  withCommandProgram: withCommand.program,
  // The proof the command was not discarded — on disk rather than in the answer.
  commandRan: fs.existsSync(path.join(restarted, "made-by-restart.txt")),
  // …and that it ran in the *fresh* session: the workspace again, and no `kept`.
  whereItRan: (withCommand.result?.stdout ?? "").trim(),
};
runtime.endShellSessions(restarting);

// ---------------------------------------------------------------------------
// A command that prints more than this runtime holds.
//
// The bound on a *result* is applied when a call settles; this is the one that
// has to be applied as the bytes arrive, because the program is the model's and
// `cat` of a large file is a command it can write. The command below prints a
// few megabytes, which is over the buffer bound and far over the answer's — so
// what is asserted is that the call still settles with its status, that what
// comes back is bounded, and that both ends of the output are in it.
const loudRoot = workspace("loud");
const loudContext = contextWith("exec_loud");
const loud = await call(
  shell(loudRoot, { timeout: { millis: 60_000, written: "60s" } }),
  { command: "printf 'the first line\n'; for i in $(seq 1 120000); do printf 'noise %s\n' \"$i\"; done; printf 'the last line\n'" },
  loudContext,
);
const bounded = {
  exitCode: loud.result?.exit_code ?? null,
  length: (loud.result?.stdout ?? "").length,
  keptTheHead: (loud.result?.stdout ?? "").startsWith("the first line"),
  keptTheTail: (loud.result?.stdout ?? "").includes("the last line"),
  saidWhatItDropped: (loud.result?.stdout ?? "").includes("dropped"),
  // …and the session is still usable, which is what says the trim did not eat
  // the marker that closes a command.
  after: (await call(shell(loudRoot), { command: "echo still usable" }, loudContext)).result,
};
runtime.endShellSessions(loudContext);

// ---------------------------------------------------------------------------
// The scrubbed environment, both ways round.
process.env["INHERITED_SECRET"] = "the value this process holds";
const scrubbedRoot = workspace("scrubbed");
const scrubbedContext = contextWith("exec_env");
const scrubbed = await call(
  shell(scrubbedRoot),
  { command: 'echo "[${INHERITED_SECRET-unset}][${DECLARED-unset}]"' },
  scrubbedContext,
);
const declared = await call(
  shell(scrubbedRoot, { env: [{ name: "DECLARED", value: ["a value the binding wrote"] }] }),
  { command: 'echo "[${INHERITED_SECRET-unset}][${DECLARED-unset}]"' },
  scrubbedContext,
);
const inherited = await call(
  shell(scrubbedRoot, { inheritEnv: true }),
  { command: 'echo "[${INHERITED_SECRET-unset}][${DECLARED-unset}]"' },
  scrubbedContext,
);
const environment = {
  scrubbed: (scrubbed.result?.stdout ?? "").trim(),
  declared: (declared.result?.stdout ?? "").trim(),
  inherited: (inherited.result?.stdout ?? "").trim(),
};
runtime.endShellSessions(scrubbedContext);

// ---------------------------------------------------------------------------
// The command deadline.
//
// A bound short enough to reach, a command far longer than it, and the call
// after it: the deadline answers the model rather than failing the node, and the
// session it took with it is replaced.
const impatientRoot = workspace("impatient");
const impatientContext = contextWith("exec_timeout");
const impatient = { tool: "bash", workspace: [impatientRoot], timeout: { millis: 300, written: "300ms" }, env: [] };
const startedTimeout = Date.now();
const outran = await call(impatient, { command: "sleep 30" }, impatientContext);
const timeout = {
  elapsedMs: Date.now() - startedTimeout,
  result: outran.result,
  program: outran.program,
  after: (await call(impatient, { command: "echo still here" }, impatientContext)).result,
};
runtime.endShellSessions(impatientContext);

// ---------------------------------------------------------------------------
// The default workspace.
//
// Two bindings that wrote none, in one execution: one directory, shared, under
// the project's data directory. Removed when the execution settles; kept when it
// parks, because the generation that resumes it reads what this one wrote.
const shared = { tool: "files", workspace: [], env: [] };
const alsoShared = { tool: "bash", workspace: [], timeout: { millis: 5_000, written: "5s" }, env: [] };
const defaulted = contextWith("exec_default_workspace");
await call(shared, { command: "create", path: "made.txt", file_text: "by the file tool\n" }, defaulted);
const sameDirectory = await call(alsoShared, { command: "cat made.txt; pwd" }, defaulted);
runtime.endShellSessions(defaulted);
const madeAt = path.join(project, ".agent-compose", "workspaces", "exec_default_workspace");
const workspaces = {
  underTheDataDirectory: fs.existsSync(madeAt),
  sharedByBothTools: (sameDirectory.result?.stdout ?? "").includes("by the file tool"),
};
await runtime.releaseWorkspaces("exec_default_workspace", true);
workspaces.keptWhenParked = fs.existsSync(madeAt);
// The parked release dropped this process's handle on it, so the settled one is
// asked over a workspace it has to find again — which is the case a resumed run
// makes, and the one a memoized path would answer wrongly.
const settling = contextWith("exec_default_workspace");
await call(shared, { command: "view", path: "made.txt" }, settling);
await runtime.releaseWorkspaces("exec_default_workspace", false);
workspaces.goneWhenSettled = !fs.existsSync(madeAt);

process.stdout.write(
  `${JSON.stringify({ containment, editingTool, viewWindow, readBound, session, restart, bounded, environment, timeout, workspaces })}\n`,
);
