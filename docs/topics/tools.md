# tools

One definition, two usage surfaces. A `tool.*` attached to an agent is
LLM-discovered and nondeterministic; the same `tool.*` reached from a
`function:` node is graph-invoked and deterministic. The definition is shared;
validation is surface-specific. That is the def/use split.

Everything on this page is a tool **this runtime dispatches**. A tool the
*provider* runs — web search, code execution — is a different thing and is
declared on the connection rather than on the agent: see `server_tools:` in
`agent-compose docs models`.

```yaml spec
version: "0.1"

tool.repo_grep:
  description: Search the repository for a pattern and return matching lines.
  input:
    pattern: { type: string, min_length: 1 }
  output:
    matches:
      type: array
      max_items: 100
      items: { type: string }
  exec:
    command: rg
    args: ["--json"]
    cwd: "${REPO_ROOT}"
    expect_exit: [0, 1]        # 1 = no match: data, not a failure

tool.file_ticket:
  description: File a ticket and return its id.
  input:
    title:   { type: string }
    summary: { type: string }
  output:
    id: { type: string }
  http:
    method: POST
    url: "https://${API_HOST}/tickets"
    headers: { authorization: "Bearer ${API_TOKEN}" }
    body:
      title: "input.title"
      body:  "input.summary"

state:
  matches:
    type: array
    max_items: 100
    items: { type: string }

flow.search:
  inputs:
    pattern: { type: string, min_length: 1 }
  outputs:
    matches:
      type: array
      max_items: 100
      items: { type: string }
  nodes:
    look:
      function: tool.repo_grep
      input: { pattern: "input.pattern" }
    notify:
      http:
        method: POST
        url: "https://${HOOKS_HOST}/notify"
        expect_status: [200, 202]
        output:
          status: { type: integer }
      input: { found: "size(state.matches)" }
  edges:
    - { from: start, to: look }
    - { from: look, to: notify }
    - { from: notify, to: end }
```

## The definition

| Key | Required | Notes |
|---|---|---|
| `description` | **yes** | LLM-facing; it is the selection signal |
| `input` | **yes** | parameters; `{}` for a no-argument tool |
| `output` | **yes** | result schema; `{}` for a tool with no result |
| `exec` \| `http` \| `function` \| `module` | **exactly one** | the implementation binding |

Tool definitions carry **no** `retry`/`timeout`/`on_error`. Policy is a property
of a use site — the node — and resolves through the chain in
`agent-compose docs policies`.

**Scope inside a binding.** A tool is a top-level definition with no enclosing
flow, so the CEL in an `http:` binding's `query:`/`body:` sees exactly one root:
`input`, the tool's own declared input object. `state`, `execution`, and
`<node>.output` are not in scope. Everything a tool needs arrives through its
parameters — which is what makes one definition usable from both surfaces.

## `exec`

```yaml
exec:
  command: ripgrep          # argv[0], never a shell line
  args: ["--json", "TODO"]  # literal argv entries: env refs yes, CEL no
  cwd: "${REPO_ROOT}"
  env: { RG_CONFIG: "${RG_CONFIG_PATH}" }
  expect_exit: [0, 1]
```

An object input arrives as environment variables (`UPPER_SNAKE_CASE` of each
field, JSON-encoded for non-scalars); a string input arrives on stdin. The
child's **stdout** is decoded as JSON and validated against `output`, except
when `output` declares **exactly one property and that property is
string-typed** — then trimmed raw stdout binds to it. The count is over the
whole property set, so two properties means JSON.

An `env:` key equal to the upper-snake-cased name of a declared input field is a
collision in which one value would silently win, and is a compile error.

## `http`

```yaml
http:
  method: POST                # explicit; effects are never defaulted
  url: "https://${API_HOST}/tickets"
  headers: { authorization: "Bearer ${API_TOKEN}" }
  body: { title: "input.title" }
  expect_status: [200, 404]
```

