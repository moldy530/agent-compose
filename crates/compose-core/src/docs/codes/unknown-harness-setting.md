# unknown-harness-setting

## What it protects

**A warning, and the composition still builds.** Like `unknown-server-tool`,
this is a diagnostic that exists to say what the compiler *did not* check.

A `coder:` node's `settings:` is harness config: the vendor's own options,
written in the vendor's own vocabulary. The compiler keeps a curated table per
harness and checks a key it knows strictly — a turn bound outside its range, a
budget written as a string, a word outside a closed set — because a config the
SDK will refuse is a run that dies after the workspace has already been prepared.

Everything else travels to the SDK unchanged. That is the whole design: a
harness option the vendor ships tomorrow has to be usable the day it ships, not
one compiler release later. The warning is the honest half of the bargain — it
names exactly what could not be verified, so a typo does not look like working
configuration, and it suggests the near miss when there is one.

**With one bound: unchecked is not unbounded.** A key that spells an option the
generated adapter owns is dropped rather than passed — the working directory, the
permission mode or sandbox preset, the environment, the output schema, the system
prompt, the tool allowlist, the abort signal, the model. Those are what
`workspace:`, `access:`, `env:`, `output:`, `prompt:`, `allow_tools:`,
`timeout:` and `model:` say, and `settings:` is not a second way to say them.

Nor does an option have to *spell* a bound to reach around one, so the dropped
set is wider than that list. A harness SDK ships options that **contain** the
bounds: extra command-line arguments, a settings file or object of permission
rules, roots beside the working directory, sandbox configuration, MCP servers
and agent definitions that put a tool — or a whole loop — within reach of a run
whose `allow_tools:` never named it, hooks and permission handlers that move the
decision elsewhere, a fallback model where the connection was supposed to stop.
Those are dropped too, and so is every option that resumes a previous session,
for the other reason: harness-native resume is a named exclusion, because a
machine-local session store is not the journal. The dropped set is per harness
and audited against the SDK release this compiler pins.

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
        harness: cc
        model: model.smart
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        settings:
          max_turn: 40
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

`max_turn` is one letter short of `max_turns`, which the `cc` table knows and
checks. As written it reaches the SDK as an option nothing reads, and the run
takes as many turns as it likes.

## The fix

Read the `help:` line. Where the spelling is close to a key the table knows, the
diagnostic names it; otherwise it lists what this harness's table holds.

Then decide which case you are in:

* **a typo, or the other harness's spelling** — correct it, and the strict tier
  checks the value from then on;
* **an option this release predates** — leave it. The warning is the record that
  the value is unchecked, and `build` emits the project anyway.

## The fix, applied

The key the `cc` table actually knows, which puts the setting back in the
checked tier.

```yaml spec
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
        harness: cc
        model: model.smart
        workspace: ${REPO_ROOT}
        prompt: Fix the failing test, then say what you changed.
        settings:
          max_turns: 40
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

Grammar: `docs/grammar.md` §8.9, Decision D140. Topic:
`agent-compose docs agents`.
