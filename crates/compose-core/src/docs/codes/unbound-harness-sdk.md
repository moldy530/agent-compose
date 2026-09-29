# unbound-harness-sdk

## What it protects

A target's `harnesses:` carries an entry for a harness **no `coder:` node of the
composition binds**. It is a warning, not an error.

A generated project installs a harness's SDK only where some `coder:` node binds
that harness — `src/harness.ts` imports only the drivers a composition uses, and
`package.json` declares only their packages. So an entry for any other harness
changes nothing `build` writes: no manifest line moves, no driver's fallback
changes, no README row appears. The warning says so, because a version written
in a deploy file reads like a decision about the project, and this one decides
nothing.

It is not refused, and that is deliberate. A deploy file outlives the
compositions it serves: a target that pins `codex`'s SDK for the flow a colleague
is about to add is a key waiting for its composition, not a mistake about this
one. That is the difference from the inert keys the grammar *does* refuse —
`else: false`, a `url:` on a `sqlite` journal — whose author expected an effect
now and would get none.

The entry is still judged in full before this is reported: a name that is not a
harness, a reserved one, a `${…}` version, a range spelling or a version outside
the audited range is refused whether or not a node binds the harness, so the
entry waiting for its composition is one that will build when it arrives.

## A spec that triggers it

The composition binds `cc` and nothing else:

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
  codex:
    sdk_version: "0.154.0"
```

## The fix

**Key the entry by the harness the composition binds**, if moving *that* SDK is
what you meant:

```yaml deploy moved
version: "0.1"

harnesses:
  cc:
    sdk_version: "0.3.285"
```

**Or drop it**, if the composition that binds `codex` is not this one — and
keep it in the deploy file of the target that builds that composition instead:

```yaml deploy dropped
version: "0.1"
```

Grammar: `docs/grammar.md` §14.8, Decision D151. Topic:
`agent-compose docs targets`.