Without `body`/`query`, the bound input object is sent as the JSON body
(body-bearing methods) or as query parameters (`GET`/`HEAD`); `body:` is illegal
on `GET`/`HEAD`. The response body decodes the same way `exec`'s stdout does,
with the same single-string-property exception.

## Accepted-outcome lists

`expect_exit` (default `[0]`) and `expect_status` (default: any 2xx) are the
same construct: the set of outcomes that **complete** the node, with everything
else a node error subject to `on_error:`. Both must be **non-empty** and have
**distinct** members — an empty list accepts nothing, and a repeated member
changes nothing, so both are inert keys and both are compile errors.

`expect_exit: [0, 1]` is how a command whose `1` means "no match" becomes
routable data instead of a failure.

## `function`

```yaml
function:
  name: rerank_candidates
```

A host-registered function: the escape hatch. The registry entry's signature is
checked against `input`/`output` at build, a missing registration is a build
error, and any composition using one is flagged as non-portable.

## `module`

The one binding whose implementation lives **inside** the project: a TypeScript
file you write, in the same tree `build` emits.

```yaml
module: ./src/tools/sign.ts     # the scalar form: a path and nothing else
```

```yaml
module:
  path: ./src/tools/sign.ts
  env:
    SIGNING_KEY: "${SIGNING_KEY}"   # what this code may read
  dependencies:
    "@noble/hashes": "1.4.0"        # what it imports, pinned exactly
```

What an `exec:` or an `http:` reaches is nobody's contract. A module is held to
the tool's declared `input:`/`output:` by the **type checker**: codegen emits a
typed interface into `src/modules.ts` from those schemas, and the authored file
has to satisfy it — so a schema change is a type error naming the field that
moved, in the file that has to change. Your file imports that type; `bun run
typecheck` is the gate.

```ts
// src/tools/sign.ts
import type { ToolSignModule } from "../modules.ts";

const toolSign: ToolSignModule = async (input, _context, env) => ({
  signature: await sign(input.payload, env.SIGNING_KEY),
});

export default toolSign;
```

The third argument is the `env:` the binding declared, typed from those names:
`env.SIGNING_KEY` compiles because the YAML lists it, and a variable it does not
list is a type error rather than an `undefined` at run time. It is a value of
this call rather than the process's own environment, so nothing it holds leaks
into a later `exec:` child or into a module running beside it.

**It is a tool, and nothing about running it is special.** The call happens in
the graph's own process, and everything around it is what every other binding
gets: the arguments are parsed against the declared `input:` before your code
sees them, the result against `output:` after; the node's
`retry:`/`timeout:`/`on_error:` chain governs it, so a module that throws is
retried and one that never answers is bounded; the call is journaled, so a
resumed execution does not run it a second time; it appears in the trace as the
tool call it is; and when a **model** called it with arguments the schema
refuses, the refusal goes back to the model rather than ending the node. Only
the binding differs.

Generated code reaches your file in exactly one place — `src/modules.ts`, which
imports it and holds it to that type. Your file may import anything the project
generates.

**Where the file goes, and who owns it.** `build` overwrites exactly the files
it emits and touches nothing else in the output directory, so your
implementation lives in the same tree without a marker comment or a manual
section anywhere. The path is project-relative, ends in `.ts` and not in `.d.ts`
— a declaration file states types and holds no code, and the seam imports your
module for its value — stays inside the project root, is at most 100 bytes long
— the artifact is served as a tar and that is what a header holds — and may
neither be a name `build` writes (`src/graph.ts`, `package.json`, …) nor sit
inside one. Two bindings may not name one file, two spellings of one file, or a
path inside another's: `src/tools/<name>.ts` is the conventional place, and one
file answers to one tool.

**You never type the signature.** `validate` refuses a binding whose file is
missing and names the repair; `agent-compose build <spec>` **scaffolds** it —
typed signature, the contract as a doc comment, a body that throws — writes it
**once**, and never writes that file again. Fill it in, commit it, rebuild: your
bytes are left exactly alone.

