# widening-permission-mode

## What it protects

`access:` and `permission_mode:` are two bounds on one run, and the second may
only choose **inside** the first. A mode may never grant an operation the node's
`access:` level would refuse — that is the whole invariant, and this code is
where it is enforced.

The admissibility table, per `access:` level, on the harness that has the axis
(`cc`):

| `access:` | derives, when `permission_mode:` is absent | admits |
|---|---|---|
| `read_only` | `plan` | `plan` |
| `workspace_write` | `acceptEdits` | `default`, `acceptEdits`, `dontAsk`, `auto` |
| `full_access` | `bypassPermissions` | all six |

Read the columns as two different statements. The middle one is what the level
means **on its own**: omit the key and the run gets exactly that mode, which is
the mapping every composition written before this key existed already has. The
right-hand one is the choice the key opens.

* **`read_only` admits `plan` alone.** The level says read the workspace and
  write nothing, and `plan` is the one mode of the six that executes no tool at
  all. Every other mode would let the loop reach a writing tool under a level
  that says it may not.
* **`workspace_write` admits four.** `default`, `acceptEdits`, `dontAsk` and
  `auto` all still run the SDK's permission machinery — they differ in *who
  answers a prompt*, which is the axis — so the containment the level states is
  the same under all four. That is the whole reason the key exists: `auto` and
  `dontAsk` were unreachable before it, by accident rather than by intent.
* **`full_access` admits all six.** It is the level that asks for no containment,
  so there is nothing left for a mode to widen.

`bypassPermissions` is the member worth reading twice. It turns the permission
machinery **off**, so it is admitted only at the level whose own derived mode
already is it. A `workspace_write` node that reached it would be a composition
whose `access:` says writes are bounded to the workspace and whose run is
bounded by nothing — with `validate` clean, `plan` and `visualize` still drawing
`workspace_write`, and the difference visible only in what the run did.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

state:
  summary: { type: string, default: "" }

flow.patch:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: "'${REPO_ROOT}'"
        access: workspace_write
        permission_mode: bypassPermissions
        prompt: Make the smallest change that satisfies the goal.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

And the same rule from the other end of the table, because **the three levels
are not a chain**. A mode can be outside its level by reaching *less* far as
well as further: `plan` executes no tool at all, and what `workspace_write`
states is that edits under the workspace are the run's job. So the most natural
planning node there is — `permission_mode: plan`, no `access:` written, and the
default `workspace_write` underneath it — is refused too, and its repair is the
**narrower** level rather than the wider one:

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

state:
  summary: { type: string, default: "" }

flow.patch:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    survey:
      coder:
        harness: cc
        model: model.implementer
        workspace: "'${REPO_ROOT}'"
        permission_mode: plan
        prompt: Read the tree and report what the change would take.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: survey }
    - { from: survey, to: end }
```

## The fix

Three repairs, and the choice is a statement about the node rather than a
formality.

* **The containment was right and the mode is not needed.** Name a mode the
  level admits, or drop the key and take the level's own. Under
  `workspace_write`, `auto` is the mode that routes a prompt to a model
  classifier and `dontAsk` is the one that denies anything not pre-approved —
  both were the point of giving this key a name, and neither widens anything.
* **The mode was right and the containment is wider than the run.** Then
  **narrow** `access:`, which is the repair a reader reaching for the widest
  level would miss. `plan` is the whole of this case: it is admitted under
  `read_only` and refused under `workspace_write`, so a node written to plan
  says `access: read_only` and gets exactly the mode it asked for, on a level
  that also says the run writes nothing. Raising to `full_access` reaches the
  same mode by asking for **no containment at all** — the widest thing this
  grammar grants, to run the one mode that executes nothing.
* **The run really does need to reach further.** Then raise `access:`, and say
  so on the node: `access: full_access` is a containment statement a reader sees
  where every other bound is written, and `plan` output reports a coder node's
  arrival as the capability it is. A mode that reached the same place with
  `access:` still reading `workspace_write` would put the widest thing this
  grammar grants behind the one key nobody has to read.

## The fix, applied

The first reading: the level stays, and the mode is one it admits.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

state:
  summary: { type: string, default: "" }

flow.patch:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: "'${REPO_ROOT}'"
        access: workspace_write
        permission_mode: auto
        prompt: Make the smallest change that satisfies the goal.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

And the second: the mode stays, and `access:` states the containment that admits
it — which here is the narrower level, not the wider one.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.implementer:
  provider: provider.anthropic
  id: claude-sonnet-4-5

state:
  summary: { type: string, default: "" }

flow.patch:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    survey:
      coder:
        harness: cc
        model: model.implementer
        workspace: "'${REPO_ROOT}'"
        access: read_only
        permission_mode: plan
        prompt: Read the tree and report what the change would take.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: survey }
    - { from: survey, to: end }
```

`permission_mode: plan` is the mode `access: read_only` derives anyway, so
writing it out changes nothing about the run — it says on the node what the
level already meant. Dropping the key is the same composition.

Grammar: `docs/grammar.md` §8.9, Decisions D138, D146. Topic:
`agent-compose docs agents`.
