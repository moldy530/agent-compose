# reserved-harness-setting

## What it protects

A `coder:` node's `settings:` is an **open** object holding the harness's own
configuration, and it is open on purpose: a harness option the vendor ships
tomorrow has to be usable the day it ships, so a key this release cannot speak
for travels to the SDK unchanged under a warning
(`agent-compose explain unknown-harness-setting`).

**Open is not unbounded.** What `settings:` buys is the vendor's *other*
options, never the ones this node already states. Each harness's SDK surface has
options that are a bound written somewhere a reader and `validate` can both see
it — the working directory, the permission mode or sandbox preset, the
environment, the output schema, the system prompt, the tool allowlist, the abort
signal, the model — and those are `workspace:`, `access:`, `permission_mode:`,
`env:`, `output:`, `prompt:`, `allow_tools:`, `timeout:` and `model:`
respectively. A key here that reached one would make `settings:` a second way to
say those things and the only one no check covers, which turns the one
deliberately open surface into the way around all the closed ones.

An option does not have to *spell* a bound to reach around it, and the reserved
set is the wider one. `extraArgs` is any command-line flag there is —
`dangerously-skip-permissions` and `add-dir` among them. `mcpServers`, `agents`
and `plugins` put a tool or a whole loop within reach of a run whose
`allow_tools:` never mentioned it. `additionalDirectories` is sandbox roots
beside `workspace:`. `pathToClaudeCodeExecutable`, `executable` and
`executableArgs` choose which program the run *is*, so a key among them does not
widen one bound — it replaces the program enforcing all of them.

**This is an error rather than a dropped value with a warning**, and that is a
deliberate change from how the key behaved when coder nodes first shipped. A key
that was warned about and then silently removed at run time left an author with
no way to tell a setting that travelled from one that never arrived: the value
looked accepted, the run ignored it, and the investigation went to the vendor's
documentation. Where a dropped key would change what a run may do, failing at
run time — or not failing at all — instead of at compile time is exactly the
class this compiler refuses.

The set is **per harness**, held in a table audited against the SDK release this
compiler pins, so a vendor's new reach-around arrives with the pin rather than
behind it. The emitted driver subtracts the same names at run time as well; that
is defence in depth for an artifact an older release built, not the place the
rule is made.

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
        workspace: ${REPO_ROOT}
        access: workspace_write
        prompt: Make the smallest change that satisfies the goal.
        output:
          summary: { type: string }
        settings:
          permissionMode: dontAsk
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

## The fix

The diagnostic names the key that states the same bound, where one does. This
one is the case the key `permission_mode:` was added for: the approval mode is a
bound, so it is written on the node.

| a reserved option spelling… | is stated by |
|---|---|
| a permission mode, or the flag one requires | `permission_mode:` |
| a sandbox preset, or sandbox configuration | `access:` |
| a working directory, or roots beside it | `workspace:` |
| a tool set, an allowlist, a permission callback, an MCP server, a subagent | `allow_tools:` |
| the environment | `env:` |
| the output format | `output:` |
| the system prompt, or plan mode's body | `prompt:` |
| the abort signal | `timeout:` |
| the model, or a model setting the harness takes | `model:` |
| which executable the harness is | `harness:` |

Three families answer to no key at all, and the message says so rather than
pointing at the nearest one:

* the **resume** family — `resume`, `continue`, `forkSession`, `sessionId` and
  their neighbours. Harness-native resume is a named exclusion rather than a
  bound: a vendor's session store is machine-local, and this runtime's journal
  is the complete hub state;
* **`fallbackModel`**, which is the failover ladder. A ladder does not reach
  inside a harness run — the harness owns its client and its own retries, and
  this compiler's `retry:` wraps whole runs;
* **`extraArgs`** and `approvalPolicy`: the first is every bound at once, and the
  second is a per-call approval tier belonging to an app server this release does
  not adopt.

## The fix, applied

The same node, with the mode written where a bound belongs.

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
        workspace: ${REPO_ROOT}
        access: workspace_write
        permission_mode: dontAsk
        prompt: Make the smallest change that satisfies the goal.
        output:
          summary: { type: string }
      input: "input.goal"
  edges:
    - { from: start, to: implement }
    - { from: implement, to: end }
```

Grammar: `docs/grammar.md` §8.9, Decisions D140, D146. Topic:
`agent-compose docs agents`.
