# flows

A flow is a module: a subgraph with a declared I/O surface that is
interchangeable with a tool's. That interchangeability is the point — the same
`flow.review_loop` can be run from the CLI, targeted by a trigger, instantiated
by a `flow:` node, dispatched by a `map`, or handed to an agent as a tool.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.writer:
  model: model.m
  prompt: Write a section for the goal you are given.
  output:
    draft: { type: string }

agent.editor:
  model: model.m
  prompt: Tighten the section you are given.
  input:
    draft: { type: string }
  output:
    polished: { type: string }

state:
  draft:    { type: string, default: "" }
  polished: { type: string, default: "" }

flow.write_section:
  description: Draft one section and polish it.
  inputs:
    goal: { type: string }
  outputs:
    polished: { type: string }
  nodes:
    write: { agent: agent.writer, input: "input.goal" }
    edit:  { agent: agent.editor, input: { draft: "state.draft" } }
  edges:
    - { from: start, to: write }
    - { from: write, to: edit }
    - { from: edit, to: end }

flow.report:
  inputs:
    topic: { type: string }
  outputs:
    polished: { type: string }
  nodes:
    section:
      flow: flow.write_section
      input: { goal: "input.topic" }
      context: isolated
      policy: { timeout: 120s, on_error: skip }
      timeout: 300s
  edges:
    - { from: start, to: section }
    - { from: section, to: end }
```

## The definition

| Key | Required | Notes |
|---|---|---|
| `nodes` | **yes** | ≥ 1 node, keyed by flow-local node id |
| `edges` | **yes** | ≥ 1 edge |
| `outputs` | **yes** | the module's result surface |
| `inputs` | no | default: no inputs |
| `description` | no | **required** when the flow is used as an agent tool |
| `finally` | no | a node of this flow that runs once when an instance ends — see [Finally](#finally) |

Node ids are scoped to their flow — two flows may both have a node `review`.
`start` and `end` are pseudo-nodes and may not be node ids.

## A node object

Exactly one **kind key**, plus common keys:

| Kind key | Value |
|---|---|
| `agent` | an `agent.*` reference |
| `exec` / `http` | an inline block |
| `function` | a `tool.*` reference |
| `flow` | a `flow.*` reference |
| `map` | an inline block |
| `human` | an inline block |
| `store` | a `store.*` reference |

Common keys: `input`, `writes`, `retry`, `timeout`, `on_error`, `description`.
A node with no kind key, or with two, is a compile error naming the node.

The rule of thumb: **schemas and implementation config live inside the kind
block; bindings and policy live at node level.**

## Inputs and outputs

`inputs:` is the parameter surface; inside the flow, `input.<field>` is in scope
everywhere.

`outputs:` is materialized **once, at quiescence**: each field is read from the
**state channel of the same name**, which must be declared in `state:`. There is
no `returns:` binding — use a node `writes:` remap to feed a differently-named
channel. The channel must also be able to *satisfy* the field: every value it
can hold must be a legal value of the field's declared type.

That is why the example above declares a `polished` channel: the flow's
`outputs.polished` reads it, and `agent.editor`'s `polished` output field writes
it by name.

## As a node

```yaml
sub:
  flow: flow.review_loop
  input: { goal: "state.topic" }
  writes: { draft: section_draft }
  context: isolated
  policy: { timeout: 120s, on_error: skip }
  timeout: 300s
