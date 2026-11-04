# cli

One static binary. Nine verbs act on a composition; five teach you about
compositions in general and take no spec at all.

```
agent-compose validate <path> [--target <name>] [--format human|json]
agent-compose plan <before> <after> [--format human|json]
agent-compose visualize <path> [--flow <addr>] [-o <path>] [--format html|json]
agent-compose build <path> [--target <name>] [--out <dir>] [--check] [--format human|json]
agent-compose run <path> <flow> [--input k=v]... [--session <key>]
                                [--target <name>] [--out <dir>] [--format human|json]
agent-compose resume <path> <execution> [--target <name>] [--out <dir>]
                                        [--format human|json]
agent-compose serve <path> [--host <host>] [--port <port>]
                           [--target <name>] [--out <dir>] [--format human|json]
agent-compose store <sub-verb> <path> <store> [operands] [--session <key> | --execution <id>]
                    [--target <name>] [--out <dir>] [--format human|json]
agent-compose worker --hub <url> --claim <name>... --token-env <VAR>
                     [--data-dir <dir>]

agent-compose docs [<topic>]
agent-compose explain <code>
agent-compose schema
agent-compose init [<dir>]
agent-compose skill [--agent <name>] [--global]
```

## `validate`

Parses the entrypoint, follows its `imports:`, resolves every name, and runs
every static check. This is the loop — run it after every edit. Human output
goes to **stderr**; `--format json` writes
`{"diagnostics": [ … ], "warnings": [ … ]}` to stdout, one shape whatever the
outcome — both keys always present, and a clean run is two empty arrays.

**The split is the verdict.** `diagnostics` holds what *refuses* the
composition and `warnings` holds what does not, so "is `diagnostics` empty" and
"did this exit `0`" are one question rather than two, and no consumer has to
filter on `severity` to answer it. A composition reported with nothing but
warnings is one this compiler **accepts**: the verdict line says it is valid and
counts them, the exit code is `0`, and `build`, `run` and `serve` go ahead. A CI
step that wants a warning to fail its build reads the `warnings` array and
decides for itself.

When it reported anything, the human output ends with one line pointing at
`explain`, once per run rather than once per diagnostic. So does every other
human report that carried a code — `build`, `run` and `serve` validate first and
print the same block, and a `plan` prints codes in its `validation` section and a
whole diagnostic block for a spec that does not resolve. The JSON is untouched: a
machine reader already has the code.

## `plan`

Reads **two** specs and reports what moved between them — components, topology,
interfaces, and what the validator now says that it did not. It is the answer to
"reviewing what changed in a graph requires reading router functions".

Either side may be a spec entrypoint or a project directory holding `main.yml`.
Everything the *validator* says is content rather than a refusal: "the after
spec introduces three errors" is a plan that was produced, and exits `0`. A spec
that does not **resolve** has no artifact to compare, and that is exit `1`.

`plan` takes no `--target`: the question it answers is what changed in the
composition. `docs/plan.md` is normative for the document it writes, and is what
a consumer pins `plan_version` on.

## `visualize`

Draws the composition's flows — one tab per flow, one node per node, one
labelled edge per routing decision an execution could take — as a **single HTML
file that fetches nothing**. No CDN, no webfonts, no network at all: it opens on
a machine that has none, and it is one file to attach to a pull request.

```sh
agent-compose visualize main.yml               # writes ./graph.html
agent-compose visualize main.yml -o docs/graph.html
agent-compose visualize main.yml --flow flow.triage
```

Click a node for its resolved configuration: the model it reaches and through
which provider, every tool on its wire — the ones an attached store synthesizes
included — its declared schemas, its input bindings, what it writes, its prompt,
and its `retry`/`timeout`/`on_error` **with the level each one resolved from**.
Drag a node to rearrange it; the arrangement is remembered by that browser and
nothing else, and *Reset layout* puts it back. A `flow:` node links to its own
flow's tab rather than expanding inline — the module boundary is real, so the
picture keeps it.

It validates first and emits only when clean, exactly as `build` does, printing
the same diagnostic block. Nothing it can refuse is a new failure class: a
`--flow` naming no flow, an unwritable path and `-o` beside `--format json` are
usage errors, and exit `2`.

