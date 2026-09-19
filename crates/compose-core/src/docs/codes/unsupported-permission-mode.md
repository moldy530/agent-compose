# unsupported-permission-mode

## What it protects

A `coder:` node carries **two** bounds, not one, and they answer different
questions. `access:` is containment — how far a run may reach — and it has three
levels, `read_only`, `workspace_write` and `full_access`. `permission_mode:` is
the harness's own **approval axis**: inside that reach, how a call gets
approved.

Only one of the two harnesses has the second axis.

* **`cc`** reaches the Claude Agent SDK, whose containment primitive *is* a
  permission mode plus a working directory. Its modes are `default`,
  `acceptEdits`, `bypassPermissions`, `plan`, `dontAsk` and `auto`, and
  `permission_mode:` is how a node names one.
* **`codex`** reaches the Codex SDK, whose containment primitive is a sandbox
  preset — the one `access:` selects, one-to-one. Its per-call approval tier
  lives in an app server this release deliberately does not adopt, so there is
  nothing on this harness for a mode to select.

So the key is **refused** on `codex` rather than mapped onto something close. A
run that escalated out of its sandbox to an approver nothing here answers for
would be `access:` saying one thing and the run doing another — and a slot
invented to take the value would be a bound this compiler claims and nothing
holds. The two harnesses are stated per harness and never implied equivalent,
which is the same discipline `allow_tools:` is under: one harness enforces it
in-loop and the other is offered it, and this document says so rather than
letting a reader assume.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.openai:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.reviewer:
  provider: provider.openai
  id: gpt-5-codex

state:
  summary: { type: string, default: "" }

flow.review:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    review:
      coder:
        harness: codex
        model: model.reviewer
        workspace: ${REPO_ROOT}
        access: read_only
        permission_mode: plan
        prompt: Read the working tree and report what it does.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: review }
    - { from: review, to: end }
```

## The fix

Two repairs, and which is right depends on which half was meant.

* **The containment was the point.** Drop `permission_mode:`. `access:` is the
  whole of what `codex` holds a run to, and it says the same thing under both
  harnesses: `read_only` is read the workspace and write nothing, and this
  harness holds it at the operating system — the tree is readable, commands run,
  and a write fails. Nothing is lost by taking the key off.
* **The mode was the point.** Run the node under `cc`, which carries the axis.
  Every other key means the same thing: `model:`, `workspace:`, `access:`,
  `prompt:`, `input:`, `output:` and `env:` are written identically, which is
  what makes the harness a binding. Note that `cc` carries the connection of an
  `anthropic` provider, so the node's `model:` moves with it.

What is **not** a repair is reaching for `settings:`. The harness options that
spell an approval policy are on that harness's reserved list and are refused
there too (`agent-compose explain reserved-harness-setting`) — for this reason:
a bound has to be readable off the node that holds it.

## The fix, applied

The containment reading of the same node, with the key taken off.

```yaml spec
version: "0.1"

provider.openai:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.reviewer:
  provider: provider.openai
  id: gpt-5-codex

state:
  summary: { type: string, default: "" }

flow.review:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    review:
      coder:
        harness: codex
        model: model.reviewer
        workspace: ${REPO_ROOT}
        access: read_only
        prompt: Read the working tree and report what it does.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: review }
    - { from: review, to: end }
```

Grammar: `docs/grammar.md` §8.9, Decisions D138, D146. Topic:
`agent-compose docs agents`.
