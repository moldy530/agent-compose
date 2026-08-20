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

If that fails, the binary is not on `PATH`. Releases publish prebuilt binaries
for Linux and macOS; there is nothing else to install, and no Rust toolchain is
needed. Node or Bun is needed only to *run* a compiled project.

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
   guessing.
5. **Read a topic** when a construct is unfamiliar. `agent-compose docs` lists
   them; `agent-compose docs <topic>` prints one.
6. **Review the change.** `agent-compose plan <before> <after>` reports what
   moved in the components, the topology, the interfaces, and the validation.
   Use it before and after a non-trivial edit, and put its output in a PR
   description rather than asking a reviewer to diff router logic.
7. **Compile and run.** `agent-compose build` emits the TypeScript project;
   `agent-compose run main.yml flow.<name> --input k=v` builds and runs one
   flow; `agent-compose serve main.yml` starts the app for `http` triggers.

## Verbs

| verb | what it does |
|---|---|
| `validate <path>` | parse, resolve imports, run every static check. **The loop.** |
| `plan <before> <after>` | diff two specs: components, topology, interfaces, validation |
| `build <path>` | emit the TypeScript project; `--check` reports drift instead of writing |
| `run <path> <flow>` | build, then run one flow; `--input k=v`, `--session <key>` |
| `serve <path>` | build, then serve the project's `http` triggers |
| `docs [<topic>]` | the topic index, or one topic |
| `explain <code>` | the expanded account of one diagnostic code |
| `schema` | the published JSON Schema, for an editor's `$schema` |
| `init [<dir>]` | scaffold a project that validates |
| `skill [--agent <name>]` | print this document, or install it |

Add `--format json` to any verb that reports — `validate`, `plan`, `build`,
`run`, `serve` — when a script is reading the output. Add `--target <name>` to
the four verbs that resolve a composition against an environment — `validate`,
`build`, `run`, `serve` — to select a `deploy/<name>.yml`; the built-in `local`
target needs no deploy file and is what those four resolve when none is named.
`plan` takes no `--target`: it answers what changed in the composition.

## Exit codes

| code | meaning |
|---|---|
| `0` | clean, or a plan was produced, or a document was printed |
| `1` | the answer is no: diagnostics, drift, a run with no answer, an occupied directory |
| `2` | the command could not run at all: bad usage, unreadable entrypoint, missing dependency |
| `3` | a `run` stopped at a `human` pause and had nobody to ask |

Branch on these rather than on message text: codes and exit codes are stable,
messages are improved between releases.

## Answering a pause from a script

A flow with a `human` node pauses. A `run` whose standard input is a terminal
prompts for the answer; a script sets `AGENT_COMPOSE_INTERACTIVE=1` and writes
**one JSON value per line** on stdin, in the order the prompts arrive. An answer
the node's output schema refuses re-prompts and does not consume the wait.
`AGENT_COMPOSE_INTERACTIVE=0` forces the exit-`3` path, which is what a
supervisor that must not block should set.

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
- **Never put a credential in a spec.** Secret-bearing fields take `${NAME}`
  environment references only, and the compiler refuses a literal.