**It travels with the artifact.** You edit the file in the project, beside
`main.yml`; a build copies each one the composition references into the output
directory at the same relative path, because that directory is what a worker
fetches and runs (`agent-compose docs targets`). So the file is in
`src/artifact.ts`'s list and inside its content hash: editing an implementation
is a new artifact, and every worker is handed it through the join handshake.
`build --check` compares those copies too — a copy that no longer matches what
you wrote is a build to re-run. A file under `src/` the composition does not
reference ships nowhere.

**Say what it reads and what it imports.** The compiler cannot walk a variable
read inside your TypeScript, and it ships no lockfile beside the
artifact, so `env:` and `dependencies:` are declarations rather than discoveries.
Declared variables reach exactly the processes that can execute the tool — the
same per-placement partition every other tool's do — and dependencies are folded
into the generated `package.json`, which is why they must be **exact** versions:
no `^`, `~`, `>`, `<`, `*` or `x`, no dist-tags, and no `git`/`file`/`npm`/
`workspace` specifiers. Two tools may share a package at one version; two
versions of one package is a compile error naming both.

## Empty result schema

`output: {}` declares a tool with no result: stdout, the response body, or the
return value is **not decoded at all**, and the node contributes nothing to
state and nothing to its edge guards. The failure signal is still observed,
which is what makes a fire-and-forget sink expressible without inventing a
placeholder field.

## Inline `exec:` and `http:` nodes

Use a `tool.*` when the implementation is shared or LLM-facing; use an inline
node for a one-off. The inline surface differs in one way that matters: **its
result is the process or response envelope, not a decoded payload.**

```yaml
run_tests:
  exec:
    command: npm
    args: ["test", "--silent"]
    expect_exit: [0, 1]
    output:
      exit_code: { type: integer }
      stdout:    { type: string }
  input: { pattern: "state.test_filter" }
  on_error: skip
```

`exit_code`, `stdout`, `stderr` (on `exec:`) and `status`, `body` (on `http:`)
are **envelope fields**: declared, they bind directly and are never decoded, and
declaring one with another type is a compile error. Every *other* declared field
is decoded from stdout or the response body as JSON — always. The
single-string-property exception is a `tool.*` rule and does not apply here,
because raw text already has a name on this surface.

Defaults are `{ exit_code: integer, stdout: string }` and
`{ status: integer, body: string }`. Declaring `exit_code:` is a *decoding*
choice and never turns a failure into data — widening `expect_exit` is what does.

An inline node's `query:`/`body:` CEL is **flow**-scoped (`input`, `state`,
`execution`), unlike the same keys inside a `tool.*` binding. Node-level
`input:` builds an ad-hoc object, and declaring both `input:` and the in-block
key that would carry it — `body:` on a body-bearing method, `query:` on
`GET`/`HEAD` — is a compile error rather than a silently ignored key.

## Built-in tools

Two tools the runtime implements — a shell and a file editor — are the fifth
implementation binding, and the only one that does not fix *what runs* at build
time. `exec:`, `http:`, `function:` and `module:` each name a program the author
chose and let the model fill schema-validated parameters; a built-in has the
**model author the program at run time**. That is the trust level `exec:` already
extends to author-arbitrary binaries, extended to the model: an agent holding
`builtin.bash` can run anything the process running the graph can run.

Two spellings. The **shorthand** attaches a built-in under its defaults — a fresh workspace,
a 120s command bound, a scrubbed environment:

```yaml
agent.fixer:
  model: model.smart
  prompt: Fix the failing test, then say what you changed.
  tools:
    - tool.repo_grep
    - builtin.files
    - builtin.bash
  output:
    summary: { type: string }
```

The **configured** form is a `tool.*` carrying a `builtin:` binding, attached by
its address like any other tool — so one configuration serves every agent that
attaches it:

