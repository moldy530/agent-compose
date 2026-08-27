# agent-compose

agent-compose is a declarative YAML DSL for agent graphs that compiles to a
LangGraph TypeScript project. You author components — models, agents, tools,
flows — and a single static binary checks that the graph they form can actually
run, before any of it does.

This document is about the **loop**, not the grammar. The binary teaches its own
grammar: `agent-compose docs` is the curriculum, and it is always in step with
the compiler that ships beside it. Never quote grammar rules from memory —
read the topic.

## Install and verify

```
agent-compose --version
```

If that fails, the binary is not on `PATH`; the project's README has the install
script and the tarball route (https://github.com/moldy530/agent-compose#install).
It is a single static executable and is the whole of the compiler: nothing else
to install, and no Rust toolchain needed. Node or Bun is needed only to *run* a
compiled project.

## The loop

1. **Start.** `agent-compose init <dir>` writes one commented `main.yml` that
   already validates. Read it — it is the shortest tour of the language there
   is.
2. **Edit**, one construct at a time.
3. **Validate, early and often.** `agent-compose validate main.yml` is designed
   to run in milliseconds after every edit. Do not batch edits and validate
   once; validate after each one, so a diagnostic points at the change that
   caused it.
4. **Explain what it reported.** Every diagnostic carries a stable code.
   `agent-compose explain <code>` gives the expanded account: what the check
   protects, a minimal spec that triggers it, and the fix. Do this before
   guessing. Read its **severity** first: an error refuses the composition, a
   warning does not — a spec reported with nothing but warnings is valid, exits
   `0`, and builds and runs. Under `--format json` the two are separate keys,
   `diagnostics` and `warnings`.
5. **Read a topic** when a construct is unfamiliar. `agent-compose docs` lists
   them; `agent-compose docs <topic>` prints one.
6. **Review the change.** `agent-compose plan <before> <after>` reports what
   moved in the components, the topology, the interfaces, and the validation.
   Use it before and after a non-trivial edit, and put its output in a PR
   description rather than asking a reviewer to diff router logic.
7. **Compile and run.** `agent-compose build main.yml` emits the TypeScript
   project; `agent-compose run main.yml flow.<name> --input k=v` builds and
   runs one flow; `agent-compose serve main.yml` starts the app for `http`
   triggers.

## Verbs

| verb | what it does |
|---|---|
| `validate <path>` | parse, resolve imports, run every static check. **The loop.** |
| `plan <before> <after>` | diff two specs: components, topology, interfaces, validation |
| `build <path>` | emit the TypeScript project; `--check` reports drift instead of writing |
| `run <path> <flow>` | build, then run one flow; `--input k=v`, `--session <key>` |
| `resume <path> <execution>` | build, then carry on a journaled execution — replaying its recorded effects rather than re-issuing them |
| `serve <path>` | build, then serve the project's `http` triggers |
| `docs [<topic>]` | the topic index, or one topic |
| `explain <code>` | the expanded account of one diagnostic code |
| `schema` | the published JSON Schema, for an editor's `$schema` |
| `init [<dir>]` | scaffold a project that validates |
| `skill [--agent <name>] [--global]` | print this document, or install it; `--global` installs under `$HOME` |

Add `--format json` to any verb that reports — `validate`, `plan`, `build`,
`run`, `resume`, `serve` — when a script is reading the output. Add
`--target <name>` to the five verbs that resolve a composition against an
environment — `validate`, `build`, `run`, `resume`, `serve` — to select a
`deploy/<name>.yml`; the built-in `local` target needs no deploy file and is
what those five resolve when none is named. `plan` takes no `--target`: it
answers what changed in the composition.

## Exit codes

| code | meaning |
|---|---|
| `0` | clean, or **warnings** only, or a plan was produced, or a document was printed |
| `1` | the answer is no: **errors**, drift, a run with no answer, a resume that diverged from its journal, an occupied directory |
| `2` | the command could not run at all: bad usage, unreadable entrypoint, missing dependency, an execution id the journal does not hold open |
| `3` | a `run` or `resume` stopped at a `human` pause and had nobody to ask |

Branch on these rather than on message text: codes and exit codes are stable,
messages are improved between releases.

## Answering a pause from a script

A flow with a `human` node pauses. A `run` whose standard input is a terminal
prompts for the answer; a script sets `AGENT_COMPOSE_INTERACTIVE=1` and writes
**one JSON value per line** on stdin, in the order the prompts arrive.
`AGENT_COMPOSE_INTERACTIVE=0` forces the exit-`3` path, which is what a
supervisor that must not block should set. What a pause asks for, and what
becomes of an answer it will not take, is `agent-compose docs human`.

## Resuming a run the machine lost

Every run is journaled, and its first line on stderr is the id to resume it
with:

```
execution: exec_9f1c8a3e-1b7d-4a20-9d61-1f0e8a2c4d55
```

`agent-compose resume main.yml <execution-id>` re-runs that execution's graph
with every recorded effect **consumed** — the model answers it got, the results
its tools produced, what its stores read, what a person answered — and only the
work it had not yet done reaches the network. It takes no `--input` and no
`--session`: the invocation it replays is the one the journal recorded. `serve`
needs no such command; it recovers every open execution on start. See
`agent-compose docs targets`.

## The topics

Run `agent-compose docs` for the list with one line each. They are the
curriculum, in reading order: `getting-started`, `schemas`, `cel`, `agents`,
`tools`, `flows`, `routing`, `cycles`, `maps`, `human`, `state`, `policies`,
`stores`, `models`, `triggers`, `targets`, `trace`, `cli`.

Each topic is example-led — a complete runnable spec first, then the rules — and
names the sections of the full grammar it derives from, for when you need the
normative text.

## Working habits that pay off here

- **Let the validator do the checking.** It knows about routing exhaustiveness,
  cycle termination, fan-out bounds, schema compatibility across every edge, and
  a few dozen other things. Write the spec you mean and read what it says.
- **Read the whole diagnostic.** It carries a span, usually a second labelled
  span, and a `help:` line naming the fix. The help line is written to be acted
  on.
- **Prefer a topic to a guess.** Constructs here have precise rules that are
  cheap to look up and expensive to get subtly wrong. Before guessing at an
  edge, at a channel write, or at what a fan-out may do, read
  `agent-compose docs routing`, `agent-compose docs state`, and
  `agent-compose docs maps`.
- **Do not hand-edit generated code.** It is a build artifact; `build --check`
  in CI is what keeps the spec the source of truth.
- **Never put a credential in a spec.** Where a secret may come from is a rule
  the compiler enforces rather than a convention you can keep by being careful;
  `agent-compose docs models` is where it is written.
