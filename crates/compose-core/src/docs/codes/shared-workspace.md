# shared-workspace

## What it protects

A `coder:` node is a whole coding-agent harness, contained by the one directory
its `workspace:` names. Two runs inside one directory are two agents editing one
checkout: they overwrite each other's files, each one's tests see the other's
half-finished edits, and nothing in the graph says so — the runs both report
success and the answers are nonsense.

This code is the one rule that makes that unwritable, reported at the two places
it can happen:

* **A `map` over a coder node.** The fan-out is the point of putting a coding
  agent in a graph — review N pull requests, fix M files — and `workspace:` is
  evaluated in the node's input scope at **each dispatch** precisely so each one
  can have a checkout of its own. A node whose expression reads nothing from that
  scope resolves to one directory for the whole fan-out, so `max_concurrency: 8`
  is eight agents in one tree. That is an **error**.

  The per-dispatch scope is the dispatch's own `input.<field>`,
  `execution.item_index`, **and** a `state` channel a node that runs *before* the
  coder node writes. That last one is not a loophole: a dispatched flow instance
  is a separate run with its own channel values (grammar §10.1), so an `exec:`
  step inside the instance that runs `git worktree add` and writes the path to a
  channel is eight dispatches preparing eight directories — and since a coder
  node's `workspace:` cannot read another node's output, a channel is the only
  way to hand it over.

  "Before" is exact, and it is the same word grammar §8.6 rule 11 uses of
  `map.over`: every path from the flow's `start` to the coder node has to pass
  through a node that writes the channel. Channel writes land when their writer
  completes (§7.6.4), so a writer the coder node feeds rather than follows writes
  a value no dispatch ever reads, and a writer on a guarded branch beside it may
  not have run at all — in both cases every dispatch reads the channel's
  `default:`, which is one directory. The message says which shape it is: it
  names the flow that never writes the channel, or the nodes that write it
  somewhere this one cannot read.

  A **bracket** is a dot: `state['checkout']` is the same read as
  `state.checkout`, and so is `input['worktree']`. A constant key names a member,
  and the rule reads the name through either spelling.

  **The refusal labels the line that has to change**, which is often neither the
  coder node nor the map. Where the expression reads an `input.<field>`, the
  message labels the `input:` entry that bound that field — the map's own where
  the map dispatches the coder node's flow directly, and a `flow:` node's where
  one stands between them. That third site is the edit: the `workspace:` reads a
  dispatch fact the way the repair asks, the map binds its item the way the
  repair asks, and the value every dispatch shares was fixed in the middle.

  The bound read is the one the dispatch really runs under, which is not always
  the one its own map declares. A map **inside** a fan-out issues its dispatches
  once per concurrent instance of the flow that holds it, so a map saying
  `max_concurrency: 1` inside a flow an outer map fans four ways still has four
  harness runs in flight — and the refusal quotes both numbers and sends you to
  the outer map, because the inner one already says `max_concurrency: 1`.

  Nesting changes what counts as per-dispatch as well as how many runs there
  are, and this is the half that surprises people. A map's item is a value drawn
  from a list, so it tells that map's *own* dispatches apart and nothing else:
  four instances each stepping their own file list serially put two agents in
  `/srv/README.md` the moment two checkouts hold that file. So inside a fan-out
  the expression has to separate the enclosing instances too, and what does that
  is an `input.<field>` the enclosing instance itself varies — carried in at the
  inner map (`input: { dir: "input.root + '/' + file" }`) and read here as
  `workspace: "input.dir"`. `execution.item_index` is not a way round it: it
  holds the innermost dispatch's index, which nested maps repeat across outer
  items. The message says which of the two questions failed, so a refusal never
  sends you back to the line you already wrote correctly.
* **Two harness runs on concurrent branches.** Two edges of one fork that are not
  provably exclusive can both fire, so the branches they start run side by side
  (grammar §7.6.1) — and if both runs write the same `workspace:`, both are in one
  directory. That is a **warning**, and the difference is stated below.

  A branch is concurrent with another whatever construct it holds, so this is
  read over the runs a **step** contains rather than over the coder nodes a flow
  declares: a `coder:` node, and every coder node inside an instance a `flow:`
  node starts or a `map` node dispatches. One subflow instantiated twice on two
  branches is two runs of one coder node, and factoring work into a reusable flow
  — or fanning it out of a map — reads the same as writing the nodes out.

  The `map` case is the one the first site cannot reach. That refusal decides a
  map's dispatches against *each other* and passes over any fan-out bounded at 1
  — and `max_concurrency: 1` is a repair it offers. That bound holds **inside**
  the map and says nothing about the branch beside it, so a serial map and a
  sibling coder node naming one directory are still two agents in one checkout,
  and this is the warning that says so.

  Two instances are not one scope, though, so a run reached through a `flow:` or
  `map` node is compared only where its expression reads **nothing** —
  `workspace: "input.worktree"` in a flow instantiated twice is two directories
  exactly when the two instantiations bind two paths, which is the repair rather
  than the collision, and under a map it is one directory per item.

## A spec that triggers it

A map that fans four dispatches into one checkout:

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

agent.planner:
  model: model.implementer
  prompt: List the checkouts that need a change, one per failing test.
  output:
    tasks:
      type: array
      max_items: 10
      items:
        type: object
        properties:
          goal: { type: string }
          worktree: { type: string }

state:
  summary: { type: string, default: "" }
  summaries:
    type: array
    max_items: 10
    items: { type: string }
    reduce: append
    default: []

