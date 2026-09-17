# conflicting-connection-variable

## What it protects

A `coder:` node's `model:` carries its provider's connection into the run: the
endpoint, the credential and the headers are mapped into the harness's own
connection surface, and on `cc` that surface is the run's **environment** —
`ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY`, `ANTHROPIC_CUSTOM_HEADERS`. On
`codex` the credential is a client option the SDK then sets as `CODEX_API_KEY`
in the environment it spawns its CLI with.

The node's own `env:` fills that same environment. So a node that declares one of
those variables and a provider that declares the fact behind it are two sources
writing one name, and there are only three things a compiler can do about it:
pick one silently, invent a precedence rule for authors to memorize, or refuse.

**It refuses.** A silent pick is a run that authenticates against the wrong
endpoint with nobody told, and a precedence rule is a fact about this compiler
that has to be remembered at every call site. One spelling per fact needs no
rule at all.

The check is computed from what the provider **declares**, not from the table:

* a provider with no `api_key:` injects **no credential variable** — not an empty
  one — so a node bound to a keyless gateway is free to declare a credential
  variable of its own, and this diagnostic does not fire;
* a fact whose slot is a typed option rather than a variable is not a collision
  either. `base_url:` under `codex` becomes a `--config` flag and touches no
  environment, so a node may declare whatever it likes beside it.

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

flow.fix:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        env:
          PATH: /usr/bin:/bin
          ANTHROPIC_API_KEY: ${ANTHROPIC_API_KEY}
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

The two lines happen to name the same variable here, which is what makes the
shape worth refusing rather than tolerating: it reads like belt and braces and is
indistinguishable from the case where they disagree.

## The fix

**Drop the `env:` entry.** The provider's credential already reaches the run —
that is what binding a `model.*` on it does — and the node's `env:` is for the
variables the *program* needs: a `PATH`, a proxy setting, a token some tool it
shells out to reads.

If the node genuinely needs a different endpoint or a different credential from
the one its model's provider declares, that is a second connection and it is
spelled as one: define another `provider.*`, bind a `model.*` to it, and point
this node's `model:` there. Providers are cheap on purpose.

## The fix, applied

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

flow.fix:
  inputs:
    goal: { type: string }
  outputs:
    summary: { type: string }
  nodes:
    implement:
      coder:
        harness: cc
        model: model.implementer
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        env:
          PATH: /usr/bin:/bin
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

Grammar: `docs/grammar.md` §8.9, §12.1, Decision D143. Topic:
`agent-compose docs agents`.
