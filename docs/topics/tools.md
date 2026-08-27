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

Normative source: `docs/grammar.md` §6, §6.1, §6.2, §8.2, §8.3, §8.4
