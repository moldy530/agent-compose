# conflicting-placement

## What it protects

Two answers to one question: which worker runs this piece of work. The code
covers the two ways a composition can hold both answers at once.

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

What is **not** refused is a placed tool reached without an agent. A
`function:` node names a `tool.*` directly, and there its own placement is the
whole of the answer. That is the case a placed tool exists for.

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
whole answer — no agent's process to disagree with.

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
