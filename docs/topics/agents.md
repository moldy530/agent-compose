# agents

An agent is **one LLM call with structured output**. It is not a loop, not a
planner, and not something that decides where the graph goes next — that is what
edges are for. What an agent may do on its own is call the tools you attached to
it, a bounded number of times.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.smart:
  provider: provider.p
  id: claude-sonnet-4-5

tool.web_search:
  description: Search the public web and return ranked result snippets.
  input:
    query: { type: string, min_length: 1 }
  output:
    results:
      type: array
      max_items: 10
      items: { type: string }
  http:
    method: GET
    url: "https://${SEARCH_HOST}/v1/search"
    query: { q: "input.query" }

agent.reviewer:
  description: Reviews a draft against the goal and returns a verdict.
  model: model.smart
  prompt: |
    You are a meticulous technical reviewer.
    Approve only when the draft fully satisfies the goal.
  tools:
    - tool.web_search
    # A built-in tool, opted into by name and taking every default. The other
    # spelling is a `tool.*` carrying a `builtin:` binding, which is where the
    # bounds are written. See `agent-compose docs tools`.
    - builtin.files
  input:
    goal:  { type: string }
    draft: { type: string }
  output:
    verdict:  { enum: [approve, revise] }
    feedback: { type: string }
  max_tool_iterations: 6

state:
  feedback: { type: string, default: "" }

flow.review:
  inputs:
    goal:  { type: string }
    draft: { type: string }
  outputs:
    feedback: { type: string }
  nodes:
    review:
      agent: agent.reviewer
      input: { goal: "input.goal", draft: "input.draft" }
      retry: { max: 2, backoff: 5s }
      timeout: 90s
  edges:
    - { from: start, to: review }
    - { from: review, to: end }
```

## Keys

| Key | Required | Default | Notes |
|---|---|---|---|
| `model` | **yes** | — | a `model.*` reference; no inline provider or settings overrides |
| `prompt` | **yes** | — | literal text, no interpolation |
| `output` | **yes** | — | a result schema with **≥ 1 property** |
| `input` | no | string-in | a field map; see below |
| `tools` | no | `[]` | `tool.*` and `flow.*` addresses, and the `builtin.*` shorthands (`agent-compose docs tools`) |
| `stores` | no | `[]` | `store.*` addresses |
| `description` | no | — | documentation only; agents are not tools |
| `max_tool_iterations` | no | `8` | integer 1..50, bounds the tool loop |

Any other key is a compile error.

## The output schema is a contract

`output:` is a result surface (`agent-compose docs schemas`): arrays inside it
need `max_items`, and `default:` is illegal. At run time the model is
**constrained** to this schema on the wire and its answer is **parsed** with it;
a nonconforming response is rejected before any edge is evaluated.

**The parse is the contract.** The two are not always the same document: a
provider's structured-output decoder compiles a subset of JSON Schema, and the
bounds it cannot compile — `max_items` among them, on most wires — come off the
request and are written into that field's own `description` instead, so the
model is asked for the bound rather than held to it. Your answer is still
checked against the whole schema, so an over-long array fails the node rather
than being trimmed. Which keywords, per wire, is in `agent-compose docs models`;
`agent-compose docs schemas` states the posture.

Enum-typed output fields are what edge guards route over and what exhaustiveness
checking covers. If you want to branch on a decision, declare it as an `enum`.

## Prompts are literal

There is **no templating**. Dynamic content reaches the model through the input
schema, which is serialized into the user turn. That keeps the spec free of
executable code and keeps prompts diffable. A prompt that must contain the six
characters `${FOO}` writes `$${FOO}`.

## Input, and the string-in default

With `input:` declared, the agent's input is that closed object, and the field
map must declare at least one field (`input: {}` is an error — omit the key
instead). At a node, fields resolve by explicit binding, then by a state channel
of the same name, then by an enclosing flow input of the same name, then by the
field's own `default:`.

With `input:` **omitted**, the agent is string-in — one unnamed string — and the
node must bind it with the scalar form:

```yaml
nodes:
  write: { agent: agent.researcher, input: "input.goal" }
