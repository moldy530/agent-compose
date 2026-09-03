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

```yaml spec
version: "0.1"

provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.p
  id: claude-sonnet-4-5

agent.fixer:
  model: model.smart
  prompt: Fix the failing test, then say what you changed.
  tools:
    - builtin.files
    - builtin.bash
  input:
    goal: { type: string }
  output:
    summary: { type: string }

state:
  summary: { type: string, default: "" }

flow.fix:
  inputs:
    goal: { type: string, min_length: 1 }
  outputs:
    summary: { type: string }
  nodes:
    fix:
      agent: agent.fixer
      input: { goal: "input.goal" }
  edges:
    - { from: start, to: fix }
    - { from: fix, to: end }
```

The **configured** form is a `tool.*` carrying a `builtin:` binding, attached by
its address like any other tool — so one configuration serves every agent that
attaches it. The whole surface, written out:

```yaml spec
version: "0.1"

provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.p
  id: claude-sonnet-4-5

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
  input:
    goal: { type: string }
  output:
    summary: { type: string }

state:
  summary: { type: string, default: "" }

flow.build:
  inputs:
    goal: { type: string, min_length: 1 }
  outputs:
    summary: { type: string }
  nodes:
    build:
      agent: agent.builder
      input: { goal: "input.goal" }
  edges:
    - { from: start, to: build }
    - { from: build, to: end }
```

| Built-in | `builtin:` | What the model calls it | Arguments |
|---|---|---|---|
| `builtin.bash` | `bash` | `bash` | `command`, `restart` |
| `builtin.files` | `files` | `str_replace_based_edit_tool` | `command` (`view`/`create`/`str_replace`/`insert`), `path`, `file_text`, `old_str`, `new_str`, `insert_line` |

The set is closed: any other `builtin:` value is a compile error naming the two.

The **arguments** are the closed set above and nothing else, on every wire, and
that set is **narrower than the vendor tool's own** on the Messages wire: these
go out there as the provider-defined types, and the text editor's `view` takes
an optional `view_range` this runtime does not implement. So a model trained on
that type may send one — a slice of a long file is the ordinary thing to reach
for — and the call is refused with the offending key named, which the model
corrects by calling again without it. **Budget for it**: the bounce costs one
turn of the agent's `max_tool_iterations` (default 8), so an agent whose work is
mostly reading long files wants a turn or two of headroom. Nothing is out of
reach either way — a `view` answers with the whole file, numbered, up to the cap
below.

**The name is the provider's, not the definition key's.** These go out as the
provider-defined tool types, each of which carries a name the wire dictates — so
`tool.sandbox` above is `bash` on the model's side, and a `tool.bash` beside it
would be two tools of one name, which is a `tool-name-collision`.

**A built-in declares no contract.** `input:` and `output:` are compile errors on
one: the arguments and the result are this compiler's, because the model writes
the program rather than filling parameters an author declared. `description:`
stays optional — the compiler writes one, and a composition may sharpen it
("the repository checkout under review").

Which is why an agent's `tools:` list is the only place a built-in is called
from. A `tool.*` binding one is still a tool by address, so a `function:` node or
a `map` dispatch can name it — and both are compile errors: those two surfaces
bind the composition's arguments against a declared `input:` and read a declared
result, and a built-in has neither. Attach it to an agent instead.

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
out of the tree are both refused. Resolution is the whole answer for a
*symbolic* link, which has a target to resolve, and no answer at all for a
**hard** one: a second name for the same file has no target, so a path inside the
workspace really is inside it while the bytes it names may have another name
outside. So the refusal moves to the write — a `create` over a file with more
than one name, and a `str_replace` or `insert` in one, are refused outright,
which is where such a file could have carried an edit out of the directory. A
`view` of one is not, since reading a path inside the workspace is inside the
bound whatever else names it. It matters only for a `workspace:` you named and
something else populated (a checkout, a package manager that links rather than
copies); nothing puts a second name in a fresh per-execution workspace. It is
interpolable, so `${WORK_DIR}` is the usual spelling and the directory is a
property of the machine running the graph rather than of the composition;
written empty it is a compile error, because an empty path is the runtime's own
working directory and a bound nobody wrote is not a bound.

**Written, the directory is yours; omitted, it is the run's.** A binding with no
`workspace:` works in one fresh directory per execution, under the project's data
directory (`.agent-compose/workspaces/<execution id>/`), shared by every built-in
of that execution that took the default — so an agent that writes a file with
`builtin.files` and compiles it with `builtin.bash` finds one directory. It is
removed when the execution settles, and kept while its journal row stays open,
because the generation that resumes a parked run reads what this one wrote. A
`workspace:` you named is never removed.