`--format json` prints the **graph document** — the JSON the page renders — to
stdout instead, so `-o` is refused beside it. That document is a public surface
with a `graph_version` of its own, and `docs/graph.md` is normative for it.

The one `--format` here that names an artifact rather than a report: `visualize`
has no machine report to shape, because its answer *is* the document. Its
diagnostics are `validate`'s, on stderr, whichever format is asked for. It takes
no `--target` either, for the reason `plan` does not: the question a picture
answers is what this composition is, and the built-in `local` needs no deploy
file.

## `build`

Validates first, and an **error** refuses the emission: generated code is a
build artifact of a valid composition, and a project emitted from a broken one
would report the same problem later as a `tsc` error with no span.

A **warning** does not refuse it. The files are written, the exit code is `0`,
and the warnings are printed above a verdict that counts them:

```
warning: wrote 16 files to `build/local` (target `local`), with 1 warning
```

That is the whole of what the severity means, and `unknown-server-tool` is the
code that makes it load-bearing: a provider declaring a server tool this release
predates is a composition the compiler cannot fully check and does not refuse —
it builds, it runs, and the warning names what could not be verified.

An accepted composition is then asked a second question `validate` never asks:
whether *this target* can express it. `pattern:` is RE2 and RE2 is not a subset
of ECMAScript, so a composition can be valid and have no TypeScript project.

`--out` defaults to `<project>/build/<target>`. `--check` writes nothing and
reports whether the directory already matches the spec — that is the CI step,
and it exits `1` on drift.

**The emitted file list is the boundary.** `build` replaces exactly the files it
emits and `--check` compares exactly them; nothing else under the output
directory is written, removed, or reported. A directory holding none of the
compiler's own files is refused rather than overwritten — the generated-file
header is how it tells its work from yours — and a file the emitter does not
produce is nobody's business but yours, wherever it sits.

Not every target emits the same *set*, though: `package_registry:` in a deploy
file adds `bunfig.toml` and `.npmrc` (see `agent-compose docs targets`), and
dropping the key takes them away again. `build` removes nothing, so `--check`
reports one of the compiler's own files left at a name this target no longer
emits — `stale`, with a help line saying to delete it, because a rebuild will
not. A file of *yours* at that name is still yours and is not reported.

The one exception is a write rather than a removal: a `tool.*` bound to
`module: ./src/tools/<name>.ts` gets that file **scaffolded once**, in the
project beside the entrypoint, when it is not there. `validate` and
`build --check` refuse a binding whose file is missing and name `build` as the
repair; `build` writes the stub and never writes or reads that file again. It is
a write into your tree, so whichever verb makes it says so: `build` on its
verdict line, and `run` and `serve` — which build on the way — on a line of their
own before the launch. See `agent-compose docs tools`.

A build also **carries** each such file into the output directory, at the same
relative path — because the composition references it, and the output directory
is what an artifact is. Those copies are part of what `--check` compares and part
of the tree's content hash, so an edited implementation is a build to re-run and
a new artifact. How many a build carried is on its verdict line, beside the count
of what it emitted.

`build --format json` writes `validate`'s two keys and two more:
`{"diagnostics": [ … ], "warnings": [ … ], "drift": [ … ], "scaffolded": [ … ]}`,
where `drift` names the files that do not match and `scaffolded` names the
implementations this build wrote into your tree — `{"path": …, "tool": …}` each,
the same write the verdict line reports, because it is the one write a build
makes outside `--out` and it is made only once. All four are always present, so
a clean build is four empty arrays and a `--check` that found something is the
same document with `drift` populated.

## `run`

Validates, builds into the same directory `build` would, then launches the
emitted project's own command line. Every flow is runnable whether or not a
`manual` trigger names it.

**`run`'s stdout is the run's answer** — the flow's outputs as one JSON object,
or the whole record under `--format json` — so the build report goes to stderr.
`--input k=v` is repeatable and is validated against the flow's input schema at
run start; `--session <key>` supplies the session identity session-scoped stores
key off.

Before invoking the graph it checks two preconditions: every `${ENV}` reference
has a value, and the pinned dependency set is installed.

A run that reaches a `human` pause with nobody to ask exits **3**. With a
terminal — or `AGENT_COMPOSE_INTERACTIVE=1` — it renders the question, reads one
JSON value per line, validates it against the node's `output:`, and carries on
in the same process.

