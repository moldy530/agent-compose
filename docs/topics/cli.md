# cli

One static binary. Five verbs act on a composition; five teach you about
compositions in general and take no spec at all.

```
agent-compose validate <path> [--target <name>] [--format human|json]
agent-compose plan <before> <after> [--format human|json]
agent-compose build <path> [--target <name>] [--out <dir>] [--check] [--format human|json]
agent-compose run <path> <flow> [--input k=v]... [--session <key>]
                                [--target <name>] [--out <dir>] [--format human|json]
agent-compose serve <path> [--host <host>] [--port <port>]
                           [--target <name>] [--out <dir>] [--format human|json]

agent-compose docs [<topic>]
agent-compose explain <code>
agent-compose schema
agent-compose init [<dir>]
agent-compose skill [--agent <name>] [--global]
```

## `validate`

Parses the entrypoint, follows its `imports:`, resolves every name, and runs
every static check. This is the loop — run it after every edit. Human output
goes to **stderr**; `--format json` writes `{"diagnostics": [ … ]}` to stdout,
one shape whatever the outcome.

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

Validates first and emits **only on a clean report** — a warning included.
Generated code is a build artifact of a valid composition; a project emitted
from a broken one would report the same problem later as a `tsc` error with no
span.

A clean report is then asked a second question `validate` never asks: whether
*this target* can express the composition. `pattern:` is RE2 and RE2 is not a
subset of ECMAScript, so a composition can be valid and have no TypeScript
project.

`--out` defaults to `<project>/build/<target>`. `--check` writes nothing and
reports whether the directory already matches the spec — that is the CI step,
and it exits `1` on drift. `build` replaces and removes only files carrying its
own generated-file header, so a directory holding somebody else's TypeScript is
refused rather than overwritten.

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

## `serve`

Same build, then starts the generated app for the project's `http` triggers:
start, resume, and status routes. `--port 0` takes one the operating system
picks. Its stdout is the app's readiness line.

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
| `0` | clean: nothing was reported, or a `plan` was produced, or a document was printed |
| `1` | diagnostics were reported, `build --check` found drift, a `run` produced no answer, a `plan`'s spec did not resolve, or a discovery verb found something already there and would not replace it |
| `2` | the command could not run: bad usage, an unreadable entrypoint, an unwritable output directory, a missing or malformed environment variable, an uninstalled dependency set, or no JavaScript runtime to launch |
| `3` | a `run` with nobody to ask stopped at a `human` pause |

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
| `AGENT_COMPOSE_INTERACTIVE` | `run` | `1` answers `human` pauses at the terminal even when stdin is not one; `0` forces the exit-`3` path. Any other value is a usage error refused before the run starts |
| `AGENT_COMPOSE_DATA_DIR` | the emitted project | moves the project's data directory — local stores, and the trace files under `.agent-compose/traces/` |
| `NO_COLOR` | every verb that reports | any non-empty value turns styling off, whatever the stream is |

Provider credentials reach a run the same way: an `${ENV}` reference in the spec
is resolved from the process environment at start, never at build.

Normative source: `docs/plan.md`, `docs/trace.md`, `docs/grammar.md` §8.7, §14
