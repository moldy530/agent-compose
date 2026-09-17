# unsupported-harness

## What it protects

A `coder:` node runs a coding-agent harness as one node, and which harness is a
**binding**: `harness:` names one of four, and the composition reads the same
whichever one serves it. Two of the four — `deepagents` and `native` — are
spelled by the grammar and are **reserved**: this release lowers `cc` and
`codex` and has no driver for the other two.

They are in the grammar on purpose rather than left out. A name the enum does
not have is a typo, and the compiler answers a typo with a suggestion; a name it
has and does not lower is a *scope* statement, and the two deserve different
answers. Writing `harness: deepagents` and being told "did you mean `codex`?"
would suggest the composition was misspelled, when what it was is early.

## A spec that triggers it

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-5

state:
  summary: { type: string, default: "" }

flow.fix:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: deepagents
        model: model.smart
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

## The fix

Bind one of the two harnesses this release ships:

* **`cc`** — the Claude Agent SDK. It enforces the node's `allow_tools:` inside
  its own loop, and takes a working directory and a permission mode as its
  containment.
* **`codex`** — the Codex SDK. It bounds at its sandbox preset, which is what
  `access:` selects, and does not enforce a tool allowlist in-loop.

Nothing else about the node changes: `model:`, `workspace:`, `access:`,
`prompt:`, `input:`, `output:`, `env:` and the `retry:`/`timeout:`/`on_error:`
chain are the same keys whichever harness runs them. That is what makes the
choice a binding.

If you are here because you want a harness this release does not have, the set
grows by a resolved question in `prd.md` rather than by a release adding a name
— which is the same rule the built-in tool set is under.

Grammar: `docs/grammar.md` §8.9, Decision D136. Topic:
`agent-compose docs agents`.