Every run is **journaled**, and under the human format the first line it writes
to stderr is the id `resume` takes:

```
execution: exec_9f1c8a3e-…
```

Under `--format json` the whole answer is still the one document on stdout, and
`execution_id` is a field of it on both of a run's ways out.

## `resume`

Carries on an execution this project's journal holds open — a `run` the machine
lost, or one that stopped at a `human` pause with nobody to ask.

```sh
agent-compose resume main.yml exec_9f1c8a3e-1b7d-4a20-9d61-1f0e8a2c4d55
```

Same build as `run`, then the graph is re-executed from its entry with every
recorded effect **consumed**: the model answers it got, the results its tools
produced, what its stores read, and what a person answered. Only the frontier —
the first effect the journal does not hold — reaches the network, so a resume
costs what is left of an execution rather than what it had already paid for.

It takes no `--input` and no `--session`: the invocation it replays is the one
the journal recorded. A resumed execution that reaches a wait nobody has
answered re-parks under its original wait id and prompts at the terminal exactly
as an interactive `run` does, so its exit codes are `run`'s.

A journal whose record no longer matches the composition — a changed prompt, a
renamed tool — fails the resume naming the divergent step rather than re-running
it. `agent-compose docs targets` has where the journal lives;
`docs/durability.md` is normative.

## `serve`

Same build, then starts the generated app for the project's `http` triggers:
start, resume, and status routes. `--port 0` takes one the operating system
picks. Its stdout is the app's readiness line.

On start it **recovers** every execution the journal holds open, before it
accepts a connection: an execution parked on a `human` pause re-parks under the
same wait id, so a `POST /executions/:id/resume` prepared against the process
that died still finds its wait. Triggers are not re-fired — recovery replays the
executions that exist.

It does not wait for those replays, so an answer can arrive while one is still on
its way back to its pause: that request is refused with `recovering: true` and
told to send it again, rather than told there is nothing waiting for it.

## `store`

Reads and writes one `kv` store through the target's own backend, with no flow
written. Same build as `run`, then the emitted project's own `store` verb opens
the store exactly as a flow of that build would — the compiler carries no
database driver, so it delegates — which is why `--target local` reads the
SQLite files under the build directory and a named target reaches the `postgres`
or `mysql` server its deploy file binds, with the target's environment variables,
as `run` would. It checks `run`'s two preconditions first, and the first is the
whole composition's: every `${ENV}` reference has a value — a provider's
credential as much as the store's address — and the pinned dependency set is
installed.

```sh
agent-compose store set main.yml store.notes greeting '{"text":"hello"}'
agent-compose store get main.yml store.notes greeting
agent-compose store set main.yml store.notes greeting '{"text":"hi"}' --if-version 1
agent-compose store delete main.yml store.notes greeting --if-version 2
agent-compose store list main.yml store.visits --session user-42 --prefix 2026- --values
agent-compose store count main.yml store.visits --session user-42
agent-compose store export main.yml store.notes > notes.jsonl
agent-compose store import main.yml store.notes --if-absent < notes.jsonl
agent-compose store log main.yml store.balances acct/a --limit 20
```

| sub-verb | flags | answers |
|---|---|---|
| `get <key>` | — | `{ value, found, version }`; exit `1` when the key holds nothing |
| `set <key> <value>` | `--if absent\|present` or `--if-version <n>`, `--ttl <duration>\|never` | `{ key, applied, version, deduped }`; exit `1` when the condition declined, naming the version the key holds; exit `2` when `value_schema` refuses the value |
| `delete <key>` | `--if-version <n>` | `{ deleted, applied, version, deduped }`; exit `1` when the condition declined |
| `list` | `--prefix <p>`, `--limit <n>` (1 to 1000, default 100), `--after <key>`, `--values` | one page, `{ keys, next, entries }`: `next` when more remain, `entries` under `--values` |
| `count` | `--prefix <p>` | `{ count }` |
| `export` | `--prefix <p>` | JSON Lines on stdout, one entry per line |
| `import` | `--if-absent`, `--file <path>` | `{ read, written, skipped, expired, present }`; exit `2`, writing nothing, when any line is refused |
| `log <key>` | `--limit <n>` (1 to 1000, default 100), `--after <seq>` | one page of the key's history, oldest first, `{ entries, next }`: `next` when more remain; exit `2` on a store that does not declare `history: true` |

