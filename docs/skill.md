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
   `agent-compose visualize main.yml` writes a single self-contained HTML page
   of the graph — every node, every edge an execution could take, and each
   node's resolved configuration — which is the other half of that review, and
   the fastest way to see what a flow you did not write actually does.
7. **Compile and run.** `agent-compose build main.yml` emits the TypeScript
   project; `agent-compose run main.yml flow.<name> --input k=v` builds and
   runs one flow; `agent-compose serve main.yml` starts the app: its `http`
   routes, its `schedule` ticks, and the programs its `event` triggers are fed
   by.

## Verbs

| verb | what it does |
|---|---|
| `validate <path>` | parse, resolve imports, run every static check. **The loop.** |
| `plan <before> <after>` | diff two specs: components, topology, interfaces, validation |
| `visualize <path>` | draw the flows as one self-contained HTML page; `--flow`, `-o`, `--format html\|json` |
| `build <path>` | emit the TypeScript project; `--check` reports drift instead of writing |
| `run <path> <flow>` | build, then run one flow; `--input k=v`, `--session <key>` |
| `resume <path> <execution>` | build, then carry on a journaled execution — replaying its recorded effects rather than re-issuing them |
| `serve <path>` | build, then serve the project's triggers: `http` routes, `schedule` ticks, and `kind: command` event sources |
| `cancel <path> <execution>` | stop an execution a `serve` is running, and every open child execution it started; `--url <base>` (or the target's `hub.public_url`), `--header 'Name: value'` for the starting trigger's `auth:` |
| `store <sub-verb> <path> <store>` | build, then read or write one `kv` store through the target's own backend with no flow: `get`, `set`, `delete`, `list`, `count`, `export`, `import`; `--session <key>` or `--execution <id>` names the partition |
| `worker --hub <url> --claim <name> --token-env <VAR>` | join a hub and run the nodes placed on the names it claims; takes no spec |
| `docs [<topic>]` | the topic index, or one topic |
| `explain <code>` | the expanded account of one diagnostic code |
| `schema` | the published JSON Schema, for an editor's `$schema` |
| `init [<dir>]` | scaffold a project that validates |
| `skill [--agent <name>] [--global]` | print this document, or install it; `--global` installs under `$HOME` |

Add `--format json` to any verb that reports — `validate`, `plan`, `build`,
`run`, `resume`, `serve`, `cancel`, `store` — when a script is reading the output. `visualize`'s
`--format` is the odd one out: it names the **artifact** (`html` or `json`, the
page or the document the page renders) rather than the shape of a report, and
its diagnostics are `validate`'s either way. Add
`--target <name>` to the six verbs that resolve a composition against an
environment — `validate`, `build`, `run`, `resume`, `serve`, `store` — to select
a `deploy/<name>.yml`; the built-in `local` target needs no deploy file and is
what those six resolve when none is named. `plan` takes no `--target`: it
answers what changed in the composition.

## Exit codes

| code | meaning |
|---|---|
| `0` | clean, or **warnings** only, or a plan was produced, or a document was printed, or a cancel was accepted |
| `1` | the answer is no: **errors**, drift, a run with no answer, a resume that diverged from its journal, an occupied directory, a cancel the app refused (`404`, `409` for an execution already ended, or the trigger's auth) |
| `2` | the command could not run at all: bad usage, unreadable entrypoint, missing dependency, an execution id the journal does not hold open (a cancelled one among them), a cancel that reached no app |
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

## Stopping a served execution

`agent-compose cancel main.yml <execution-id> --url <app>` stops an execution a
`serve` is running: every node it is running settles `cancelled` — a model
call, a command, a harness run, a placed dispatch — each open instance ends
`cancelled` and runs its `finally:` node, every open child execution it started
is cancelled too, and the status route then reports `cancelled`. A cancelled
execution is ended for good: `resume` refuses it. A `run` is stopped with
`Ctrl-C` instead, which leaves it open for `resume`.

## Schedules and event sources

`serve` fires `schedule` triggers and runs the program behind each `event`
trigger's source; `run` does neither. A `cron:` is five POSIX fields in a
`timezone:` (default `UTC`), each tick fires once — journaled before it starts,
so a restart or a second hub never repeats it — and ticks that fell while
nothing was serving are skipped, never back-filled: a flow catches itself up
from `payload.scheduled_at`. An event source of `kind: command` is a program
printing one JSON object per line; its `checkpoint:` is handed back on every
restart as `AGENT_COMPOSE_CHECKPOINT` and as a `{checkpoint}` argument, and
`dedupe_key:` (default `payload.id`) keeps a re-emitted event from starting a
second execution. `GET /sources/<name>` reports the program. `redis_streams`,
`sqs` and `nats` validate, but `serve` refuses them by name. See
`agent-compose docs triggers` and `agent-compose explain invalid-cron`.

## Looking inside a store

`agent-compose store get main.yml store.<name> <key>` reads what a flow would
read, through the same backend; `set`, `delete`, `list`, `count`, `export` and
`import` are the rest of the surface, and a value is held to the store's
`value_schema` before it is written. Use it to seed a store before a run, or to
see what a run left, rather than writing a flow to do either. A write made this
way is not an effect of any execution and is recorded nowhere. See
`agent-compose docs cli`.

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
  in CI is what keeps the spec the source of truth. The code you *do* write by
  hand goes in a file of its own: a `tool.*` may bind `module: ./src/tools/<name>.ts`,
  and `agent-compose build main.yml` scaffolds that file once — typed signature,
  contract, a body that throws — and never writes it again. `build` overwrites
  only the files it emits, so your implementation lives in the same tree with
  nothing to merge, and it carries the ones the composition references into the
  build so they travel with the artifact. Running one is running a tool: the
  node's policy chain, the journal and the trace treat it exactly as they treat
  an `exec:`. `agent-compose docs tools` has the rules, including the `env:` and
  exactly-pinned `dependencies:` a module declares.
- **Never put a credential in a spec.** Where a secret may come from is a rule
  the compiler enforces rather than a convention you can keep by being careful;
  `agent-compose docs models` is where it is written.
