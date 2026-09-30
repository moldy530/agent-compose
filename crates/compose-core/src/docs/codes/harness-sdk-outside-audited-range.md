# harness-sdk-outside-audited-range

## What it protects

A target's `harnesses:` pins a harness's SDK to a version **outside the range
this compiler release audited it for**, above it or below it.

Every compiler release pins each harness SDK exactly, and for two reasons. A
harness SDK *is* what a `coder:` node's behaviour is. And the reserved
`settings:` list — the options a `settings:` key may not reach, because each one
would reach around `workspace:`, `access:`, `allow_tools:`, `model:` or another
bound the node states — is an audit of **one** release's option surface: a
release nobody read may carry an option that reaches around a bound, and nothing
in `validate` would know. `harnesses:` lifts the first reason and keeps the
second. A target may move the SDK, but only inside the range the audit covers:

| Harness | SDK | Pin | Audited range |
|---|---|---|---|
| `cc` | `@anthropic-ai/claude-agent-sdk` | `0.3.284` | `[0.3.284, 0.4.0)` |
| `codex` | `@openai/codex-sdk` | `0.154.0` | `[0.154.0, 0.155.0)` |

Each range is `[audited, next minor)`: the release the audit was performed at,
forward through that minor. What it promises is precise and small — **no
*known* reach-around**. Every option the audited release accepts was classified
there, under both readings of its option surface (the typed options *and* the
names its runtime option reader takes), and the driver's other contracts — the
callback a bare `allowedTools` entry would shadow, the permission-mode table,
the connection table — were verified there too. Within the SDK's own patch
series the compiler **expects** those to hold, and says plainly that it has not
checked them beyond the floor.

**Above the range** is the next minor, which nobody audited. **Below it** is
refused as well: an older SDK carries a subset of the audited options, but it
is not the release the driver's contracts were verified against, and the reason
to go backward — a CLI that works — is a reason for a compiler release to move
the floor, not for a deployment to reach below it.

Placement is semver precedence, and the range is one minor's releases:
`0.3.99` is *below* `0.3.284` whatever its text sorts as, a prerelease of the
floor (`0.3.284-rc.1`) is below the floor, and `0.4.0-rc.1` is above the range
even though it precedes `0.4.0`, because it is a release of the minor nobody
audited.

**What this is not.** A range spelling — `^0.3.284`, `0.3.x` — is
`invalid-value`, and a `${CC_SDK_VERSION}` is `unexpected-env-ref`: both are
refused before the range is read, because neither is one version `validate`
can place. A key that is not a harness is `unknown-variant`; `deepagents` and
`native` are `unsupported-harness`. An entry for a harness no `coder:` node
binds is `unbound-harness-sdk`, a warning.

## A spec that triggers it

The composition is beside the point; the deploy file is what refuses:

```yaml triggers
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.coding:
  provider: provider.anthropic
  id: coding-model

flow.fix:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    implement:
      coder:
        harness: cc
        model: model.coding
        workspace: "'${REPO_ROOT}'"
        prompt: Make the failing test pass, then say what you changed.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

```yaml deploy canary
version: "0.1"

harnesses:
  cc:
    sdk_version: "0.4.0"
```

`0.4.0` is `cc`'s range's own ceiling: the range is half-open, so the first
release of the next minor is the first version this release did not audit.

## The fix

**Pin a version inside the range.** If the patch you need is in the audited
minor, name it — `build` writes it into `package.json` in the pin's place, for
this SDK alone, and the emitted README's pins table shows it beside the pin.

```yaml deploy patched
version: "0.1"

harnesses:
  cc:
    sdk_version: "0.3.285"
```

**Or drop the entry**, and the project installs this release's pin.

```yaml deploy pinned
version: "0.1"
```

If what you need is only past the range, the way forward is a compiler release
that re-audits the SDK's option surface and moves the floor — never a deploy
key. A range is widened beside the audit it rests on, so the day it moves, the
reserved list has been read against the release it admits.

Grammar: `docs/grammar.md` §14.8, §8.9, Decision D151. Topic:
`agent-compose docs targets`.