**The partition is named by the scope the store declares.** `--session <key>`
for a `scope: session` store — the session identity itself, which is what a
run's `--session` supplies or what its trigger's `session_key:` turns that into
— `--execution <id>` for a `scope: execution` store, the id a run prints as
`execution: exec_…`, and neither for a `scope: global` one. The partition is
derived exactly as the runtime derives it, so the command reads what a flow in
that partition reads. A flag the scope does not take, a missing one, an address
naming no store, a `vector` or `blob` store, and a sub-verb that does not exist
are all usage errors and exit `2`; so is a store whose rows live only in the
memory of the process running each execution — every `scope: execution` store
under `--target local`, and any store bound to `provider: memory` — since this
command is another process. A target binding the store to a server reaches it.
For the same reason, under `--target local` a `store` command run beside a live
`run` or `serve` is a second process on the same SQLite files: the one-process
boundary the `targets` topic describes applies to it, and a write that contends
with a running node waits and then fails with `database is locked`.
A write to the partition of an execution that has already ended is a row
nothing will delete, because the run released that partition when it settled.

**A value is JSON**, on the command line or, as `-`, on standard input. A
`set`'s value and every `import` line are held to the store's `value_schema` by
the emitted Zod before anything is written; a refused value exits `2` naming the
issues — nothing was read or written, and the value is what is to fix, as with a
`run --input` that does not fit its type — so a supervisor retrying on `1` never
retries a write that cannot land. `--ttl` is `ttl:`'s spelling — `250ms`, `30s`,
`5m`, `24h`, or `never` to override a store's `default_ttl:` — and a `set` with
none takes the default, exactly as a node's would. A key whose time is up is
absent to `get`, `list`, `count`, `export` and every condition
(`agent-compose docs stores`).

**`export` and `import` move a partition as JSON Lines.** `export` writes one
`{"key": …, "value": …, "version": …, "expires_at": …}` per live key, in the
byte order a `list` pages in; `expires_at` is an ISO 8601 UTC instant, absent
for a key that does not expire. `import` reads the same shape from standard
input or `--file` and checks every line before it writes any, so a file with a
line the schema refuses writes nothing and names each such line. Each line is
then written at the partition's **next** version — the exported `version` is not
restored, since a version is the partition's sequence — and an `expires_at`
still ahead becomes the `ttl` that leaves the key expiring then, to within the
moment its write takes; a line already past it is skipped and counted as
`expired`; a line with none writes a key that does not expire, whatever the
store's default. `--if-absent` writes under `if: absent`, so a key the partition
already holds is kept and counted as `present`. A backend failure part-way says
which line it stopped at and how many were written before it.

**The output is a table** for a person, on stdout, with what went wrong on
stderr. `--format json` answers one JSON document on stdout instead: the op's
own answer row for `get`, `set`, `delete`, `list`, `count` and `log` — the row a
store-op node's output carries — and the summary for `import`. `export`'s lines
are its stdout under both formats, so its summary — `{"written": n}` under
`--format json` — goes to stderr, and so does its failure, as
`{"error": …, "written": n}` counting the lines already out, and so does the
report of a composition the compiler refuses: a failure is never written among
the lines a redirect is collecting for an import. A failure under
`--format json` is otherwise `{"error": …}` on stdout, with `lines` naming each
refused line of an import — a refused value is answered so too, though it exits
`2`, so the issues it names reach a script reading the document; any other usage
error is a sentence on stderr under either format, as every verb's is.