`timeout:`, `env:` and `inherit_env:` are `builtin.bash`'s alone — `builtin.files`
reads and writes through the runtime and forks nothing, so a command bound and a
child environment there would configure nobody, and each is a compile error.
`env:` takes exactly the `exec:` shape (`agent-compose docs cel` for `${ENV}`
refs), and `inherit_env: false` is the default: a built-in's children see the
variables the binding declared and nothing else — not a credential this process
holds, and not a `PATH` unless you wrote one (`bash` falls back to its own
compiled-in default when it finds none, which is why the shorthand works at all).
`inherit_env: true` is the opt-in, and a declared `env:` still layers over it.

`bash`'s `timeout:` bounds one command; a node's own `timeout:` bounds the whole
agent node, tool loop included, and the two compose.

### What the model gets back

`builtin.bash` answers with `stdout`, `stderr` and `exit_code` — the status only
where a command completed, so a killed one and a bare `restart:` come back
without it — and a `notice` where something happened to the *session*.
`builtin.files` answers with the path
and what the operation did: a `view` comes back with the file's lines numbered
(or a directory's entries), a `create` with the bytes written, an edit with the
line it changed and a few lines around it. **A `create` with no `file_text`
writes an empty file** and says `wrote 0 bytes` — `file_text` is a defaulted
parameter, so a model that sends `""` and one that leaves it out arrive
identically, and a runtime that refused the pair would put `.gitkeep` out of
reach of an agent holding only this tool. `insert` refuses an empty `new_str`
instead, and names the spelling that works: a lone newline, which is the blank
line. Both answers are bounded, and the two bounds differ where it
matters: a command's output is cut in the **middle** — a build that failed says
why in its last lines — and a file view is cut at the end, because a file is read
from line 1. Either way the cut is said rather than silent. A command that prints
more than the runtime will hold is bounded as it arrives, too, so a `cat` of
something enormous costs a truncated answer rather than the process.

The path a `files` call names is the model's, so what it *reads* is bounded the
same way for the same reason. A `view` of a file past that bound answers with the
front of it and says where it stopped; a `str_replace` or an `insert` on one is
**refused**, because an edit writes back what it read and a truncated read would
truncate the file rather than the answer. Work on something that large with
`bash`, which streams rather than holds.

**A shell is a session.** One `bash` child per agent node execution, with its
standard input open: the working directory, the variables and the shell options
carry from one call to the next, so `cd build` and then `make` is one thought
rather than two unrelated commands. A node `retry:` opens a fresh one, exactly as
it restarts the ordinals of grammar §9.4. The model can restart it itself —
`restart: true`, which is what the provider-defined tool's own parameter is for.
Sent alone it runs nothing and answers with a notice and **no** `exit_code`,
because no command completed; sent beside a `command`, that command runs in the
fresh session and answers with its own status. It is never dropped: a runtime
that swallowed it would tell the model a write succeeded that never happened,
and leave a trace saying that command ran on that host.

**What fails and what bounces.** The split of `agent-compose docs agents` reads
one way here that is worth stating, because a built-in has no contract of the
composition's to fail:

- **the model's mistakes come back to the model** — a call with neither a
  `command` nor a `restart`, a `path` that resolves outside the workspace, a
  write to a file that carries a second name, a file that is not there, a
  `str_replace` whose `old_str` matched nothing or matched twice. Each is a
  statement about arguments the model chose, and it can choose again;
- **a command's own outcome is an answer, not a failure** — a nonzero exit comes
  back with the status, and a command that outran the `timeout:` is killed and
  comes back saying so, with what it printed by then and a notice that the
  session went with it. The model decides what to do about both, which is what it
  is there for;
- **the tool being unusable fails the node** — a `workspace:` that names no
  directory, a `${WORKSPACE}` that resolved empty, a host with no `bash` on
  `PATH`. No call can fix those, so `retry:`/`on_error:` decide the run, exactly
  as they do for a failing `exec:` tool.

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
configured one — **and it carries the program the model wrote**: the command and
its exit status, or the file operation and its path, with a sentence about what
an edit changed. That is the one thing the trace format carries for these tools
and for no other, because for every other binding the program is in the spec a
reader already has (`docs/trace.md` §7.4). What stays out is what stays out
everywhere else: the command's output, and what a file holds.

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