flow.fix:
  inputs:
    goal: { type: string }
    worktree: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: "'${REPO_ROOT}'"
        prompt: Make the smallest change that satisfies the goal.
        input:
          goal: { type: string }
        output:
          summary: { type: string }
      input:
        goal: "input.goal"
      writes: { summary: summary }
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }

flow.main:
  outputs:
    summaries:
      type: array
      max_items: 10
      items: { type: string }
  nodes:
    plan:
      agent: agent.planner
      input: "'every failing test'"
    work:
      map:
        over: plan.output.tasks
        as: task
        node: flow.fix
        input:
          goal: "task.goal"
          worktree: "task.worktree"
        max_concurrency: 4
        writes: { summary: summaries }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: work }
    - { from: work, to: end }
```

The item already carries the directory — `task.worktree` is bound into the
instance as `input.worktree` — and the coder node reads neither. Nothing else
about the composition is wrong, which is what makes the failure worth refusing:
it validates, it builds, it runs, and what it produces is four runs' work in one
tree.

## The fix

Four repairs, and each says something different about the graph.

* **Bind the dispatch's own path.** Carry it on the map's `input:` and read it on
  the node: `workspace: "input.worktree"`. This is the repair the ruling exists
  for — the expression is evaluated per dispatch, so each instance is contained
  by the directory its own item names. Something upstream has to *make* those
  directories: a `git worktree add` in an `exec:` node or a `tool.*` before the
  map, because provisioning source control is a step in the graph rather than a
  guess the harness adapter makes. Where the map is itself inside a fan-out, the
  path has to be built from the enclosing item as well as this map's — the item
  alone repeats across instances — which is one expression at the inner map's
  `input:` rather than a second repair.
* **Let each dispatch make its own.** That upstream step can live *inside* the
  dispatched flow instead, ahead of the coder node, writing the path it made to a
  state channel the node then reads: `workspace: "state.checkout"`. The instance
  holds its own channel values, so this is one directory per dispatch — and it is
  the only shape available when the path is not known until the dispatch runs,
  since a coder node's `workspace:` cannot read another node's output directly.
  Ahead of it on **every** path: if the step that writes the channel can be
  skipped, or runs after the coder node, the node reads the channel's `default:`
  and this is still the error.
* **Take a directory per dispatch from the runtime.** `workspace: fresh`
  provisions one under `.agent-compose/workspaces/<execution>/<instance path>`,
  named by the instance path (grammar §9.4) and created clean at the start of
  every attempt. It satisfies this rule by construction. What it does not do is
  put a checkout in it: a run that needs source control still needs the step
  above. An instance path also carries each node's **traversal ordinal**, so a
  coder node on a bounded cycle gets a *different*, empty `fresh` directory on
  the second pass — a review loop that sends the agent round again under `fresh`
  is asking it to start over, not to fix up what it wrote. A `retry:` is not
  that: it re-runs the same attempt at the same ordinal. A loop whose second
  pass needs the first pass's tree takes one of the repairs above instead, with
  the channel written by a step **outside** the loop.
* **Say the runs are serial.** `max_concurrency: 1` on the map is a statement
  rather than a workaround — one dispatch at a time is one run in the directory
  at a time — and it is the honest spelling of what a shared-workspace fan-out
  was doing anyway, minus the corruption. Where the message named an *enclosing*
  fan-out, that is the map to write it on: serialising the inner one leaves one
  run per instance and as many instances as the outer map allows.

## The fix, applied

The first repair: the item's own path, read where the run is contained.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

agent.planner:
  model: model.implementer
  prompt: List the checkouts that need a change, one per failing test.
  output:
    tasks:
      type: array
      max_items: 10
      items:
        type: object
        properties:
          goal: { type: string }
          worktree: { type: string }

state:
  summary: { type: string, default: "" }
  summaries:
    type: array
    max_items: 10
    items: { type: string }
    reduce: append
    default: []

flow.fix:
  inputs:
    goal: { type: string }
    worktree: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: "input.worktree"
        prompt: Make the smallest change that satisfies the goal.
        input:
          goal: { type: string }
        output:
          summary: { type: string }
      input:
        goal: "input.goal"
      writes: { summary: summary }
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }

flow.main:
  outputs:
    summaries:
      type: array
      max_items: 10
      items: { type: string }
  nodes:
    plan:
      agent: agent.planner
      input: "'every failing test'"
    work:
      map:
        over: plan.output.tasks
        as: task
        node: flow.fix
        input:
          goal: "task.goal"
          worktree: "task.worktree"
        max_concurrency: 4
        writes: { summary: summaries }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: work }
    - { from: work, to: end }
```

## Why the second site is a warning

Two runs on concurrent branches whose `workspace:` values are written
identically get a warning naming both, not a refusal. The rule the compiler
would have to decide is "do these two resolve to one directory", and that is not
a compile fact: a `${ENV}` reference resolves on the machine that runs the graph,
so two expressions this compiler reads as different — `"'${REPO_A}'"` and
`"'${REPO_B}'"` — may be one path at launch, and two it reads as the same is the
half it can see. Refusing on the half would be this compiler claiming a fact it
does not have, and saying nothing would be hiding the half it does.

The map site is different in exactly that respect: there the compiler is not
comparing two values, it is reading **one** expression and asking whether it
depends on the dispatch at all. That question has an answer in the composition,
so the refusal is a refusal.

Grammar: `docs/grammar.md` §4.1, §8.6, §8.9, §9.4, Decisions D138, D147. Topic:
`agent-compose docs agents`.
