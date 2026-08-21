# getting-started

An agent-compose project is YAML that compiles to a LangGraph TypeScript
project. You declare components — models, agents, tools, flows — and the
compiler checks that the graph they form can actually run before any of it does.

Here is a whole project. Save it as `main.yml` and run
`agent-compose validate main.yml`.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-5

agent.summarizer:
  model: model.smart
  prompt: Summarize the document you are given in two sentences.
  input:
    document: { type: string, min_length: 1 }
  output:
    summary: { type: string }

state:
  summary: { type: string, default: "" }

flow.summarize:
  inputs:
    document: { type: string, min_length: 1 }
  outputs:
    summary: { type: string }
  nodes:
    run: { agent: agent.summarizer, input: { document: "input.document" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }

triggers:
  cli: { type: manual, flow: flow.summarize }
```

## The loop

```
agent-compose init my-project        # a project like this one, in a new directory
agent-compose validate main.yml      # every static check, in milliseconds
agent-compose explain <code>         # what a reported code means and how to fix it
agent-compose docs <topic>           # a construct you have not met before
agent-compose plan before.yml main.yml   # what a change did to the graph
agent-compose run main.yml flow.summarize --input document="…"
```

`validate` is the loop. It parses, follows `imports:`, resolves every reference,
and runs every static check — reference resolution, schema compatibility across
edges, routing exhaustiveness, cycle termination, fan-out bounds. Run it after
every edit; it is designed to be cheap enough for that.

## Files, and how they find each other

A project is one entrypoint plus whatever it imports. **Only the entrypoint may
declare `imports:`**, and imports are not transitive — the import list is the
authoritative statement of what is in this graph. There is no directory
scanning and there are no globs.

```yaml
# main.yml — the entrypoint
version: "0.1"
imports:
  - providers.yml
  - models.yml
  - agents/reviewer.yml
  - flows/review_loop.yml
  - triggers.yml
```

Paths are relative to the entrypoint's directory (the *project root*), use `/`
separators, and must stay inside the root. A `deploy/<target>.yml` is never
imported — it is selected with `--target`.

The conventional layout is a convention, not a rule:

```
main.yml            version, imports, state, defaults
providers.yml       provider.* definitions
models.yml          model.* definitions
agents/*.yml        agent.* definitions
tools/*.yml         tool.* definitions
flows/*.yml         flow.* definitions
stores/*.yml        store.* definitions
triggers.yml        the triggers section
deploy/staging.yml  a deploy target (see `agent-compose docs targets`)
```

`version: "0.1"` is required in the entrypoint and in every deploy file, and
must be **quoted** — unquoted `0.1` is a YAML float. An imported file may
declare it, and then it must match.

## The YAML profile

One document per file, a mapping at the root, string keys, no duplicate keys
(never last-wins). Anchors and aliases work; **merge keys (`<<:`) and tags
(`!!str`) do not**. UTF-8, no byte-order mark.

## Typed addresses

Every definition is a top-level key of the form `<namespace>.<name>`:

| Namespace | Defines |
|---|---|
| `agent.` | one LLM call with structured output |
| `tool.` | a callable implementation |
| `flow.` | a subgraph module |
| `store.` | durable attachable storage |
| `provider.` | an inference connection |
| `model.` | a model binding, or a failover route |

An address is global across the composition, whichever file declares it —
splitting into files is authoring convenience, and the compiler flattens
everything into one artifact. Namespaces are disjoint, so `agent.triage` and
`tool.triage` can coexist.

The same string used as a *value* is a **reference**, and every reference
position accepts a fixed set of namespaces. `model:` on an agent takes a
`model.*` and nothing else; a node's `agent:` takes an `agent.*`; a trigger's
`flow:` takes a `flow.*`. Anything else is `invalid-reference`, and a name
nothing defines is `undefined-reference` — usually with a spelling suggestion.

Names are lowercase snake_case, 1–64 characters, starting with a letter. One
identifier class covers definition names, node ids, channel names, field names,
variant tags, trigger names, and backend aliases.

## Reserved names

Seven identifiers already mean something inside an expression and may not be
reused: `input`, `state`, `execution`, `item`, `messages`, `output`, `payload`.
They are illegal as a state channel name and as a flow-local node id (`item` is
legal as a `map`'s `as:` binding, since that is its own default name). `start`
and `end` are additionally reserved as node ids. Definition *names* are
unaffected — `agent.state` is fine, because a namespaced address is never a bare
token in an expression.

## Where to go next

`agent-compose docs schemas` for the type language every declaration uses;
`agent-compose docs flows` and `agent-compose docs routing` for the graph;
`agent-compose docs cli` for the verbs and their exit codes.

Normative source: `docs/grammar.md` §1, §1.1–1.7, §2, §2.1–2.3, §2.5