**A reader that leaves early is not a failure**, here as for every verb (see
[Exit codes](#exit-codes)). When the reader of a pipe goes before the answer
reaches it — `| head -1` with its line, a `jq` that failed — what is left of the
answer is dropped and the exit code is still the op's own: a `get` that found
its key and a `set` that applied end `0`, so a supervisor retrying on `1` never
re-issues a write that landed. An answer that was itself a failure, the
`{"error": …}` of `--format json`, is said on stderr instead. An `export` stops
at the page the pipe refused, reading nothing more from the store, and ends `0`
with a summary saying where — `{"written": n, "stopped": …}` under
`--format json`. A standard output that fails under a reader still there — a
full disk under `> entries.jsonl` — is said on stderr; for an `export` it is the
export failing, exit `1`, since that file does not hold the partition.

**An operator's write is not an effect, and is recorded in one place at
most.** It belongs to no execution, carries no idempotency key — so `deduped`
is always `false` — and writes no journal row, no ledger row and no trace
record. On a store that declares `history: true` it is recorded in its key's
history, as actor `cli` — a `set`, a `delete` and each line an `import` writes
alike — and that row is its only record anywhere. On any other store it is
recorded nowhere: a flow that reads the key afterwards finds the value, and
that is all.

**`log` reads a key's history**, on a store that declares `history: true`
(PRD resolved q74; `agent-compose docs stores`): one page, oldest first, of the
rows its writes left — each one's `seq`, `at` as an ISO 8601 UTC instant, `op`,
`version`, actor (`node`, `tool` or `cli`, with the execution and instance path
of a node or tool write) and the value before and after, `—` where the key held
nothing. Where more rows remain the table ends with the `next` cursor, which
`--after` takes to read the page after it; `--format json` prints the page as
one document, `{ entries, next }`, the row a `history` node's output carries —
not JSON Lines, since a page is one answer. A key with no rows exits `0`, said
on stderr as a `list` of no keys is; a store without `history: true`, and an
`--after` that is not a page's `next`, are usage errors and exit `2`. `export`
and `import` carry no history: an export is the entries, and an import's
writes are recorded as `cli` rows of their own.

## `worker`

The other half of a distributed deployment (`docs/distributed.md`). It takes no
spec: a worker holds no checkout and no YAML, it joins a hub over the URL you
give it and is served the compiled project.

```
agent-compose worker --hub https://hub.example --claim mac --token-env MESH_TOKEN
```

`--claim` is a placement name from the deploy file's `placements:`, repeated for
each name this machine offers. `--token-env` is the **variable** the join token
is read from — the same one that target's `hub.join_token:` names; the worker is
told the name by its invocation because the artifact that would have named it is
what the hub serves *after* the join. `--data-dir` is where materialised
artifacts are kept, keyed by content hash, so a redeployment and a rollback are
both a directory that is already there.

It runs until it is stopped, or until a join is **refused** — a credential that
does not verify, a release that does not match this hub's, a claim naming no
placement, a manifest this machine does not satisfy — which exits `2` echoing
what the hub said. Those are conditions another attempt would meet identically,
so restarting it is an operator's act.

Bun is required: a worker executes the artifact under it, which is a scoped
exception to the Node fallback a generated project otherwise runs under.

## `docs`

Bare, a topic index. With a topic, that topic's document — the curriculum an
agent reads to learn the grammar without a checkout of this repository. Unknown
topic exits `2` listing the valid ones.

## `explain`

An expanded account of one diagnostic code: what the check protects, a minimal
spec that triggers it, the fix, and where the grammar says so. Every code the
compiler can emit has one. Unknown code exits `2` with a suggestion.

## `schema`

The published JSON Schema, byte for byte, on stdout — for an editor's `$schema`
or a linter. `agent-compose schema > schema.json` is the point of the verb.

## `init`

Writes one commented `main.yml` that validates clean: an agent with an output
schema, a one-node flow, a manual trigger. The target directory must be **empty
or absent**; an occupied one exits `1` naming what it found.

## `skill`

Bare, prints the agent-agnostic skill document — what the CLI is and how the
loop goes — on stdout. `--agent <name>` **installs** it instead: `claude` writes
`.claude/skills/agent-compose/SKILL.md` and `codex` writes
`.codex/skills/agent-compose/SKILL.md`, under the current directory or under
`$HOME` with `--global`, refusing to overwrite a file that differs and no-opping
on one that matches. The bytes are the same either way — both agents load the
same portable `SKILL.md`, frontmatter and all — so the agent picks the root
directory and nothing else. An unknown `--agent` exits `2` listing the supported
ones.

`--global` names where an install writes, so it is **refused** rather than
ignored where nothing is written: bare `skill --global` exits `2`. An install
needs `HOME`, and exits `2` saying so when that is unset. A flag silently
dropped would leave you believing something had been installed under `$HOME`.

## Exit codes

| code | meaning |
|---|---|
| `0` | clean: nothing was reported, or nothing but **warnings** was, or a `plan` was produced, or a document was printed, or a `store` op found or applied |
| `1` | **errors** were reported, `build --check` found drift, a `run` produced no answer, a `resume` diverged from its journal, a `store` op found nothing, declined its condition or met a failing backend, a `store export` could not write its lines (a full disk, never a reader that left), a `plan`'s spec did not resolve, or a discovery verb found something already there and would not replace it |
| `2` | the command could not run: bad usage, an unreadable entrypoint, an unwritable output directory, a missing or malformed environment variable, an uninstalled dependency set, no JavaScript runtime to launch, a `resume` naming an execution the journal does not hold open, a `store` address, sub-verb or partition flag the composition does not take, a `store log` of a store that keeps no history, or a `store` value its `value_schema` refuses |
| `3` | a `run` or `resume` with nobody to ask stopped at a `human` pause |

The split between `1` and `2` is *the answer is no* versus *the command could
not be run at all*. A missing `imports:` entry is the composition's problem and
exits `1` with a diagnostic naming the file; an entrypoint that is not a
readable file is the command's own precondition and exits `2` with a plain
message, because there is no span to point at.

`3` is a code of its own rather than a shade of `1` because a run holding a
pause did everything it was asked to and is waiting on a person. A supervisor
that read it as `1` would re-run a graph whose effects have already happened.

A reader that stops reading — `| head`, `| less` then `q` — is never a failure:
what is left of the report is dropped and the exit code is still the verdict.

## Targets

`--target` selects `deploy/<name>.yml` and, with it, the answer to the
target-dependent rules: a store's `backend:` alias, an `event` trigger's
`source:`, and `detach: true`. Omitting it resolves the built-in `local`, which
requires no deploy file at all. See `agent-compose docs targets`.

## Environment variables

| variable | read by | meaning |
|---|---|---|
| `AGENT_COMPOSE_INTERACTIVE` | `run`, `resume` | `1` answers `human` pauses at the terminal even when stdin is not one; `0` forces the exit-`3` path. Any other value is a usage error refused before the run starts |
| `AGENT_COMPOSE_DATA_DIR` | the emitted project | moves the project's data directory — local stores, the execution journal, and the trace files under `.agent-compose/traces/` |
| `AGENT_COMPOSE_CALLBACK_RETRY` | `serve` | overrides the callback delivery retry schedule with a comma-separated list of durations (`0s,1s,2s`), for a test or a diagnostic run; unset, the schedule is `docs/durability.md` §3.7's five attempts across fifteen minutes. A value that is not such a list — an unspellable duration, or an empty list, which a variable *set to nothing* is — is a usage error refused before the app listens |
| `AGENT_COMPOSE_MESH_POLL_HOLD_MS` | `serve`, on a target with `placements:` | how long the hub holds a worker's poll before answering it empty, in whole milliseconds; unset, it is `docs/distributed.md` §2's 25 seconds. It has to stay shorter than the liveness window, and a pair that does not is refused before the app listens. Lengthening it needs nothing of the workers (§10.2): `agent-compose worker` waits a hold out and then waits longer, which is how it learns one it was not built expecting |
| `AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS` | `serve`, on a target with `placements:` | how long a worker session may go without a request before the hub declares it gone and supersedes the dispatch it was holding, in whole milliseconds; unset, it is §2's 90 seconds |
| `AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS` | the emitted project | whole milliseconds added to the clock a `kv` store judges expiry by — the process's on SQLite, the server's on `postgres`/`mysql` — for a test that moves past a `ttl:` without waiting for it (`agent-compose docs stores`); unset, it is `0`. A value that is not an integer fails the store op that reads it |
| `NO_COLOR` | every verb that reports | any non-empty value turns styling off, whatever the stream is |

Provider credentials reach a run the same way: an `${ENV}` reference in the spec
is resolved from the process environment at start, never at build.

Normative source: `docs/plan.md`, `docs/graph.md`, `docs/trace.md`, `docs/durability.md`, `docs/grammar.md` §8.7, §14
