# cli

One static binary. Seven verbs act on a composition; five teach you about
compositions in general and take no spec at all.

```
agent-compose validate <path> [--target <name>] [--format human|json]
agent-compose plan <before> <after> [--format human|json]
agent-compose build <path> [--target <name>] [--out <dir>] [--check] [--format human|json]
agent-compose run <path> <flow> [--input k=v]... [--session <key>]
                                [--target <name>] [--out <dir>] [--format human|json]
agent-compose resume <path> <execution> [--target <name>] [--out <dir>]
                                        [--format human|json]
agent-compose serve <path> [--host <host>] [--port <port>]
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

The one exception is a write rather than a removal: a `tool.*` bound to
`module: ./src/tools/<name>.ts` gets that file **scaffolded once**, in the
project beside the entrypoint, when it is not there. `validate` and
`build --check` refuse a binding whose file is missing and name `build` as the
repair; `build` writes the stub and never writes or reads that file again. See
`agent-compose docs tools`.

`build --format json` writes `validate`'s two keys and a third:
`{"diagnostics": [ … ], "warnings": [ … ], "drift": [ … ]}`, where `drift` names
the files that do not match. All three are always present, so a clean build is
three empty arrays and a `--check` that found something is the same document
with the last one populated.

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
| `0` | clean: nothing was reported, or nothing but **warnings** was, or a `plan` was produced, or a document was printed |
| `1` | **errors** were reported, `build --check` found drift, a `run` produced no answer, a `resume` diverged from its journal, a `plan`'s spec did not resolve, or a discovery verb found something already there and would not replace it |
| `2` | the command could not run: bad usage, an unreadable entrypoint, an unwritable output directory, a missing or malformed environment variable, an uninstalled dependency set, no JavaScript runtime to launch, or a `resume` naming an execution the journal does not hold open |
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
| `NO_COLOR` | every verb that reports | any non-empty value turns styling off, whatever the stream is |

Provider credentials reach a run the same way: an `${ENV}` reference in the spec
is resolved from the process environment at start, never at build.

Normative source: `docs/plan.md`, `docs/trace.md`, `docs/durability.md`, `docs/grammar.md` §8.7, §14