```

The two forms are not interchangeable: a scalar `input:` on an agent with a
declared input object, or a field map on a string-in agent, is a compile error.

## Tools, and what a refused call does

`tools:` entries are `tool.*` or `flow.*` addresses; duplicates are an error. A
`flow.*` in a tool list **must** declare `description:` — the model needs one —
and its `inputs`/`outputs` become the tool's parameter and result schemas. See
`agent-compose docs tools` and `agent-compose docs flows`.

`stores:` attaches stores, and the compiler synthesizes LLM-facing tools from
each store's kind and `agent_access:` (`agent-compose docs stores`). A
synthesized name colliding with an attached tool name is `tool-name-collision`.

Two outcomes are carefully different:

- **A refused call comes back to the model.** Arguments the tool's declared
  schema rejects, or a call naming a tool this agent was never offered, return
  as an error tool result so the model can correct itself. This is uniform
  across `tool.*` inputs, synthesized store-tool arguments, and a `flow.*`'s
  `inputs:`.
- **A failed *execution* fails the node.** An `exec:` exiting outside its
  accepted list, an `http:` request refused, a child flow instance that failed,
  a store backend that could not answer — `retry:`/`on_error:` govern those.

The line is what the model could do about it: a schema refusal is a statement
about the *call*, which the model chose and can choose differently; an execution
failure is a statement about the world.

## Built-in tools, at run time

`builtin.bash` and `builtin.files` (`agent-compose docs tools`) are the two tools
whose program the **model** writes, and the line above is where that shows. Four
things are worth knowing at the node:

- **a shell is a session, and the session is the node execution's.** One `bash`
  child per agent node execution: the working directory and the shell state carry
  from one call to the next, and a node `retry:` starts a fresh one — the same
  restart the ordinals of a retried attempt get. The model can start a fresh one
  itself with `restart: true`, and a `command` it sends beside that runs in the
  new session rather than being dropped. It ends when the node does, whichever
  way the node ends.
- **what a command *said* is an answer, not a failure.** A nonzero exit comes
  back with its status; a command that outruns the binding's `timeout:` is killed
  and comes back saying so, with what it printed by then. Both leave the loop
  running, and the model decides. What fails the node is the tool being unusable
  — a workspace that is not a directory, a host with no `bash`.
- **the workspace is the bound, and it is per execution unless you name one.**
  Every `builtin.files` path resolves inside it and one that does not is refused
  to the model; a file inside it that carries a second name — a hard link — is
  refused for writing, since resolution cannot see where its other name is;
  `bash` starts there. A binding that writes no `workspace:` gets a fresh
  directory per execution, shared with that execution's other built-ins and
  removed when the run settles.
- **the child environment is scrubbed.** A built-in's children see the variables
  its binding declared and nothing else, unless it wrote `inherit_env: true`.

The trust framing is the same one the tools page states and is worth repeating
where the node is: every other binding fixes *what runs* at build time and lets
the model fill schema-validated parameters, while these two have the model author
the program at run time. That is the trust level `exec:` already extends to
author-arbitrary binaries, extended to the model — an agent holding
`builtin.bash` can run anything the process running the graph can run, and the
trace records what it ran (`docs/trace.md` §7.4).

## The loop is bounded

`max_tool_iterations` (default 8) bounds *turns* of the intra-agent tool loop. A
correction is another model call, so a refused call spends one exactly as a call
that ran does — there is no second counter, and several refusals in one answer
are corrected together for one iteration. A model that never corrects spends the
bound and fails the node, with the error naming the last refusal it was holding.

## The call that ends it

The structured output is a separate, final model call. **How** the schema is
asked for on that call is the provider integration's problem and never yours:
the runtime prefers each wire's own structured-output parameter — `output_config`
on Messages, `response_format` on Chat Completions, `text.format` on Responses —
and keeps a synthetic tool carrying the schema, pinned by name, as the other
rung, choosing between them per endpoint at run time. There is no key for it; the
trace records which one answered (`agent-compose docs models`).

The loop before that call ends on the model's own answer, so the runtime **closes
the exchange with one fixed user turn** — `Now produce the structured result.` —
before making it. That is a wire shape rather than a knob: a conversation ending
on an assistant turn is the prefill feature, and strict Anthropic-compatible
gateways refuse prefill beside a structured-output ask, whichever of the two
shapes it takes. The turn belongs to that one request; it is not part of the
agent's conversation and does not appear in the trace.

## As a node

```yaml
review:
  agent: agent.reviewer
  input: { goal: "input.goal", draft: "state.draft" }
  writes: { feedback: reviewer_feedback }
  retry: { max: 2, backoff: 5s }
  timeout: 90s
  on_error: { fallback: escalate }
