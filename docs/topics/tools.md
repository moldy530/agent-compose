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
| `exec` \| `http` \| `function` | **exactly one** | the implementation binding |

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

## Runtime built-ins

Four tools the runtime implements — a shell and three file operations — are
attached from an agent's `tools:` list, **one name per entry**, each carrying the
bounds it runs under. They are the boilerplate removed from a `tool.*` an author
could already have hand-rolled with `exec:`, and they move the trust boundary
nowhere: a model holding `builtin.bash` holds arbitrary code execution on the
host running the graph.

```yaml
agent.fixer:
  model: model.smart
  prompt: Fix the failing test, then say what you changed.
  tools:
    - tool.repo_grep
    - builtin.read_file:  { root: "${WORKSPACE}" }
    - builtin.write_file: { root: "${WORKSPACE}" }
    - builtin.list:       { root: "${WORKSPACE}" }
    - builtin.bash:       { root: "${WORKSPACE}", timeout: 30s }
  output:
    summary: { type: string }
```

| Built-in | Arguments | Result |
|---|---|---|
| `builtin.bash` | `command` | `stdout`, `stderr` |
| `builtin.read_file` | `path` | `content` |
| `builtin.write_file` | `path`, `content` | `bytes_written` |
| `builtin.list` | `path` (default `.`), `glob` (default none) | `entries`, `truncated` |

The set is closed. There is no key that grants all four, and no ambient default:
which capabilities an agent holds is meant to be readable off the entries that
hold them.

**`root:` is required on all four.** Every path argument is relative to it, and a
path that *resolves* outside it is refused — resolution, not string comparison,
so `../../etc/passwd` and a symlink pointing out of the tree are both refused,
and a write to a file that does not exist yet is checked through its parent
directory. `builtin.bash` runs with the root as its working directory. The value
is interpolable, so `${WORKSPACE}` is the usual spelling and the directory is a
property of the machine running the graph rather than of the composition.

Two consequences of "resolution" worth knowing before you meet them. A link
whose target does not exist is **refused rather than followed** — there is
nothing to resolve, so where it points cannot be checked, and writing through it
would create the file it names. And a `builtin.list` walk **does not descend into
a symlinked directory**: the link is reported as an entry, without the trailing
`/` a directory gets, because a walk that followed it would answer with paths
outside the root that no path check was ever asked about. Reading through such a
link is a `read_file` call, where the check is asked.

It has to name something, too: an empty `root:` is a compile error, and a
`${WORKSPACE}` that comes back empty fails the call rather than resolving. An
empty path is the runtime's own working directory, so a bound that accepted one
would be the ambient capability these entries exist to refuse — read off no
entry, and different on a developer's machine and a deployment's.

`builtin.list`'s `glob` matches `*` and `?` inside one path segment and `**`
across them — `docs/*.md` is the files directly under `docs/`, `**/*.md` is every
one of them at any depth. `**` matches zero segments as readily as several, so
writing several of them says what one says.

**`timeout:` is required on `builtin.bash`** and illegal on the file tools, which
run no command. What bounds a file tool is the node's own `timeout:`: a listing
over a large tree stops where it is when the node's deadline runs out or the run
is cancelled, rather than finishing a walk nothing is waiting for. `bash`'s own
`timeout:` bounds one command; the node's bounds the whole agent node, tool loop
included, and the two compose. The deadline kills the command's whole **process
group** and ends the call — the shell is almost never
where the work is, and a `npm run build` that outlived its own deadline would go
on writing inside `root:` after the node had already failed. What survives is
what left the group on purpose (`setsid`, `set -m`, a daemon that double-forks),
exactly as it would have from a hand-rolled `exec:` tool; the runtime stops
reading after such a process rather than waiting for it.

**What fails and what bounces.** Arguments the tool's schema refuses go back to
the model, which can call again — a missing `path`, an empty `command`.
Everything else fails the agent node under its `retry:`/`on_error:`, exactly as a
failing `exec:` tool does: a nonzero exit, a command killed at the timeout, a
path that resolved outside the root, a `root:` that names no directory, a host
with no `bash` on `PATH`.

**Containment is the root and the timeout, and nothing more.** The tools run with
the privileges of the process running the graph. Container and syscall isolation,
and any refusal keyed on where a component is deployed, are not in this release
and are not implied by anything on this page.

A built-in call is recorded in the trace like any other tool call — the address
`builtin.bash` as the target, and no result, because
`agent-compose docs trace` keeps tool answers out of that format. The durability
journal keeps the answer in full, which is why a resumed execution consumes a
recorded `bash` instead of running the command a second time.

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
