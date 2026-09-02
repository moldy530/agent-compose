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
**constrained** to this schema on the wire and its answer is **parsed** with the
same schema — the two are the same document by construction, so an agent cannot
honour its contract and still fail the parse. A nonconforming response is
rejected before any edge is evaluated.

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

## The loop is bounded

`max_tool_iterations` (default 8) bounds *turns* of the intra-agent tool loop. A
correction is another model call, so a refused call spends one exactly as a call
that ran does — there is no second counter, and several refusals in one answer
are corrected together for one iteration. A model that never corrects spends the
bound and fails the node, with the error naming the last refusal it was holding.

## The call that ends it

The structured output is a separate, final model call: the output schema goes on
the wire as a tool and the answer is pinned to it. The loop before it ends on the
model's own answer, so the runtime **closes the exchange with one fixed user
turn** — `Now produce the structured result.` — before pinning. That is a wire
shape rather than a knob: a conversation ending on an assistant turn is the
prefill feature, and strict Anthropic-compatible gateways refuse prefill beside a
forced tool choice. The turn belongs to that one request; it is not part of the
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

Normative source: `docs/grammar.md` §5, §5.1–5.4, §8.1