```

**Bindings are total.** Every input field the subflow declares without a
`default:` must be bound by the instantiating node's `input:`. Nothing falls
through by name across a module boundary — a subflow declaring `{goal, draft}`
instantiated with `input: { goal: … }` is a compile error naming `draft`, even
where the caller happens to have a `draft` channel.

`context:` is `isolated` (the default) or `inherit`, and decides whether the
subflow shares the caller's conversation history. It is a `flow:`-node key only.

**`policy:` and the node's own policy keys are different things**, and a `flow:`
node may carry both:

- `policy:` sets the policy for **every node inside** the instantiated subflow,
  propagated into nested instantiations. Where two instantiation sites in a
  nesting chain set the same field, the **outermost** wins.
- node-level `retry`/`timeout`/`on_error` are this node's own: `timeout` bounds
  the whole instance, `on_error` fires when the instance fails, and `retry`
  re-executes the instance from its entry as a fresh instance.

So `{ policy: { timeout: 30s }, timeout: 10s }` reads "no node inside may run
longer than 30s, and the whole instance may not run longer than 10s".

## As a tool

Listing `flow.review_loop` in an agent's `tools:` makes its `inputs`/`outputs`
the tool's parameter and result schemas; `description:` becomes required. One
call starts an instance exactly as a `flow:` node does, and:

- its **arguments are the flow's `inputs:`**, checked before anything is
  instantiated; arguments the schema refuses go back to the model as an error
  tool result;
- its **conversation history is isolated**, always — there is no `context:` key
  at a tool attachment;
- its **`execution` is the caller's**, so a session-scoped store inside it
  addresses the caller's partition;
- the agent node's **`timeout:` bounds it**;
- a call that *fails* — the instance failed, or reached quiescence with no
  output — **fails the agent node**. It is never answered with a plausible
  result and never quietly dropped.

## Reachability, and no recursion

A flow **reaches** its own nodes' components, its maps' dispatch targets, the
stores and tools of any agent it reaches (including `flow.*` tool entries), and
everything those reach in turn. One relation, used by four checks: session
coherence, sync-trigger interrupt-freedom, detached-dispatch
interrupt-freedom, and recursion.

**A flow that reaches itself is a compile error.** There is no termination proof
analogous to the cycle rule and it would break the fan-out bounds. Extract the
shared part into a third flow, or break the cycle.

Every node must be reachable from its flow's `start`, counting edges plus the
two control-transfer positions — `on_error: { fallback: … }` and
`human.on_timeout:`. A `cleanup` node reached only by a fallback is live code.
So is the flow's `finally:` node, which is reachable because it is `finally:`.

## Finally

`finally: <node id>` names the node an instance of the flow runs **once, when it
ends** — whether the body completed, failed, or (once cancel exists) was
cancelled. Inside that node, and nowhere else, `execution.outcome` says how
(`completed`, `failed`, `cancelled`) and `execution.error` says why, where the
body failed:

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5
agent.drafter:
  model: model.m
  prompt: Draft what you are asked for.
  output:
    draft: { type: string }
store.runs:
  kind: kv
  scope: global
  value_schema:
    outcome: { type: string }
    error:   { type: string }
state:
  draft: { type: string, default: "" }
flow.write_up:
  inputs:
    topic: { type: string }
  outputs:
    draft: { type: string }
  nodes:
    write:
      agent: agent.drafter
      input: "input.topic"
    record:            # runs once, however `write` went
      store: store.runs
      op: set
      key: "execution.id"
      value:
        outcome: "execution.outcome"
        error: "has(execution.error) ? execution.error : ''"
      retry: { max: 2, backoff: 1s }
      timeout: 10s
  edges:
    - { from: start, to: write }
    - { from: write, to: end }
  finally: record

triggers:
  cli: { type: manual, flow: flow.write_up }
```

**It is outside routing.** No edge names the finally node, at either end, and
neither does an `on_error: { fallback: … }` or a `human.on_timeout:` — each is a
compile error (`routed-finally`). It needs no `- { from: …, to: end }` either:
it is on no branch.

**It runs once per instance, innermost first.** The root instance of a run and
every `flow:` node's, `map` dispatch's and flow-as-tool call's instance run
their own flow's finally node when they end, so a subflow's finally node runs
before the node that instantiated it settles, and the caller's runs at the
caller's end. That includes a `flow:` node or a `map` whose own `timeout:` ran
out: the deadline stops the instance, and the node fails only once the
instance's finally node has run. A detached `flow.*` dispatch is a child
execution, and runs its own. A body that ended through `on_error: skip` or a `fallback:` to `end`
**completed**, and its finally node reads `completed`.

**A pause is not an end.** A run that parks at a `human` node runs no finally
node; the resumed run answers the pause, runs the body on, and runs it when the
instance ends. That is also why the finally node may not be a `human` node or
reach one, through a subflow, a map or an agent's flow tools
(`finally-interrupt`): a step that must always finish cannot wait on a person.

**It changes nothing the instance produced.** The outputs are read before it
runs, so a completed run stays completed — same outputs, exit code `0` — even
when its finally node fails. Its own `retry:` and `timeout:` apply, through the
whole policy chain; `on_error:` does not, at any level — once its retries are
spent, its failure is `finally_error` on the execution's record (the journal row
and the trace envelope), and a `run` prints it on one stderr line.

**A restart runs it once.** The end of the instance is journaled before the
finally node starts, and its own effects are journaled like any node's: a
process killed in the middle of it resumes, replays the body, and runs it again
only up to what it had not done yet.

`execution.outcome` and `execution.error` anywhere but inside the finally node —
an edge guard, another node's `input:`, a store key — are a compile error
(`outcome-outside-finally`).

Normative source: `docs/grammar.md` §7, §7.1, §7.5, §7.7, §7.9, §8.5, Decision D159