```

Every common node key is legal. Where the output goes is
`agent-compose docs state`; what `retry:`/`timeout:`/`on_error:` resolve to is
`agent-compose docs policies`.

## The other kind of model-backed node: `coder:`

An agent node is **one LLM call** with a tool loop this runtime drives. A
`coder:` node is the other thing: a whole coding-agent harness — the vendor's own
loop, its tool shapes, its context management — run to completion as one node,
with its answer parsed against a schema you declare.

```yaml
implement:
  coder:
    harness: cc                 # or `codex`
    model: model.smart          # the same registry address an agent names
    workspace: "${REPO_ROOT}"   # required — no default
    access: workspace_write     # read_only | workspace_write | full_access
    prompt: |
      Make the smallest change that satisfies the goal, run the tests,
      and report what you touched.
    output:                     # required, like an agent's
      summary: { type: string }
    allow_tools: [Bash, Edit, Read, Write]
    env:                        # what the *program* needs — not the credential
      PATH: "/usr/bin:/bin"
  input: "input.goal"           # no `input:` in the block ⇒ string-in
  writes: { summary: summary }
  retry: { max: 1, backoff: 30s }
  timeout: 20m
```

**`harness:` is a binding.** Swap `cc` for `codex` and every other key means the
same thing — that is the point of it, and it is why a later harness is a name in
an enum rather than new grammar. What *carries* a key is the harness's own
primitive, though, and the grammar states those per harness rather than implying
they are equivalent: `access: read_only` is a sandbox under one and a permission
mode under the other, and `prompt:` joins the harness's own instructions rather
than replacing them either way. `deepagents` and `native` are reserved and
refused today; `agent-compose explain unsupported-harness` is the whole story.

**Read the containment paragraph before you write one.** What bounds a harness
run is the `workspace:`, the `access:` preset, the declared `env:`, and the
node's own `timeout:` — and nothing else. The built-in tools above are bounded
because *this* runtime implements them; a harness implements its own, so a coder
node hands both the program and the loop to somebody else's agent inside their
tool surface. `allow_tools:` is enforced per call by `cc` and is only *offered*
to `codex` — named in the run's instructions, with the sandbox as the thing that
actually holds; a `plan` report and a `visualize` canvas both say which of the
two a node is.

**One run is one journaled effect.** A resume consumes the recorded answer and
the harness never runs twice; a crash mid-run is an attempt failure and the
node's `retry:` re-runs the whole thing. The trace carries the run's turns, its
tool events and what it cost — `agent-compose docs trace`.

**`model:` is the registry address, and its connection crosses with it.** The
adapter reads three things off it: the provider-native id, the small settings
subset the harness has a place for, and the **connection** behind the provider —
its `base_url:`, its credential and its `headers:` — mapped into the harness's
own connection surface. So pointing a project at a gateway stays the one-line
edit in `providers.yml` that it is for an agent node, and a coder node's `env:`
holds what the *program* needs rather than a hand-carried copy of the
credential. Four rules come with it: a slot is an endpoint and a key on **one**
wire, so a provider whose `kind:` the harness does not speak is refused
(`agent-compose explain unsupported-provider-kind` — `cc` carries `anthropic`,
`codex` carries `openai` and `openai_compatible`, and the cloud-SDK kinds'
credentials have no slot on either); a provider that declares no `api_key:`
injects **no** credential at all rather than an empty one; a fact the bound
harness has no slot for is refused rather than dropped
(`agent-compose explain unsupported-connection-fact` — `codex` has nowhere to put
a header); and writing a variable the map would set into the node's `env:` is
refused too, because one fact gets one spelling
(`agent-compose explain conflicting-connection-variable`).

What still does **not** cross is a **route**: failover belongs to the caller that
issues a request, and a harness issues its own. Bind a direct `model.*` and let
the node's `retry:` be the ladder.

Normative source: `docs/grammar.md` §5, §5.1–5.4, §8.1, §8.9
