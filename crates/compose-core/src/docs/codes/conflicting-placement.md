# conflicting-placement

## What it protects

Two answers to one question: which worker runs this piece of work. The code
covers every way a composition can hold both answers at once.

**Two placements naming one component.** Placements are disjoint. A worker
claiming `mac` and a worker claiming `gpu` are different machines by
construction — that is what a claim is for — so a component named in both would
leave the hub to pick, and the pick would be invisible in the spec. The choice
belongs to the author, in the deploy file, where both lines are.

**A placed tool attached to an agent placed elsewhere.** This is the one worth
reading twice, because it looks like two independent statements and is not. The
whole generated artifact reaches every worker (there is no per-placement
slicing), so a placement decides which *process* runs a node rather than which
code exists where. An attached tool is called from inside its agent's own tool
loop, in the process running the agent — so whichever worker took the agent's
dispatch runs the tool call too, and the tool's own placement never gets a say.

Three of the four combinations are fine and one is the failure:

| the agent | the tool | verdict |
|---|---|---|
| placed `mac` | no placement | fine — the artifact is everywhere, and the tool runs where the agent runs |
| placed `mac` | placed `mac` | fine — the same answer written twice |
| placed `mac` | placed `gpu` | refused |
| no placement | placed `mac` | refused — the agent runs on the hub, so the tool would too |

The last row is the one an author is most likely to write. A `mac`-only tool
attached to an agent nobody placed runs on the hub, where the signing keys are
not — a placement written, accepted, and silently ignored.

It is the last row even where the agent *looks* like it can only ever run inside
somebody else's process — an agent named by a flow that a placed agent attaches,
and by nothing else. Every flow a composition declares is runnable on its own,
whether or not a `manual` trigger names it, so that flow is one the hub can start
directly and the `agent:` node inside it is dispatched by the hub when it does.
The repair is the same as any other last row: name the agent in the placement
too.

**A placement reached through a flow the agent attaches.** The same table, one
indirection out. A `flow.*` in a `tools:` list cannot itself be placed — placing
a flow is deferred — but a flow-as-tool call starts an instance *inside the same
tool loop*, so every agent and tool that instance reaches runs in the attaching
agent's process too. A `mac` tool reached only that way is the last row again,
with a flow standing between the two lines.

What is **not** refused is anything the hub schedules. A `function:` node names
a `tool.*` directly, and there its own placement is the whole of the answer —
that is the case a placed tool exists for. The same goes for a flow instantiated
by a `flow:` node rather than attached as a tool: the hub schedules its nodes,
so each placement inside it is honoured.

## A spec that triggers it

`agent.builder` attaches `tool.xcodebuild`, and the deploy file puts the two on
different machines:

```yaml triggers
version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

tool.xcodebuild:
  description: Build and sign the macOS app.
  input:
    scheme: { type: string }
  output:
    log: { type: string }
  exec:
    command: xcodebuild
    args: ["-json"]

agent.builder:
  model: model.smart
  prompt: Build the requested scheme and report what the log says.
  tools: [tool.xcodebuild]
  input:
    scheme: { type: string }
  output:
    verdict: { type: string }
```

```yaml deploy mesh
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  mac:
    members: [tool.xcodebuild]
  gpu:
    members: [agent.builder]
```

## The fix

**Put the two in one placement.** If the tool needs the signing keys, the agent
that calls it has to run where they are — the call happens in that agent's
process. One placement holding both says exactly that, and it is the repair
below.

**Or drop the tool's placement.** The artifact is everywhere, so a tool that
claims nothing runs wherever the agent that called it runs. Write the placement
on the agent alone and the tool follows it.

**Or reach the tool from a `function:` node.** A tool invoked by the graph
rather than by a model is dispatched on its own, and its placement is then the
whole answer — no agent's process to disagree with. Where the conflict came
through a `flow.*` in the agent's `tools:`, the same repair is a `flow:` node:
the hub schedules the instance's nodes, so every placement inside it is honoured
instead of being folded into the caller's process.

For the disjointness half, the repair is the same shape: name the component in
one of the two placements. If it genuinely needs to run on either machine,
that is one placement claimed by both workers — several workers claiming one
name form a pool the hub dispatches across, which is what a pool is for.

```yaml deploy fixed
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  mac:
    members: [agent.builder, tool.xcodebuild]
    description: the machine with the signing keys
```

Grammar: `docs/grammar.md` §14.1, §14.2, Decisions D128, D129. Topic:
`agent-compose docs targets`. Protocol: `docs/distributed.md`.