```yaml
tool.sandbox:
  builtin: bash
  workspace: "${WORK_DIR}"
  timeout: 120s
  env:
    PATH: "/usr/bin:/bin"
  inherit_env: false

agent.builder:
  model: model.smart
  prompt: Build the project and report what broke.
  tools:
    - tool.sandbox
  output:
    summary: { type: string }
```

| Built-in | `builtin:` | What the model calls it | Arguments |
|---|---|---|---|
| `builtin.bash` | `bash` | `bash` | `command` |
| `builtin.files` | `files` | `str_replace_based_edit_tool` | `command` (`view`/`create`/`str_replace`/`insert`), `path`, `file_text`, `old_str`, `new_str`, `insert_line` |

The set is closed: any other `builtin:` value is a compile error naming the two.

**The name is the provider's, not the definition key's.** These go out as the
provider-defined tool types, each of which carries a name the wire dictates — so
`tool.sandbox` above is `bash` on the model's side, and a `tool.bash` beside it
would be two tools of one name, which is a `tool-name-collision`.

**A built-in declares no contract.** `input:` and `output:` are compile errors on
one: the arguments and the result are this compiler's, because the model writes
the program rather than filling parameters an author declared. `description:`
stays optional — the compiler writes one, and a composition may sharpen it
("the repository checkout under review").

### The bounds

| Key | Applies to | Default |
|---|---|---|
| `workspace` | both | a fresh per-execution directory, shared by every built-in that took the default |
| `timeout` | `bash` | `120s` |
| `env` | `bash` | nothing — children run scrubbed |
| `inherit_env` | `bash` | `false` |

`workspace:` is where the tool works: `builtin.bash` runs there, and every
`builtin.files` path is relative to it and refused if it resolves outside it —
resolution, not string comparison, so `../../etc/passwd` and a symlink pointing
out of the tree are both refused. It is interpolable, so `${WORK_DIR}` is the
usual spelling and the directory is a property of the machine running the graph
rather than of the composition; written empty it is a compile error, because an
empty path is the runtime's own working directory and a bound nobody wrote is
not a bound.

`timeout:`, `env:` and `inherit_env:` are `builtin.bash`'s alone — `builtin.files`
reads and writes through the runtime and forks nothing, so a command bound and a
child environment there would configure nobody, and each is a compile error.
`env:` takes exactly the `exec:` shape (`agent-compose docs cel` for `${ENV}`
refs), and `inherit_env: false` is the default: a built-in's children see the
variables the binding declared and nothing else.

`bash`'s `timeout:` bounds one command; a node's own `timeout:` bounds the whole
agent node, tool loop included, and the two compose.

**What fails and what bounces.** Arguments the tool's schema refuses go back to
the model, which can call again — a missing `command`, a `path` that resolves
outside the workspace. Everything else fails the agent node under its
`retry:`/`on_error:`, exactly as a failing `exec:` tool does.

**Containment is the workspace and the timeout, and nothing more.** The tools run
with the privileges of the process running the graph. Container and syscall
isolation, and any refusal keyed on where a component is deployed, are not in
this release and are not implied by anything on this page.

Placement is the feature rather than a leak: an agent holding built-in tools
joins the executes-in closure exactly as one holding `exec:` tools does, so a
placed agent runs its model-authored commands on the worker that took its
dispatch — which is what "the machine with the capability" placements are for
(`agent-compose docs targets`).

A built-in call is recorded in the trace like any other tool call, under the
address it was attached by — `builtin.bash` for a shorthand, `tool.sandbox` for a
configured one.

## `function:` nodes

```yaml
file:
  function: tool.file_ticket
  input: { title: "state.headline", summary: "state.detail" }
  writes: { id: ticket_id }
```

`input:` values are CEL, checked field by field against the tool's `input`
schema at compile time. A mismatch here **fails the node** — nothing proposed
this call, so there is nobody to hand a refusal back to.

Normative source: `docs/grammar.md` §5.5, §6, §6.1, §6.2, §8.2, §8.3, §8.4
