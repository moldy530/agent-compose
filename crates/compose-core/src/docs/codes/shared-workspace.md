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
* **Two harness runs on concurrent branches.** Two edges of one fork that are not
  provably exclusive can both fire, so the branches they start run side by side
  (grammar §7.6.1) — and if both runs write the same `workspace:`, both are in one
  directory. That is a **warning**, and the difference is stated below.

  A branch is concurrent with another whatever construct it holds, so this is
  read over the runs a **step** contains rather than over the coder nodes a flow
  declares: a `coder:` node, and every coder node inside the instance a `flow:`
  node starts. One subflow instantiated twice on two branches is two runs of one
  coder node, and factoring work into a reusable flow reads the same as writing
  the nodes out. Two instances are not one scope, though, so a run reached
  through a `flow:` node is compared only where its expression reads **nothing**
  — `workspace: "input.worktree"` in a flow instantiated twice is two
  directories exactly when the two instantiations bind two paths, which is the
  repair rather than the collision.

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

Three repairs, and each says something different about the graph.

* **Bind the dispatch's own path.** Carry it on the map's `input:` and read it on
  the node: `workspace: "input.worktree"`. This is the repair the ruling exists
  for — the expression is evaluated per dispatch, so each instance is contained
  by the directory its own item names. Something upstream has to *make* those
  directories: a `git worktree add` in an `exec:` node or a `tool.*` before the
  map, because provisioning source control is a step in the graph rather than a
  guess the harness adapter makes.
* **Take a directory per dispatch from the runtime.** `workspace: fresh`
  provisions one under `.agent-compose/workspaces/<execution>/<instance path>`,
  named by the instance path (grammar §9.4) and created clean at the start of
  every attempt. It satisfies this rule by construction. What it does not do is
  put a checkout in it: a run that needs source control still needs the step
  above.
* **Say the runs are serial.** `max_concurrency: 1` on the map is a statement
  rather than a workaround — one dispatch at a time is one run in the directory
  at a time — and it is the honest spelling of what a shared-workspace fan-out
  was doing anyway, minus the corruption.

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
