# cel

CEL is the only expression language in the spec: non-Turing-complete, sandboxed,
and type-checked at compile time against declared schemas. A CEL value is always
a YAML **string**.

Three expression-ish forms exist and are never interchangeable: CEL, environment
references (`${NAME}`), and durations (`30s`).

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.reviewer:
  model: model.m
  prompt: Review the draft against the goal.
  input:
    goal:  { type: string }
    draft: { type: string }
  output:
    verdict:  { enum: [approve, revise] }
    feedback: { type: string }

state:
  draft:    { type: string, default: "" }
  feedback: { type: string, default: "" }

flow.review:
  inputs:
    goal: { type: string }
  outputs:
    draft: { type: string }
  nodes:
    review:
      agent: agent.reviewer
      input:
        goal:  "input.goal"                       # the flow's own input
        draft: "state.draft"                      # a declared channel
      timeout: 90s                                # a duration, not CEL
  edges:
    - { from: start, to: review }
    - { from: review, to: end,
        when: "review.output.verdict == 'approve' && size(state.feedback) >= 0" }
    - { from: review, to: end, else: true }
```

## What each surface may read

Every surface exposes a fixed set of **root identifiers**. Referencing anything
else is `unknown-root`.

| Surface | Roots in scope | Result |
|---|---|---|
| edge `when:` | `<from>.output` (the source node only), `input`, `state`, `execution` | bool |
| edge `when:` leaving `start` | `input`, `state`, `execution` — `start` has no output | bool |
| `map.over` | `<node>.output` for a node that **dominates** the map node, `input`, `state` | list, path expression only |
| `map` per-item `input:` | the `as:` name (default `item`), `input`, `state`, `execution` | field-typed |
| node `input:` bindings | `input`, `state`, `execution` | field-typed |
| store-op `key`/`value`/`query`/`prefix`/`filter`/`metadata` | `input`, `state`, `execution` | per op |
| inline `http:` node `query`/`body` | `input`, `state`, `execution` | field-typed |
| `tool.*` `http:` binding `query`/`body` | `input` **only** — the tool's own input object | field-typed |
| trigger `input:` values | `payload` | field-typed |
| trigger `session_key` / `callback` / `dedupe_key` | `payload` | string |

Meanings:

- **`input`** — the enclosing flow instance's input object. Inside a `tool.*`
  implementation binding there is no enclosing flow, so `input` is the *tool's*
  own declared input and is the only root there.
- **`state`** — the state object; only declared channels are members.
- **`execution`** — `execution.id`, `execution.session_key` (empty when the
  invocation supplied none), `execution.item_index` (the source-item index of
  the innermost enclosing `map` dispatch; **absent** where no map encloses the
  expression).
- **`payload`** — the trigger payload, shaped per trigger type.
- **`<node>.output`** — a node's output object.

**Node outputs are readable only from edge guards and `map.over`.** Node
configuration never reads another node's output; data that must travel goes
through a state channel. That is what keeps node configs order-independent.

## The supported surface

Standard CEL operators and the standard macro set over those roots: comparisons,
boolean logic, arithmetic, indexing, `in`, `size()`, `has()`,
`startsWith`/`endsWith`/`contains`/`matches`, the four conversions `string()`,
`int()`, `double()`, `bool()`, the strings extension's `list.join()`, and the
comprehension macros `all`, `exists`, `exists_one`, `filter`, `map`. Nothing
else — no other conversion (`uint()`, `timestamp()`), no other extension
function, no custom extension functions.

| function | forms | takes | gives |
|---|---|---|---|
| `size` | `size(x)`, `x.size()` | a string, bytes, a list, a map or an object | `int` |
| `has` | `has(x.f)` | one selection | `bool` |
| `startsWith`, `endsWith`, `contains` | `s.startsWith(p)` | two strings | `bool` |
| `matches` | `s.matches(p)`, `matches(s, p)` | two strings, the second a pattern | `bool` |
| `string` | `string(x)` | any value | `string` |
| `int` | `int(x)` | a number, a string, or a bool | `int` |
| `double` | `double(x)` | a number, a string, or a bool | `double` |
| `bool` | `bool(x)` | a bool, or the string `true` / `false` | `bool` |
| `join` | `l.join(sep)`, `l.join()` | a list of strings; a string separator, or none | `string` |

What the conversions answer:

- **`string()`** of a number is its digits (a double never with an exponent:
  `string(2.0)` is `2`), of a bool `true`/`false`, of `null` `null` — and of a
  **list, a map or an object, its canonical JSON**: keys sorted, no whitespace,
  strings JSON-escaped. That is how a list of objects reaches a model as text:
  a prompt is literal, so the node's binding converts —
  `input: "string(state.findings)"` hands a string-in agent the list.
- **`int()`** truncates a double toward zero, reads a decimal string with an
  optional sign and nothing around it, and makes a bool `1` or `0`; a double
  outside `int64` or a string that is not a decimal integer fails.
- **`double()`** reads a decimal or exponent string (`1.5`, `.5`, `-2e3`) and,
  in any case and with an optional sign, `inf`, `infinity` and `nan` — so
  `double('nan')` is a NaN and `double('-Infinity')` an infinity, not a failed
  read — and makes a bool `1.0` or `0.0`; any other string, whitespace
  included, fails.
- **`bool()`** reads exactly `true` or `false`. `bool(1)` is a compile error —
  CEL gives a number no `bool()`.
- **`l.join(sep)`** joins a list of strings with `sep`, and `l.join()` with
  nothing; a non-string element fails.

A conversion that fails fails the read, the way reading an absent value does
(below).

Expressions must be **type-correct against declared schemas**.
`review.output.verdict == 'aprove'` against `enum: [approve, revise]` is a
compile error, not a silent false.

## Reading something that is not there

Presence is a runtime property. Four things may legitimately be absent:

- a state channel with no `default:` that nothing has written yet;
- a property of a `merge` channel no write has supplied;
- a property a value may omit — one listed in an object's `optional:`, and the
  `value` field a `kv`/`blob` `get` omits on a miss;
- `execution.item_index` where no `map` dispatch encloses the expression.

Reading one **fails the execution**, naming what was absent and the expression
that read it. `has()` is how an expression asks first; on a `get`, the companion
`found` field is the idiomatic test. Type-checking is unaffected — expressions
are checked against declared schemas, never against runtime values.

## Path expressions

`map.over` takes a strict subset: a root identifier followed by field selections
and integer literal indexes.

```
path = root , { "." identifier | "[" integer "]" } ;
```

Legal: `plan.output.tasks`, `state.batches[0].items`. Illegal:
`plan.output.tasks.filter(t, t.ready)`, `a + b`, anything with a call. The
validator has to statically resolve the array's schema to prove the `max_items`
bound and to narrow union variants; a call would make that undecidable.

## What is not CEL

**Environment references.** `${NAME}` is read from the process environment at
start, never evaluated. Two forms: the whole string is one reference
(`api_key: ${ANTHROPIC_API_KEY}`), or references are embedded in text
(`url: "https://${SEARCH_HOST}/v1/search"`). `$${` is the escape for a literal
`${`. Quote a reference written inside `{ … }` or `[ … ]` — YAML forbids those
indicators in a plain scalar there.

Every string surface is in exactly one of three classes. **Env-ref value only**:
`api_key`, `token`, `password`, `url`, `base_url`, `dsn` and the rest of the
secret and connection fields — a literal there is a compile error.
**Interpolable**: `http` urls and headers, the whole `exec:` block, non-secret
provider keys, backend and event-source config values. **No refs at all**:
everything else — prompts, descriptions, every part of a schema, every CEL
expression, model `id`, `settings:` values, `version:`, `imports:` entries, a
trigger's `path:` and `cron:`. An unescaped `${NAME}` in class 3 is an error
rather than text that silently survives, because whoever wrote it expected a
substitution.

Env refs survive **unresolved** into the IR: `validate` and `build` check syntax
only, so an artifact builds anywhere, including CI holding no secrets. Presence
is checked at process start, and `run`/`serve` fail fast before invoking the
graph.

**Durations.** `<integer><ms|s|m|h>`, single segment only and greater than zero:
`250ms`, `30s`, `5m`, `24h`. `1m30s` is a parse error — write `90s`. Used by
`timeout:`, `retry.backoff`, `retry.max_backoff`, `human.timeout`, and a sync
trigger's `timeout:`.

Normative source: `docs/grammar.md` §4, §4.1–4.4
