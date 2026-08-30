# agent-compose — Grammar Specification

**Spec version:** `0.1`
**Status:** Normative for M0 (parser, resolver, validator)
**Companion artifacts:** [`schemas/agent-compose.schema.json`](../schemas/agent-compose.schema.json) (editor-facing JSON Schema), [`examples/`](../examples) (conformant specs)

This document defines the concrete syntax and static shape of the `agent-compose`
DSL. It is the contract that the parser, resolver, and validator are implemented
against. Where it fixes a shape the PRD left open, the choice is recorded in
[Appendix A — Decisions](#appendix-a--decisions) with its rationale and the PRD
section it must stay consistent with.

**Conformance language.** MUST / MUST NOT / REQUIRED / SHOULD / MAY are used in
the RFC 2119 sense. "Compile error" means `agent-compose validate` rejects the
composition; "parse error" means rejection before resolution. Both are failures
with a diagnostic; the distinction only tells you which pass produces it.

**Layer split.** This document specifies *shape*: what keys exist, what values
they take, which references are well-typed, and what defaults apply. It also
states the *static rules* each construct feeds, but the rules themselves —
exhaustiveness, SCC termination, schema compatibility, reference resolution —
are specified by their PRD sections and implemented in the validator. Every
construct below cross-references the PRD section whose static rules consume it.

---

## Table of contents

1. [Document model](#1-document-model)
2. [Identifiers and typed addresses](#2-identifiers-and-typed-addresses)
3. [The schema language](#3-the-schema-language)
4. [Expressions: CEL, env refs, durations](#4-expressions-cel-env-refs-durations)
5. [Agent definitions](#5-agent-definitions)
6. [Tool definitions](#6-tool-definitions)
7. [Flow definitions](#7-flow-definitions)
8. [Node types](#8-node-types)
9. [Error policy](#9-error-policy)
10. [State](#10-state)
11. [Stores](#11-stores)
12. [Providers and models](#12-providers-and-models)
13. [Triggers](#13-triggers)
14. [Deploy layer](#14-deploy-layer)
15. [Reserved grammar summary](#15-reserved-grammar-summary)
16. [Appendix A — Decisions](#appendix-a--decisions)
17. [Appendix B — Editor integration](#appendix-b--editor-integration)
18. [Appendix C — Construct reference card](#appendix-c--construct-reference-card)

---

## 1. Document model

### 1.1 YAML profile

A spec file is a YAML 1.2 file with these restrictions (PRD 5.1, G3):

- Exactly **one document** per file. A `---` document separator introducing a
  second document is a parse error.
- The document root MUST be a **mapping**.
- Keys MUST be strings. Non-string keys (`1:`, `[a]:`, `? complex`) are a parse error.
- Duplicate keys in the same mapping are a parse error (never last-wins).
- **Anchors and aliases** (`&name` / `*name`) are permitted and expanded by the
  parser before any spec-level processing.
- **Merge keys** (`<<:`) are NOT supported (they are a YAML 1.1 extension outside
  YAML 1.2 core). Use anchors/aliases or explicit repetition.
- **Tags** (`!!str`, `!custom`) are NOT supported. The spec never carries
  language-level type directives (PRD 5.5: the spec contains no executable code).
- Files use the `.yml` or `.yaml` extension. `.yml` is the convention in this
  repository's examples.
- Encoding is UTF-8 without BOM.

Everything below describes the mapping structure inside such a file.

### 1.2 Document kinds

There are exactly two document kinds. They are disjoint: a file is one or the
other, never both (Decision [D3](#d3-spec-files-and-deploy-files-are-disjoint-document-kinds)).

| Kind | Selected by | May contain |
|---|---|---|
| **Spec file** | reachable from the entrypoint's `imports:`, or being the entrypoint | `version`, `imports` (entrypoint only), `defaults`, `state`, `triggers`, typed-address definition keys |
| **Deploy file** | `--target <name>` → `deploy/<name>.yml` | `version`, `hub`, `placements`, `storage_backends`, `event_sources` |

A spec file that declares `hub`, `placements`, `storage_backends`, or
`event_sources` is a compile error, and a deploy file that declares definitions,
`imports`, `state`, `triggers`, or `defaults` is a compile error. This is the
mechanical enforcement of the PRD 5.8 per-target invariant: only the deploy layer
forks per environment.

The **entrypoint** is the spec file named on the command line (conventionally
`main.yml`). It is the only file that MAY declare `imports:`.

### 1.3 The `version` field

```yaml
version: "0.1"
```

- Type: string. The value MUST be quoted (unquoted `0.1` is a YAML float).
- REQUIRED in the entrypoint and in every deploy file.
- OPTIONAL in imported spec files; when present it MUST be byte-identical to the
  entrypoint's value, otherwise a compile error naming both files.
- The accepted set is exactly the compiler build's `SUPPORTED_SPEC_VERSIONS`
  (`crates/compose-core/src/lib.rs`). For this build: `["0.1"]`.
- A version outside the supported set is refused, and told the set this build
  accepts; old syntax is never silently reinterpreted. The codemod PRD §9.5
  commits to is a future verb, and is not named by any message this build emits.

### 1.4 `imports`

```yaml
imports:
  - providers.yml
  - models.yml
  - agents/researcher.yml
  - flows/review_loop.yml
  - triggers.yml
```

- Type: array of strings. MAY be empty or omitted (a single-file project).
- Each entry is a **relative path** resolved against the directory containing the
  entrypoint (the *project root*).
- Absolute paths, URLs, and glob/wildcard patterns are parse errors. There is **no
  directory scanning** — "what is in this graph" is the import list (PRD 5.1).
- **Path form** (Decision [D80](#d80-the-published-schemas-per-file-bounds-are-grammar-rules)):
  segments separated by `/`; each segment is `.`, `..`, or a name matching
  `[A-Za-z0-9_][A-Za-z0-9_.-]*`; the last segment ends in `.yml` or `.yaml`
  (§1.1). No backslashes, no whitespace, no leading `/`. One portable spelling
  keeps a path identical in the IR, on a command line, and in a diagnostic on
  every host.
- A path MAY contain `..`, but every prefix of the path MUST stay inside the
  project root during resolution — the root is a fence, not a floor. A path
  that climbs out and returns (`../<root-dir-name>/models.yml`) is refused
  even though its final resolution lands inside the root: whether such a path
  re-enters the same project depends on the checkout's parent directory
  layout, which the spec cannot see. Write the in-root spelling instead
  (`models.yml`).
- Entries MUST be unique after path normalization, and MUST NOT name the
  entrypoint itself.
- Imports are **not transitive**: an imported file that declares `imports:` is a
  compile error (Decision [D1](#d1-imports-are-entrypoint-only-and-non-transitive)).
- `deploy/*.yml` files MUST NOT appear in `imports:`; they are selected by
  `--target`.
- Import order does not affect semantics. Definitions are order-independent and
  the resolver emits IR in a stable canonical order (PRD 5.12).

### 1.5 Section placement

| Section | Entrypoint | Imported spec file | Deploy file | Cardinality |
|---|---|---|---|---|
| `version` | REQUIRED | optional (must match) | REQUIRED | — |
| `imports` | optional | **illegal** | illegal | ≤ 1 |
| `defaults` | allowed | allowed | illegal | ≤ 1 per composition |
| `state` | allowed | allowed | illegal | ≤ 1 per composition |
| `triggers` | allowed | allowed | illegal | ≤ 1 per composition |
| `agent.*` `tool.*` `flow.*` `store.*` `provider.*` `model.*` | allowed | allowed | illegal | 1 per address |
| `hub` | illegal | illegal | allowed | ≤ 1 per target |
| `placements` | illegal | illegal | allowed | ≤ 1 per target |
| `storage_backends` | illegal | illegal | allowed | ≤ 1 per target |
| `event_sources` | illegal | illegal | allowed | ≤ 1 per target |

"≤ 1 per composition" means the section may appear in **at most one file** of the
resolved composition; declaring `state:` in two imported files is a compile error
naming both (Decision [D2](#d2-singleton-sections-are-declared-in-exactly-one-file)).
Definition keys are globally unique: the same address defined in two files is a
compile error naming both.

Any other top-level key is a compile error ("unknown top-level key").

### 1.6 Conventional project layout

The layout is a convention, not a rule — the import list is authoritative.

```
main.yml            # version, imports, state, defaults
providers.yml       # provider.* definitions
models.yml          # model.* definitions
agents/*.yml        # agent.* definitions
tools/*.yml         # tool.* definitions
flows/*.yml         # flow.* definitions
stores/*.yml        # store.* definitions
triggers.yml        # triggers section
deploy/local.yml    # the built-in target (not imported); no storage_backends (§14)
deploy/staging.yml  # deploy target (not imported)
```

A file MAY hold several definitions and MAY mix namespaces; one definition per
file is the recommended default for reviewability.

### 1.7 Compact whole-project example

```yaml
# main.yml
version: "0.1"
imports:
  - providers.yml
  - models.yml
  - agents/reviewer.yml
  - flows/review_loop.yml
  - triggers.yml

defaults:
  timeout: 60s
  on_error: fail

state:
  draft:    { type: string, default: "" }
  feedback: { type: string, default: "" }
```

The full, runnable version of this project is
[`examples/review-loop/`](../examples/review-loop); a second project exercising
fan-out, stores, human nodes, and reserved grammar is
[`examples/triage-fanout/`](../examples/triage-fanout).

---

## 2. Identifiers and typed addresses

### 2.1 Identifier grammar

```
identifier  = lower , { lower | digit | "_" } ;
lower       = "a" … "z" ;
digit       = "0" … "9" ;
```

- Length 1–64 characters. Case-sensitive; only lowercase is legal.
- Used for: definition names, flow-local node ids, state channel names, schema
  field names, discriminator variant tags, trigger names, backend aliases, event
  source names, and the `as:` binding on `map`.
- Rationale: one identifier class means one error message, and lowercase
  snake_case survives every codegen target without mangling (PRD 5.12).

### 2.2 Namespaces and definition keys

A **definition key** is a top-level mapping key of the form
`<namespace>.<identifier>`:

| Namespace | Defines | PRD |
|---|---|---|
| `agent.` | an LLM call with structured output | 5.2, 5.5 |
| `tool.` | a callable implementation (LLM-facing and/or graph-invoked) | 5.5 |
| `flow.` | a subgraph module | 5.1 |
| `store.` | durable attachable storage | 5.8 |
| `provider.` | an inference connection | 5.9 |
| `model.` | a model binding or route | 5.9 |

```yaml
agent.reviewer:      # definition
  ...
```

The address `agent.reviewer` is global across the composition (multi-file is
authoring UX; the resolver flattens to one IR document — PRD 5.1). Namespaces are
disjoint: `agent.x` and `tool.x` may coexist.

### 2.3 References and type checking

A **reference** is the same `namespace.name` string used as a *value*. Every
reference position accepts a fixed set of namespaces; anything else is a compile
error of the form "expected an `agent.*` reference, found `tool.web_search`"
(PRD 5.1).

| Position | Accepts | Notes |
|---|---|---|
| `agent.<a>.model` | `model.*` | REQUIRED |
| `agent.<a>.tools[]` | `tool.*`, `flow.*` | flow-as-tool equivalence (PRD 5.1) |
| `agent.<a>.stores[]` | `store.*` | |
| `model.<m>.provider` | `provider.*` | direct models only |
| `model.<m>.route[]` | `model.*` | direct models only, no nested routes |
| `store.<s>.embed.provider` | `provider.*` | REQUIRED; which connection computes the vectors (§11.2); `vector` stores only |
| node `agent:` | `agent.*` | |
| node `function:` | `tool.*` | the def/use split of PRD 5.5 |
| node `flow:` | `flow.*` | no recursion |
| node `store:` | `store.*` | |
| `map.node`, `map.routes.<tag>.node`, `map.default.node` | `agent.*`, `tool.*`, `flow.*` | dispatch targets are components, not flow-local ids |
| `on_error.fallback` | flow-local node id, `end` | control transfer stays inside one flow (§2.4) |
| `human.on_timeout` | flow-local node id, `end` | same positions as `on_error.fallback` |
| edge `from` / `to` | flow-local node id, `start`, `end` | |
| `triggers.<t>.flow` | `flow.*` | |
| `placements` keys | `agent.*`, `tool.*`, `flow.*` | deploy layer |

### 2.4 Flow-local node ids and pseudo-nodes

Node ids are identifiers scoped to their flow; two flows may both have a node
`review`. `start` and `end` are **pseudo-nodes**:

- `start` — the flow's entry. It has no output; one or more edges MUST leave it,
  and nothing MAY target it. `start` is legal **only** as an edge `from`. Edges
  leaving `start` MAY carry `when:`/`else:`, and at least one of them MUST be
  unconditional or carry `else: true` so an execution always has a first step
  (§7.6.3).
- `end` — the flow's exit. No edge MAY leave it. Reaching `end` **retires the
  branch that reached it**; the flow instance finishes at quiescence, when every
  live branch has retired, and materializes its `outputs` then (§7.6.3, §7.5).
  `end` is legal as an edge `to` and in the two **control-transfer positions** —
  `on_error.fallback` (§9.2) and `human.on_timeout` (§8.7) — where it means
  "this branch is done", never "stop the run". The two control-transfer
  positions accept exactly the same targets; nothing else accepts a pseudo-node.

A node MUST NOT be named `start` or `end`; every node MUST have at least one
outgoing edge (§7.6.3) and MUST be reachable from `start` (§7.8).

### 2.5 Reserved names

The **reserved root names** are the seven identifiers that already have a fixed
meaning inside an expression (§4.1, §10.4):

`input`, `state`, `execution`, `item`, `messages`, `output`, `payload`

"Reserved root name" is a term of art for this list — it is what D74 and the
published schema's `reservedRootName` denote, not a claim that all seven are
roots. Five are: `input`, `state`, `execution`, `payload`, and `item`, the last
being the default name of a `map`'s per-item binding (§8.6). The other two are
on the list for their own reasons, and they are exactly the two a reader will
not find in §4.1's table of roots:

- **`messages`** is the implicit conversation-history channel (§10.4). Reserving
  it is what puts §10.4's "MUST NOT be declared in `state:`" into this one list
  rather than leaving it as a lone prohibition three sections away.
- **`output`** is the fixed **selector** of a node-output path: the second
  segment of every `<node>.output.<field>` an edge guard or a `map.over` reads
  (§4.1). No surface exposes a bare `output` root, and that is the point —
  reserving it keeps the token to one meaning wherever an expression is written.
  A node id `output` yields `output.output.verdict`, where the same word is the
  node and then the selector; a binding `as: output` puts
  `over: plan.output.tasks` and `input: { x: "output.summary" }` in one `map:`
  block with `output` naming a selector on one line and the item on the next.
  Neither is ambiguous to a *parser* — which is why this member is reserved for
  legibility and for the list staying one rule with one diagnostic
  ([D5](#d5-one-identifier-class-lowercase-snake_case),
  [D74](#d74-reserved-roots-may-not-be-shadowed-by-node-ids-or-item-bindings)),
  not because a reading could go two ways.

A reserved root name MUST NOT be used as (Decision
[D74](#d74-reserved-roots-may-not-be-shadowed-by-node-ids-or-item-bindings)):

| Position | Rule |
|---|---|
| a **state channel** name | illegal — for the five roots the channel would shadow the root in every expression; `messages` names the implicit channel that already exists (§10.4); `output` is the selector, above |
| a **flow-local node id** | illegal — a node id is the root of `<node>.output` in edge guards and `map.over` (§4.1), so a node named `input` makes `input.output.x` ambiguous with the flow input object |
| a `map` **`as:`** binding | illegal, **except `item`**, which is that binding's own default name (§8.6) |

`start` and `end` are additionally reserved as node ids. Definition names have no
reserved words beyond the identifier grammar: `agent.state` and `agent.output`
are both fine, because a namespaced address is never a bare token in expression
position.

Shadowing is a compile error rather than a precedence rule: there is no reading
of `input.output.verdict` that is obviously right when a node is named `input`,
and a document that had to name a winner would be teaching a trap (PRD G3).
Where no parse could go two ways — `messages` and `output` — the same refusal
is what keeps the list one rule with one message instead of seven names with two
behaviors.

---

## 3. The schema language

Schemas are the load-bearing construct: they make routing decidable (PRD 5.3),
fan-out bounded (5.6), edges serializable (5.7/5.10), and store ops checkable
(5.8). The DSL uses a **closed subset of JSON Schema**, written in a compact
shorthand. Nothing outside this subset is legal; unsupported JSON Schema keywords
(`anyOf`, `allOf`, `$ref`, `patternProperties`, `if/then`, `not`, …) are compile
errors.

### 3.1 Field maps

Every declaration surface (`output:`, `input:`, `inputs:`, `outputs:`,
`state:`, `value_schema:`, `metadata_schema:`) takes a **field map**: a mapping
from field name (identifier, §2.1) to **type node**.

```yaml
output:
  verdict:  { enum: [approve, revise] }
  feedback: { type: string }
```

A field map denotes a **closed object**: exactly the declared properties, no
others (§3.6). An empty field map `{}` denotes an object with no properties (a
no-argument tool).

Field maps appear only at declaration surfaces. Nested objects are written
explicitly (§3.4) — a bare nested mapping is not a shorthand for an object
(Decision [D6](#d6-field-map-shorthand-only-at-declaration-surfaces)).

### 3.2 Type nodes

A type node is a mapping that carries exactly one **discriminating key**:
`type`, `enum`, or `discriminator`. Which one determines its form:

| Form | Discriminating key | §  |
|---|---|---|
| scalar | `type: string \| integer \| number \| boolean` | 3.3 |
| object | `type: object` | 3.4 |
| array | `type: array` | 3.5 |
| enum | `enum: [...]` | 3.3 |
| discriminated union | `discriminator: <field>` | 3.7 |

Every form additionally accepts `description` (string; surfaces to the model in
structured-output schemas and to editors).

#### 3.3 Scalars and enums

```yaml
title:      { type: string, min_length: 1, max_length: 200 }
confidence: { type: number, minimum: 0, maximum: 1 }
attempts:   { type: integer, minimum: 0, multiple_of: 1 }
urgent:     { type: boolean }
verdict:    { enum: [approve, revise, escalate] }
started_at: { type: string, format: date-time }
```

| Key | Applies to | Type | Notes |
|---|---|---|---|
| `min_length` / `max_length` | string | integer ≥ 0 | |
| `pattern` | string | string | **RE2 syntax** — no backreferences, no lookaround (Decision [D12](#d12-pattern-is-re2-format-is-a-closed-list)) |
| `format` | string | enum | one of `date-time`, `date`, `time`, `duration`, `email`, `uri`, `uuid`, `hostname`, `ipv4`, `ipv6` |
| `minimum` / `maximum` | integer, number | number | inclusive |
| `exclusive_minimum` / `exclusive_maximum` | integer, number | number | |
| `multiple_of` | integer, number | number > 0 | |
| `default` | scalar, enum, object, array | literal validating against the type node | input surfaces and state channels only; never on a union (§3.6) |

`enum` takes a non-empty array of **unique string literals** (identifier-like
values are conventional but any non-empty string is legal). `enum` implies
`type: string`; writing a `type:` alongside `enum:` is an error
(Decision [D9](#d9-enums-are-string-only)). Enum-typed fields in an agent's
output are what routing exhaustiveness is computed over (PRD 5.3).

#### 3.4 Objects

```yaml
author:
  type: object
  properties:
    name:  { type: string }
    email: { type: string, format: email }
  optional: [email]
```

- `properties` — REQUIRED, a field map (may be `{}`).
- `optional` — array of property names that are not required. Every declared
  property is REQUIRED unless listed here
  (Decision [D7](#d7-properties-are-required-by-default-optional-lists-the-exceptions)).
  Every name in the array MUST be a property the same object declares; an entry
  naming something else is a compile error listing the unknown names and the
  declared ones (Decision
  [D89](#d89-optional-entries-must-name-declared-properties)). The published
  schema cannot check this — JSON Schema constrains an array's items without
  reference to a sibling object's keys (Appendix B) — so it is a validator rule,
  and it is what keeps `optional: [emial]` a diagnostic instead of a silently
  still-required `email`. A property listed here may be **absent from a value**
  at run time; reading an absent property fails the execution (§4.1, Decision
  [D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)).
- Objects are **closed**. There is no `additional_properties` knob; unknown keys
  in an instance are invalid (Decision [D8](#d8-objects-are-closed)).
- Nesting depth is limited to **8** levels (declaration surface counts as 1).

#### 3.5 Arrays, and the result surfaces

```yaml
tasks:
  type: array
  max_items: 20
  items: { type: string }
```

- `items` — REQUIRED; a type node (§3.2) or a discriminated union (§3.7).
- `max_items` — integer in `1..=10000`.
- `min_items` — integer ≥ 0, ≤ `max_items`.
- `unique_items` — boolean, default `false`.

**The result surfaces.** These declaration surfaces are **result schemas** — the
places a node's, module's, or store's *result* is declared:

`agent.output`, `tool.output`, `flow.outputs`, `human.output`, an inline
`exec:`/`http:` node's `output` (§8.2, §8.3), `store.value_schema`,
`store.metadata_schema`.

This one list is what §3.5's `max_items` rule and §3.6's `default:` prohibition
are both stated over, and the inline node outputs are in it (Decision
[D96](#d96-an-inline-exechttp-nodes-output-is-a-result-schema)). Every other
declaration surface — `agent.input`, `tool.input`, `flow.inputs`, `human.input`,
and `state` channels — is an **input surface**.

**`max_items` is REQUIRED** when the array (Decision [D10](#d10-max_items-is-required-on-result-schemas-and-fanned-out-arrays)):

1. appears anywhere inside a **result schema** — any surface named above.
   Model-produced cardinality must be bounded, and structured-output validation
   rejects longer arrays so the model *cannot* return more (PRD 5.6); the same
   bound keeps every edge payload finite and serializable (PRD 5.7, 5.10),
   which is why the two inline node surfaces are in the list even though
   nothing they decode is model-produced; or
2. is the schema a `map.over` path resolves to — unbounded fan-out is a compile
   error (PRD 5.6).

It is OPTIONAL on every **input surface** (`agent.input`, `tool.input`,
`flow.inputs`, `human.input`, and state channels), where the value is not
model-produced and its bound comes from whatever produced it.

#### 3.6 `default`, requiredness, and surface rules

- `default:` is legal on **scalar, enum, object, and array** type nodes at every
  **input surface** (`agent.input`, `flow.inputs`, `tool.input`, `human.input`,
  and state channels), and nested inside one; on a channel it is the channel's
  initial value. The literal MUST validate against the type node it sits on — an
  object default supplies every required property, an array default is an array
  of the `items:` type (Decision
  [D77](#d77-default-is-legal-on-every-type-node-form-except-a-union)).
- `default:` is ILLEGAL on a **discriminated union** (§3.7) at every surface. A
  union default would have to name a variant, and manufacturing a discriminator
  tag is the same silent routing decision the result-surface rule below refuses.
- `default:` is ILLEGAL on every **result surface** — the ones §3.5 names,
  which include an inline `exec:`/`http:` node's `output` (D96). A defaulted
  result would silently manufacture routing values (PRD 5.3): a field the model
  never emitted, or — on an inline node — a field the process never printed and
  the response never carried.
- A property with a `default:` is implicitly optional at its surface.

#### 3.7 Discriminated unions

```yaml
findings:
  type: array
  max_items: 50
  items:
    discriminator: kind
    variants:
      auto_fixable: { file: { type: string }, patch_hint: { type: string } }
      needs_human:  { summary: { type: string }, severity: { enum: [low, high, critical] } }
```

- `discriminator` — REQUIRED, an identifier naming the tag field.
- `variants` — REQUIRED, a mapping from **variant tag** (identifier) to a field
  map. At least **2** variants (a one-variant union is an object —
  Decision [D11](#d11-discriminated-union-shape)).
- A variant's field map MUST NOT declare the discriminator field; the compiler
  synthesizes it as a string constant equal to the tag. Instances therefore carry
  `kind: auto_fixable` and the model's contract stays legible in its schema.
- Legal positions: as `items:` of an array, and as the type of a property in a
  field map. A union MUST NOT be the top level of a declaration surface (the top
  level is always a field map, so routing fields have names).
- Consumed by `map.route_by` for heterogeneous fan-out with per-variant schema
  narrowing and exhaustiveness (PRD 5.6).

### 3.8 Mapping to JSON Schema and to codegen types

The compiler lowers every schema to JSON Schema draft 2020-12 in the flat IR, and
codegen lowers that to the target language's validation types (Zod for the
TypeScript/LangGraph JS target).

| DSL | JSON Schema 2020-12 | Zod |
|---|---|---|
| field map `{a: T, b: U}` | `{"type":"object","properties":{...},"required":["a","b"],"additionalProperties":false}` | `z.object({...}).strict()` |
| `optional: [b]` | `b` omitted from `required` | `.optional()` on `b` |
| `{type: string, pattern: p}` | `{"type":"string","pattern":"p"}` | `z.string().regex(/p/)` |
| `{enum: [x, y]}` | `{"type":"string","enum":["x","y"]}` | `z.enum(["x","y"])` |
| `{type: integer, minimum: 0}` | `{"type":"integer","minimum":0}` | `z.number().int().min(0)` |
| `{type: array, items: T, max_items: n}` | `{"type":"array","items":T,"maxItems":n}` | `z.array(T).max(n)` |
| `{discriminator: k, variants: {...}}` | `{"oneOf":[{...,"properties":{"k":{"const":"tag"},...}}]}` | `z.discriminatedUnion("k", [...])` |
| `default: v` | `{"default":v}` | `.default(v)` |

Key-name mapping is mechanical snake_case → camelCase (`max_items` → `maxItems`,
`min_length` → `minLength`, `exclusive_minimum` → `exclusiveMinimum`,
`multiple_of` → `multipleOf`, `unique_items` → `uniqueItems`). The DSL uses
snake_case everywhere for one consistent key style.

### 3.9 Where schemas appear

The **Class** column is the §3.5 split, and it is the whole of what a surface's
class decides: a result surface requires `max_items` on every array inside it
and refuses `default:`; an input surface does the opposite on both counts.

| Surface | Class | Required | `{}` legal? |
|---|---|---|---|
| `agent.<a>.output` | field map — **result** | REQUIRED (PRD 5.2) | no — ≥ 1 property (§5.1) |
| `agent.<a>.input` | field map — input | optional; default string-in (§5.3) | no — omit it instead (D62) |
| `tool.<t>.input` | field map — input | REQUIRED | yes (no-argument tool) |
| `tool.<t>.output` | field map — **result** | REQUIRED | yes (no result) |
| `flow.<f>.inputs` | field map — input | optional (default: no inputs) | yes |
| `flow.<f>.outputs` | field map — **result** | REQUIRED | yes (no result) |
| `human.input` | field map — input | REQUIRED | yes |
| `human.output` | field map — **result** | REQUIRED | yes |
| inline `exec:` / `http:` node `output` | field map — **result** (D96) | optional (kind default, §8.2/§8.3) | yes |
| `state` channels | field map — input, + `reduce` | optional section | yes (no channels) |
| `store.<s>.value_schema` / `metadata_schema` | field map — **result** | per kind (§11) | yes |

**Emptiness.** An empty field map `{}` is a closed object with no properties
(§3.1) and is legal at every surface above except the two agent surfaces:
`agent.output` MUST declare ≥ 1 property because it is what routing reads
(PRD 5.2, 5.3), and `agent.input: {}` is a compile error because omitting
`input:` is the way to say "no declared input"
(Decision [D62](#d62-an-agents-declared-input-has-at-least-one-field)). A
result surface with no fields is a real contract — a tool whose effect is its
only purpose, a flow that only writes stores — and it stays type-checkable: such
a node contributes nothing to state and nothing routable to its edge guards.

---

## 4. Expressions: CEL, env refs, durations

Three expression-ish value forms exist. They are syntactically distinct and never
interchangeable.

### 4.1 CEL expressions

CEL (Common Expression Language) is the only expression language in the spec:
non-Turing-complete, sandboxed, type-checked at parse time (PRD 5.5). A CEL value
is always a **YAML string**.

**Surfaces and scopes.** Each surface exposes a fixed set of root identifiers.
Referencing anything else is a compile error.

| Surface | Roots in scope | Result type |
|---|---|---|
| edge `when:` | `<from>.output` (the edge's source node only), `input`, `state`, `execution` | bool |
| edge `when:` on an edge leaving `start` | `input`, `state`, `execution` — `start` has no output, so there is no `<from>.output` root (§2.4) | bool |
| `map.over` | `<node>.output` for any node that **dominates** the map node (§8.6 rule 11), `input`, `state` | list (path expression only, §4.2) |
| `map.input` / `map.routes.<tag>.input` / `map.default.input` values | `<as-name>` (the item), `input`, `state`, `execution` | field-typed, or a single scalar for a string-in target (§8.6 rule 12) |
| node `input:` bindings | `input`, `state`, `execution` | field-typed |
| store-op `key`, `value`, `query`, `prefix`, `filter`, `metadata` values | `input`, `state`, `execution` | per §11.4 |
| inline `http` node `query` / `body` values | `input`, `state`, `execution` | field-typed |
| `tool.<t>` `http:` binding `query` / `body` values | `input` **only** (the tool's own input object) | field-typed |
| trigger `input:` values | `payload` | field-typed |
| trigger `session_key`, `callback`, `dedupe_key` | `payload` | string |

Root identifier meanings:

- **`input`** — the input object of the construct the expression is written
  inside. On every *flow-scoped* surface (edge guards, node `input:` bindings,
  an inline `http` node's `query`/`body` values, `map` per-item bindings,
  store-op parameters) that is the enclosing flow instance's input object
  (`input.goal`).
  On a **`tool.*` implementation binding** — the one surface that has no
  enclosing flow — it is the tool's own declared `input:` object, and it is the
  only root in scope there (§6.1, Decision
  [D65](#d65-a-tool-implementation-binding-sees-only-the-tools-own-input)).
- **`state`** — the state object; only declared channels (§10) are members.
- **`execution`** — run metadata: `execution.id` (string), `execution.session_key`
  (string; empty when the invocation supplied no session key — a trigger with no
  `session_key:`, or a CLI run without `--session`, §13.2),
  `execution.item_index` (integer; the source-item index of the **innermost**
  enclosing `map` dispatch — present in that map's own per-item expressions and
  everywhere inside the instance it dispatches, and **absent** anywhere else,
  which is the fourth case of the absent-value rule below).
- **`payload`** — the trigger payload; shape per trigger type (§13).
- **`<node>.output`** — a node's node-scoped output object (PRD 5.7 tier 1).
- **`<as-name>`** — the per-item binding of a `map` (`item` unless renamed with
  `as:`, §8.6). It is a root only inside that map's per-item expressions.

The per-item **index** has exactly one spelling, `execution.item_index`; there is
no bare `item_index` root. One spelling means the reserved-name list (§2.5) needs
only `item`, and a channel can never shadow the index.

**Node outputs are only readable from edge guards and `map.over`**
(Decision [D42](#d42-node-outputs-are-readable-only-from-edge-guards-and-mapover)).
Node configuration never reads another node's output; data that must travel goes
through a state channel (§10), which keeps node configs order-independent and
placement-agnostic (PRD 5.10).

**Supported CEL surface.** Standard CEL operators and the standard macro/function
set over the exposed roots: comparisons, boolean logic, arithmetic, indexing,
`in`, `size()`, `has()`, `startsWith`/`endsWith`/`contains`/`matches`, and the
comprehension macros (`all`, `exists`, `exists_one`, `filter`, `map`). No custom
extension functions in v0. Expressions MUST be type-correct against the declared
schemas; `review.output.verdict == 'aprove'` against
`enum: [approve, revise]` is a compile error, not a silent false.

```yaml
when: "review.output.verdict == 'revise' && size(state.feedback) > 0"
```

**Reading a value that is not there.** Presence is a runtime property, and four
things may legitimately be absent when an expression reads them — three the
composition declares, and one this section does:

- a state channel with no `default:` that nothing has written yet (§10.1,
  [D78](#d78-channel-initial-values-and-reading-an-unset-channel));
- a property of a `merge` channel that no write has supplied (§10.1,
  [D101](#d101-a-merge-channels-properties-are-unset-until-supplied));
- a property a *value* may legally omit: one listed in an object's `optional:`
  (§3.4), and the `value` field a `kv` or `blob` `get` omits on a miss — the one
  top-level field of any result schema in this grammar that its own node may not
  return (§11.4);
- **`execution.item_index` where no `map` dispatch encloses the expression**
  (above, §8.6). This one is a property of the *site*, not of the flow: the same
  `flow.*` may be a dispatch target at one instantiation and a directly
  instantiated subflow at another, and every flow is additionally runnable as a
  root instance from the CLI (§13), so no static rule can promise the index is
  there (Decision
  [D115](#d115-executionitem_index-is-absent-where-no-map-dispatch-encloses-the-expression)).

Reading one **fails the execution**, naming what was absent and the expression
that read it (Decision
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)).
`has()` is how an expression asks first, and on a `get` the companion `found`
field is the idiomatic test. Type-checking is unaffected throughout:
expressions are checked against **declared** schemas, never against runtime
values, so `load_prefs.output.value.theme` is a `string` wherever it is legal to
write — exactly as §10.1 says of an unset channel and of a `merge` channel's
properties.

### 4.2 Path expressions

`map.over` takes a **path expression**, a strict subset of CEL: a root identifier
followed by field selections and integer literal indexes.

```
path = root , { "." identifier | "[" integer "]" } ;
```

Legal: `plan.output.tasks`, `state.batches[0].items`.
Illegal: `plan.output.tasks.filter(t, t.ready)`, `a + b`, anything with a call.

Rationale: the validator must statically resolve the array's schema to prove
`max_items` bounding and to narrow union variants (PRD 5.6). A call would make
that undecidable (Decision [D43](#d43-mapover-is-a-path-expression)).

### 4.3 Environment references

The spec never contains credentials (PRD 5.8, 5.9). Two forms:

```
env-ref-value      = "${" , env-name , "}" ;              # the whole string
interpolated       = { text | env-ref } ;                  # embedded refs
env-name           = ( "A"…"Z" | "_" ) , { "A"…"Z" | "0"…"9" | "_" } ;
```

- **Env-ref value form** — the entire string is exactly one reference:
  `api_key: ${ANTHROPIC_API_KEY}`. Matches `^\$\{[A-Z_][A-Z0-9_]*\}$`.
- **Interpolated form** — references embedded in text:
  `url: "https://${SEARCH_HOST}/v1/search"`.
- `$${` is an escape producing a literal `${`. It is recognized wherever a
  `${…}` token is, which includes the surfaces where env refs are illegal: a
  prompt that must contain the six characters `${FOO}` writes `$${FOO}`.

**Quoting.** A `${…}` reference is only *bare*-writable in YAML block context
(`api_key: ${ANTHROPIC_API_KEY}`). YAML forbids the indicators `{`, `}`, `[`,
`]`, and `,` inside a plain scalar in **flow** context, so a reference written
inside a `{ … }` mapping or `[ … ]` sequence MUST be quoted —
`kv: { provider: redis, url: "${REDIS_URL}" }` — exactly as `version:` must be
quoted for a different reason (§1.3). Unquoted, it is a YAML syntax error before
the compiler sees the file. Quoting is always legal, so quoting every env ref is
the safe habit.

**Secret-bearing fields take the env-ref value form only** — a literal is a
compile error (Decision [D41](#d41-env-ref-forms-and-the-secret-field-list)):

| Field | Where |
|---|---|
| `api_key`, `api_secret`, `token`, `password`, `access_key_id`, `secret_access_key`, `session_token`, `credentials_json` | `provider.*`, `storage_backends.*`, `event_sources.*` |
| `url`, `base_url`, `endpoint`, `dsn` | `provider.*`, `storage_backends.*`, `event_sources.*` |
| `token`, `secret` | an `http` trigger's `auth:` and `callback_auth:` blocks (§13.3) |
| `join_token` | the deploy layer's `hub:` block (§14.2) |

The table classifies these field *names* wherever they occur; it never makes one
legal where its section's own key rules do not admit it. A `provider.*` takes
the keys of its `kind`'s row and no others (§12.1, Decision
[D106](#d106-a-provider-kinds-key-row-is-closed)), so `token:` on a provider is
an unknown key whatever this table says about the name, while `storage_backends`
and `event_sources` configs are plugin objects (D50) where the wider list is
live.

**The classification is total.** Every string-valued surface in this grammar
falls in exactly one of three classes, and class 3 is the default: a surface
this section does not place in class 1 or class 2 is in class 3 (Decision
[D92](#d92-the-env-ref-classification-is-total-over-string-surfaces)). No string
surface is left to an implementer's judgement.

| Class | Surfaces | Rule |
|---|---|---|
| **1. Env-ref value only** | the secret and connection fields tabulated above | the whole string is one `${NAME}` reference; a literal is a compile error |
| **2. Interpolable** | `http` node and `http:` tool-binding `url` and `headers` values; the whole `exec:` surface — `command`, every entry of `args`, `cwd`, and `env` values, on both the tool binding (§6.1) and the inline node (§8.2); provider `headers` values and the non-secret provider keys of §12.1 (`region`, `location`, `project`, `organization`, `profile`, `api_version`); non-secret `storage_backends` and `event_sources` config values (§14.3, §14.4) | embedded `${NAME}` tokens are substituted at process start |
| **3. No refs** | **everything else** | an unescaped `${NAME}` token is a **compile error** naming the field |

Class 3 therefore covers, among others: prompts; every `description:`; every
part of a schema (`enum` members, `pattern`, `format`, `default:` literals); CEL
expressions on every surface; model `id` and every value inside `settings:`;
`embed.model` (§11.2); every identifier and reference position (node ids,
channel names, typed addresses, a store's `backend:` alias, a tool's
`function.name`, an event trigger's `source:`); `version:`; `imports:` entries;
a trigger's `path:`, `cron:`, and `timezone:` (§13.3, §13.4); the `header:` and
`prefix:` of an `auth:`/`callback_auth:` scheme and every `callback_allow:` entry
(§13.3); `hub.public_url:` (§14.2), which is shape-checked here and is part of
what a deployment *is*; a `blob put`'s `content_type:` (§11.4); and every
enum-valued key.

Nothing is interpolated in class 3, so an unescaped token there is an error
rather than text that silently survives into the output — the author who wrote
it expected a substitution. The `$${` escape (above) is how a literal `${NAME}`
is written where one is genuinely wanted.

Two boundaries are worth stating outright, because they are the ones an author
is most likely to guess at:

- **The `exec:` block is interpolable end to end.** A process invocation is the
  archetypal env-parameterized value (`command: "${TOOLBIN}/rg"`,
  `args: ["--host=${DB_HOST}"]`), `cwd` and `env` were interpolable already, and
  nothing here is shell-interpreted — a substituted value is one argv element or
  one variable, never a re-parsed command line (§6.1). This is orthogonal to
  [D24](#d24-function-nodes-reference-tool-defs-inline-exechttp-nodes-stay-for-one-offs)'s
  rule that `args` carries no **CEL**: CEL reads graph data at run time, an env
  ref reads the process environment at start, and the two restrictions are
  independent.
- **Compile-time-checked and identity-bearing strings take no refs.** A cron
  expression, an IANA timezone, a route `path:`, a model `id`, an
  `embed.model` and a `content_type:` are either shape-checked by the validator
  or part of what the composition *is*; a value that only exists at process
  start is a value neither `validate` nor a diff can see. `settings:` is in this
  class for PRD 5.9's reason — every LLM configuration in a project stays
  greppable in one file.

Env refs **survive unresolved into the IR**. `validate` and `build` check
syntax only — an artifact is buildable anywhere, including environments holding
no secrets. Presence is a launch-time check: generated code verifies its
environment at process start, and `run`/`serve` fail fast before invoking the
graph, naming the missing variable (PRD 5.9, §9.15).

### 4.4 Durations

```
duration = integer , ( "ms" | "s" | "m" | "h" ) ;    # e.g. 250ms, 30s, 5m, 24h
```

Single-segment only: `1m30s` is a parse error, write `90s`. Values MUST be > 0.
Used by `timeout:`, `retry.backoff`, `retry.max_backoff`, `human.timeout`, and
trigger `timeout:`.

---

## 5. Agent definitions

An agent is one LLM call with structured output (PRD 5.2, 5.5).

```yaml
agent.reviewer:
  description: Reviews a draft against the goal and returns a verdict.
  model: model.smart
  prompt: |
    You are a meticulous technical reviewer.
    Approve only when the draft fully satisfies the goal.
  tools: [tool.web_search]
  stores: [store.style_guide]
  input:
    goal:  { type: string }
    draft: { type: string }
  output:
    verdict:  { enum: [approve, revise] }
    feedback: { type: string }
  max_tool_iterations: 6
```

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `model` | `model.*` ref | **yes** | — | PRD 5.9; no inline provider or settings overrides |
| `prompt` | string (non-empty) | **yes** | — | system instructions; literal text, no templating (D13) |
| `output` | field map (result surface, §3.5) | **yes** | — | PRD 5.2; MUST have ≥ 1 property |
| `input` | field map (input surface) | no | string-in | §5.3 |
| `tools` | array of `tool.*` / `flow.*`, and `builtin.*` entries | no | `[]` | PRD 5.5, 5.1; the built-ins are §5.5 |
| `stores` | array of `store.*` | no | `[]` | PRD 5.8 |
| `description` | string | no | — | documentation only; not LLM-facing (agents are not tools) |
| `max_tool_iterations` | integer 1..50 | no | `8` | bounds the intra-agent tool loop — *turns* of it, so a refused call spends one exactly as a call that ran does (D51, D119) |

Additional keys are a compile error.

### 5.1 Output schema

`output:` is a field map and MUST declare at least one property. Enum-typed
output fields are what edge guards route over and what exhaustiveness checking
covers (PRD 5.3); arrays in an output MUST carry `max_items` (§3.5).

At runtime the model is constrained to the output schema (structured output);
codegen emits the equivalent validation type and rejects nonconforming responses
before any edge is evaluated.

### 5.2 Prompts

`prompt:` is literal text. There is **no interpolation**: dynamic content reaches
the model through the input schema, which is serialized into the user turn. This
keeps the "spec contains no executable code" posture of PRD 5.5 intact and keeps
prompts diffable (Decision [D13](#d13-prompt-is-required-and-literal)).

### 5.3 Input schema and the string-in default

- With `input:` declared, the agent's input is that closed object. The rendered
  user turn is its JSON serialization. The field map MUST declare at least one
  field: `input: {}` would render an empty object as the user turn, which is
  neither the string-in default nor a usable schema, so it is a compile error.
  (`{}` stays legal on `tool.input`, where it means a no-argument tool — §3.1.)
- With `input:` omitted, the agent is **string-in**: a single unnamed string. At
  a node position it MUST be bound with the scalar form
  (Decision [D14](#d14-string-in-agents-bind-with-a-scalar-input-at-the-node)):

```yaml
nodes:
  write: { agent: agent.researcher, input: "input.goal" }
```

- Name-based wiring (§8.0) applies only to declared, named input fields.

### 5.4 Tools and stores

- `tools:` entries are `tool.*` or `flow.*` addresses. A `flow.*` in a tool list
  MUST declare `description:` (the LLM needs one) and its `inputs`/`outputs`
  become the tool's parameter/result schemas (PRD 5.1 flow-as-tool).
- `stores:` attaches stores; codegen synthesizes LLM-facing tools from the
  store's kind and schema (PRD 5.8). See §11.5 for the synthesized surface.
- Duplicate entries in either list are a compile error.

**What one flow-as-tool call is.** Grammar and runtime are both active: a call
starts an instance of the flow, exactly as a `flow:` node does (§8.5), and the
instance's `outputs:` are the result the model is handed. Six properties fix what
that instance is, and each is another section's rule reaching this call site:

- **its arguments are the flow's `inputs:`**, checked against that schema before
  anything is instantiated. Arguments the schema refuses **return to the model**
  as an error tool result, so it can call again — the same event, answered the
  same way, as arguments a `tool.*` or a synthesized store tool refuses (§6,
  §11.5, Decision [D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node));
- **its instance path** is the agent node's own frame plus `<tool name>/<call
  ordinal>` (§9.4), so a store write inside it derives a key no other call of
  this loop derives;
- **its conversation history is isolated**, always: `context: inherit` is a
  `flow:`-node key (D27), and PRD 5.1 makes a flow's behaviour independent of the
  caller's conversation the reason there is no way to spell otherwise here;
- **its `execution` is the caller's** (§4.1), so a `session`-scoped store inside
  it addresses the caller's partition — which is what §7.7's session-coherence
  check quantifies over when it walks clause 4;
- **its level-1 policy is the one that reached the calling instance** (§9.3,
  D79). A tool attachment declares no `policy:` of its own, and an outer
  hardening of a module is not undone by the depth at which it is reached;
- **the agent node's `timeout:` bounds it**: the node's deadline crosses the
  boundary, as it does at a `flow:` node (§9.2) — with §8.7's exemption intact,
  since a `human` node below still holds that budget still (D102).

A call that **fails** — the instance failed, or it reached quiescence without an
output — fails the agent node. It is never answered with a plausible result, and
never quietly dropped: what the model asked for did not happen, and a run in
which it silently appeared to would be the one outcome PRD 5.3's "the runtime
decides every transition" is written against. That is the other half of D119's
split, and the line between the two is what the model could do about it: a
contract that refuses a *call* is answered by calling differently, and an
instance that ran and failed is not.

A call that is **refused** — its arguments, above, or a name this agent does not
offer — is handed back as an error tool result and costs the loop one of its
`max_tool_iterations` (D51, §5), because the correction is another model call.
A model that never corrects therefore spends the bound and fails the node exactly
as one that never answered does. A refused call spends **no call ordinal**
(§9.4): that ordinal counts invocations and a refusal invokes nothing, so the
next call takes the frame the refused one was offered and an instance path is not
moved by a refusal being inserted before it.

`docs/trace.md` §5 and §7.3 are where the call, the instance and the link between
them are recorded — a refused call under `outcome: "refused"`, whose `error`
carries the sentence the model was handed as the `<message>` half of that
format's `<error name>: <message>` shape. The model's copy is that sentence with
no class in front of it; the record's is the same sentence under the envelope
every error in a trace wears.

### 5.5 Runtime built-in tools

Four tools this runtime implements — a shell and three file operations — are
attached from the same `tools:` list, one name at a time, each carrying the
bounds it runs under:

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

| Built-in | Arguments | Result | Bounds |
|---|---|---|---|
| `builtin.bash` | `command` | `stdout`, `stderr` | `root` (working directory), `timeout` |
| `builtin.read_file` | `path` | `content` | `root` |
| `builtin.write_file` | `path`, `content` | `bytes_written` | `root` |
| `builtin.list` | `path` (default `.`), `glob` (default none) | `entries`, `truncated` | `root` |

The set is **closed**: `builtin.<anything else>` is a compile error, and it grows
by a resolved question rather than by a release adding a name (PRD resolved q31).

**The entry shape.** A `tools:` entry is either a bare `tool.*`/`flow.*` address
(§5.4) or a **single-key mapping** whose key is the built-in's name and whose
value is its bounds. One entry attaches one built-in; a mapping carrying two keys
is a compile error, and there is no key anywhere that grants the set. A built-in
written as a bare address is a compile error naming the mapping form, because the
bounds are not optional.

**`root:` is required on every built-in**, on `builtin.bash` as much as on the
file tools, and it must be **non-empty**: `root: ""` is a compile error, and a
`root:` whose `${VAR}` resolves to the empty string fails the call, because an
empty path is the directory the runtime happened to be started in and a bound
nobody wrote is not a bound. It is interpolable (§4.3 class 2), resolved at
process start, and resolved again as a real directory at each call — a `root:`
naming a directory that does not exist fails the call. Every path argument is
taken relative to it, and a path that **resolves** outside it is refused:
resolution, not string comparison, so a `..` that climbs out and a symlink that
points out are both refused, and a write to a file that does not exist yet
resolves through its parent. A symlink whose target does not exist is refused
rather than followed: there is nothing to resolve, so where it points cannot be
checked, and a write through it would create the file it names. A `builtin.list`
walk does not **descend** into a symlinked directory for the same bound's sake —
the link is one entry of the listing, reported without the trailing `/` a
directory gets, because a walk that followed it would answer with paths outside
the root that no path check was asked of. `builtin.bash` runs with the resolved
root as its working directory.

**`timeout:` is required on `builtin.bash`** and is a §4.4 duration. It bounds
one command; §9.2's node-level `timeout:` bounds the whole agent node, deadline
included, and the two compose rather than replace one another. `timeout:` on a
file tool is an unknown key — there is no command there to bound. What bounds a
file tool is that node-level deadline: a `builtin.list` walk stops where it is
when the node's `timeout:` runs out or the run is cancelled, rather than
finishing a listing the graph has already stopped waiting for
([D124](#d124-a-built-ins-deadline-kills-the-commands-process-group-not-just-the-shell)).

`builtin.list`'s `glob` matches `*` and `?` within one path segment and `**`
across them, which is the spelling most tools use. `**` matches *zero* or more
segments, so a run of them accepts exactly what one accepts.

What the deadline kills is the shell **and every process it started**, and what
it ends is the **call**. The command runs in a process group of its own and the
deadline kills the *group*, because the shell is almost never where the work is:
`npm run build`, `a | b`, `(cd sub && make)` and a plain `some-server &` are all
`bash` forking, and a kill aimed at the shell alone would leave every one of them
running — still writing inside `root:` — after the node they belonged to had
already failed. Under `retry:` that would be two generations of one command in
one root ([D124](#d124-a-built-ins-deadline-kills-the-commands-process-group-not-just-the-shell)).

What outlives the deadline is what **left the group deliberately**: a command
that calls `setsid`, a shell that turned job control on (`set -m`), a daemon that
double-forks away. Those are exactly the processes a hand-rolled `exec:` tool
would have left behind too; the runtime stops reading what such a process holds
rather than waiting on it, so the bound is the composition's however long the
escapee lives. Cleaning up after one is the command's own business, and
containing it is the distribution work's (below).

**A built-in's name on the wire is its local name** — `bash`, `read_file`,
`write_file`, `list` — exactly as an attached `tool.*`'s is, so a `tool.bash` on
the same agent is a `tool-name-collision` (§11.5). Its **address** is what
`docs/trace.md` §7.3 records as the call's target.

**Failure and refusal follow §5.4's split unchanged.** Arguments the built-in's
own schema refuses are handed back to the model
([D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node)).
Everything else — a nonzero exit, a command killed at the timeout, a path that
resolved outside the root, a host with no `bash` on `PATH` — is an *execution*
failure and fails the agent node, where §9's chain decides the run exactly as it
does for an `exec:` tool.

**What bounds a built-in is the root and the timeout, and nothing else.** The
tools run with the privileges of the process running the graph, which is what a
hand-rolled `exec:` tool has always done: a model holding `builtin.bash` holds
arbitrary code execution on that host. Container and syscall isolation, and any
refusal keyed on a deploy target, are the distribution work's and are stated here
rather than implied (PRD resolved q31).

Traces gain no surface: a built-in call is a `ToolCallRecord` like any other, and
`docs/trace.md` §11 keeps its answer out of the format exactly as it keeps an
`exec:` tool's. The **journal** holds the answer in full, which is what makes a
resumed execution consume a recorded `bash` rather than run it again
(`docs/durability.md` §3.2).

---

## 6. Tool definitions

One definition, two usage surfaces: attached to an agent (LLM-discovered,
nondeterministic) and invoked as a `function` node (graph-invoked, deterministic).
The definition is shared; validation is surface-specific (PRD 5.5).

**Where the arguments come from decides what a refusal is.** `input:` is parsed
before the implementation runs on both surfaces, and the two differ in who has to
hear about a mismatch. At a `function:` node the arguments are the composition's,
checked field-by-field at compile time (§8.4), so a runtime mismatch is the
graph's own failure and §9's chain decides the run. Attached to an agent the
arguments are a **model's**, so a schema that refuses them is answered back to
the model as an error tool result and the tool loop turns again — the same rule a
`flow.*` (§5.4) and a synthesized store tool (§11.5) follow, stated once in
Decision
[D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node).
The two even name the tool differently, for the same reason: the refusal a model
reads names it `web_search`, the local name the request offered, because that is
the only spelling a corrected call could use, while the mismatch a `function:`
node fails with names `tool.web_search`, the address whoever fixes the
composition has to find.
What the tool's **implementation** then does is nobody's contract: an `exec:`
that exits outside its accepted list, an `http:` whose response a non-2xx rule
refuses, or a result the tool's own `output:` refuses, fails the node on both
surfaces alike. No rephrasing of a call fixes any of those.

```yaml
tool.web_search:
  description: Search the public web and return ranked result snippets.
  input:
    query:       { type: string, min_length: 1 }
    max_results: { type: integer, minimum: 1, maximum: 10, default: 5 }
  output:
    results:
      type: array
      max_items: 10
      items:
        type: object
        properties:
          title:   { type: string }
          url:     { type: string, format: uri }
          snippet: { type: string }
  http:
    method: GET
    url: "https://${SEARCH_HOST}/v1/search"
    headers:
      authorization: "Bearer ${SEARCH_API_KEY}"
    query:
      q: "input.query"
      n: "input.max_results"
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `description` | string (non-empty) | **yes** | LLM-facing; the selection signal (PRD 5.5) |
| `input` | field map (input surface) | **yes** | tool parameters; `{}` for no-arg tools |
| `output` | field map (result surface, §3.5) | **yes** | result schema; makes edges serializable (PRD 5.7) |
| `exec` \| `http` \| `function` | block | **exactly one** | implementation binding |

### 6.1 Implementation bindings

**Scope inside a binding.** A tool definition is a top-level definition: it is
invocable from any flow and from any agent's tool list, so it has no enclosing
flow and no state. The CEL values a binding may contain — an `http:` binding's
`query:` and `body:` values — therefore see exactly **one** root, `input`, the
tool's own declared `input:` object; `state`, `execution`, and `<node>.output`
are not in scope, and referencing one is a compile error (§4.1, Decision
[D65](#d65-a-tool-implementation-binding-sees-only-the-tools-own-input)).
Everything a tool needs arrives through its declared parameters, which is what
makes the same definition usable from both surfaces.

**`exec`** — shell/subprocess (PRD 5.5):

```yaml
exec:
  command: ripgrep          # argv[0]; not a shell line
  args: ["--json", "TODO"]  # literal strings
  cwd: "${REPO_ROOT}"
  env:
    RG_CONFIG: "${RG_CONFIG_PATH}"
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `command` | string (non-empty, interpolable) | yes | executable name/path; never shell-interpreted |
| `args` | array of string (interpolable) | no | literal argv entries: no CEL (D24), env refs legal (§4.3) |
| `cwd` | string (interpolable) | no | |
| `env` | map env-var-name (`[A-Za-z_][A-Za-z0-9_]*`) → string (interpolable) | no | added to the child environment |
| `expect_exit` | **non-empty** array of **distinct** integers in `0..=255` | no | accepted exit statuses; default `[0]`. Non-empty and distinct on the same rule `expect_status` obeys — see *Accepted-outcome lists* below (D84, [D100](#d100-both-accepted-outcome-lists-are-non-empty-and-distinct)) |

Input/output convention (PRD 5.5): an object input is passed as environment
variables (`UPPER_SNAKE_CASE` of each field, JSON-encoded for non-scalars); a
string input is passed on stdin. The child's **stdout** is decoded as JSON and
validated against `output`, except when `output` declares **exactly one property
and that property is string-typed**, in which case trimmed raw stdout binds to
it.

**The count is over the whole declared property set**, never over its
string-typed members alone (Decision
[D109](#d109-the-single-string-property-exception-counts-the-whole-property-set)).
`output: { text: { type: string } }` takes raw stdout;
`output: { text: { type: string }, count: { type: integer } }` declares two
properties, so stdout is decoded as JSON and both fields are read out of it. The
other reading — "exactly one of the properties is string-typed" — binds one
field from the raw stream and leaves every field beside it with no source at
all, and makes a tool's decoding turn on the *types* of the fields the exception
does not bind.

That single-string-property exception is a **`tool.*`-surface rule**. A tool
declares a *domain* result and has no other way to name raw text, so the
exception is how a tool wrapping a text-emitting command stays expressible.
Inline `exec:`/`http:` nodes do have another way — the envelope fields `stdout`
and `body` (§8.2, §8.3) — so the exception does not apply there and a
non-envelope field is always decoded (Decision
[D91](#d91-the-single-string-property-decode-exception-is-a-tool-surface-rule)).

**Failure.** An exit status outside `expect_exit` is a node error subject to §9;
a status inside it completes the node normally. `expect_exit: [0, 1]` is
therefore how a command whose `1` means "no match" or "tests failed" becomes
routable data instead of a failure — the same knob `expect_status` is for `http`,
and the same predicate on both the tool surface and the inline-node surface
(§8.2, Decision
[D84](#d84-execs-and-https-failure-predicate-is-one-rule-on-both-surfaces-and-both-halves-are-configurable)).

A `tool.*` declares a *domain* result schema, so every one of its fields is
decoded as above — `exit_code`/`stdout` are not special here. Inline `exec:`
nodes are the surface that exposes the process envelope; see §8.2.

Because the input object arrives as environment variables, an `env:` key equal to
the upper-snake-cased name of a declared `input:` field is a collision in which
one value would silently win, and is a compile error naming both — the same rule
inline `exec:` nodes obey (§8.2, Decision
[D66](#d66-an-inline-nodes-input-never-competes-with-its-block-for-the-same-slot)).

**`http`**:

```yaml
http:
  method: POST
  url: "https://${API_HOST}/tickets"
  headers: { authorization: "Bearer ${API_TOKEN}" }
  body:
    title: "input.title"
    body:  "input.summary"
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `method` | enum `GET POST PUT PATCH DELETE HEAD OPTIONS` | yes | explicit; effects are never defaulted |
| `url` | string (interpolable) | yes | |
| `headers` | map header-name (`[A-Za-z0-9_-]+`) → string (interpolable) | no | header names are case-insensitive |
| `query` | map param-name (`[A-Za-z0-9_-]+`) → CEL over `input` | no | |
| `body` | map identifier→CEL over `input` | no | JSON body; illegal for `GET`/`HEAD` |
| `expect_status` | **non-empty** array of **distinct** integers in `100..=599` | no | default: any 2xx. Non-empty and distinct on the same rule `expect_exit` obeys — see *Accepted-outcome lists* below (D80, [D100](#d100-both-accepted-outcome-lists-are-non-empty-and-distinct)) |

Without `body`/`query`, the bound input object is sent as the JSON body
(body-bearing methods) or as query parameters (`GET`/`HEAD`). The response body
is decoded as JSON and validated against `output`, except when `output` declares
exactly one property and that property is string-typed, in which case the raw
response text binds to it — the same `tool.*`-surface exception the `exec`
binding carries above, counted the same way over the whole property set
([D109](#d109-the-single-string-property-exception-counts-the-whole-property-set)),
and inapplicable on inline nodes for the same reason (D91). A status outside
`expect_status` is a node error subject to §9, and a status
inside it completes the node — `expect_status: [200, 404]` makes a 404 routable
data rather than a failure, exactly as `expect_exit` does for an exit code
(D84). As with `exec`, this is the *tool* surface: the response envelope
(`status`, raw `body`) is exposed by inline `http:` nodes only (§8.3).

**Accepted-outcome lists.** `expect_exit` and `expect_status` are the same
construct on two surfaces — a set of outcomes that complete the node, with
everything else a node error (D84) — so they obey one shape rule, on the tool
binding and on the inline node alike (§8.2, §8.3, Decision
[D100](#d100-both-accepted-outcome-lists-are-non-empty-and-distinct)):

- **non-empty**. An empty list accepts no outcome at all, so every run or call
  would be an error — a key that inverts its own purpose, which is the inert
  key [D61](#d61-else-takes-the-literal-true) refuses;
- **distinct members**. Membership is a set test, so a repeated member changes
  nothing about which outcomes are accepted. `expect_exit: [0, 0]` and
  `expect_status: [200, 200]` are compile errors, on the same reasoning that
  makes a repeated `route:` member one (§12.2, D80).

Both halves are decidable in one file, so the published schema enforces them too
(Appendix B).

**`function`** — host-registered function (escape hatch; breaks spec
portability — PRD 5.5):

```yaml
function:
  name: rerank_candidates
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `name` | identifier | yes | looked up in the host registry at build time |

The registry entry's signature is checked against `input`/`output` at build; a
missing registration is a build error, and any composition using a `function`
binding is flagged as non-portable in `validate` output.

**Empty result schema.** `output: {}` (legal at every result surface except an
agent's, §3.9) declares a tool with no result: stdout, the response body, or the
function's return value is **not decoded at all**, and the node contributes
nothing to state and nothing to its edge guards. The failure signal is still
observed — an exit status outside `expect_exit` or a status outside
`expect_status` remains a node error under §9 — which is what makes a
fire-and-forget sink (a queue push, a webhook notification) expressible without
inventing a placeholder field.

### 6.2 Policy on tool definitions

Tool definitions carry **no** `retry`/`timeout`/`on_error`. Policy is a property
of a *use site* — the node — and resolves through the chain in §9.3.

---

## 7. Flow definitions

A flow is a module: a subgraph with a declared I/O surface, interchangeable with
a tool's (PRD 5.1).

```yaml
flow.review_loop:
  description: Draft a document and revise it until the reviewer approves.
  inputs:
    goal: { type: string }
  outputs:
    draft: { type: string }
  nodes:
    write:  { agent: agent.researcher, input: "input.goal" }
    review:
      agent: agent.reviewer
      input: { goal: "input.goal", draft: "state.draft" }
      retry: { max: 2, backoff: 5s }
  edges:
    - { from: start, to: write }
    - { from: write, to: review }
    - { from: review, to: write, when: "review.output.verdict == 'revise'", max_iterations: 3 }
    - { from: review, to: end,   else: true }   # escape edge, §7.4
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `nodes` | map node-id→node | **yes** | ≥ 1 node |
| `edges` | array of edge | **yes** | ≥ 1 edge |
| `outputs` | field map (result surface, §3.5) | **yes** | the module's result surface (PRD 5.1) |
| `inputs` | field map (input surface) | no | default: no inputs |
| `description` | string | no | REQUIRED when the flow is used as an agent tool |

### 7.1 Nodes

`nodes:` maps a node id (§2.4) to a **node object**. A node object carries
exactly one **kind key** plus common keys:

| Kind key | Value | Config | §  |
|---|---|---|---|
| `agent` | `agent.*` ref | node-level keys | 8.1 |
| `exec` | inline block | in-block | 8.2 |
| `http` | inline block | in-block | 8.3 |
| `function` | `tool.*` ref | node-level `input` = args | 8.4 |
| `flow` | `flow.*` ref | node-level keys | 8.5 |
| `map` | inline block | in-block | 8.6 |
| `human` | inline block | in-block | 8.7 |
| `store` | `store.*` ref | node-level `op`/`key`/… | 8.8 |

A node with no kind key, or with more than one, is a compile error naming the
node.

**Common node keys** (legality per kind in §8):

| Key | Type | Notes |
|---|---|---|
| `input` | map field→CEL, or scalar CEL where §8.0 allows it | input bindings; §8.0 (the scalar form on `agent:`/`exec:` only, D88) |
| `writes` | map output-field→channel | write remap (PRD 5.7) |
| `retry` | block | §9.1 |
| `timeout` | duration | §9.2 |
| `on_error` | `fail` \| `skip` \| `{ fallback: <node id or end> }` | §9.2 |
| `description` | string | documentation only |

**Rule of thumb**: schemas and implementation config live *inside* the kind
block; bindings and policy live at node level.

### 7.2 Edges

```yaml
edges:
  - from: review
    to: write
    when: "review.output.verdict == 'revise'"
    max_iterations: 3
  - { from: review, to: end, else: true }   # the escape §7.4 requires
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `from` | node id \| `start` | yes | |
| `to` | node id \| `end` | yes | |
| `when` | CEL (bool) | no | guard over the source node's output (§4.1). Legal on an edge leaving `start` too, where the only roots are `input`/`state`/`execution`; at least one `start` edge must still be unconditional or `else:` (§7.6.3) |
| `else` | `true` | no | marks the default edge; mutually exclusive with `when`. `true` is the only legal value — `else: false` says nothing (an unguarded edge is already unconditional) and is a compile error (D61). REQUIRES a `when:`-guarded sibling edge leaving the same node: with none there is nothing for the edge to be *else* to, and it is unconditional under another name (§7.3, D107) |
| `max_iterations` | integer 1..1000 | no | cycle bound (PRD 5.4); the source node then also needs an unconditional or `else:` edge leaving the cycle (§7.4). REQUIRES `when:` on the same edge — an unconditional or `else:` edge is a guarantee, and a guarantee a budget can withdraw is not one (§7.3.1, D90) |

Self-edges (`from == to`) are legal and form a one-node SCC, which must be
bounded like any other cycle. Duplicate edges (same `from`, `to`, `when`) are a
compile error.

### 7.3 Routing semantics

Deterministic, and evaluated after the source node's output has been validated
(PRD 5.3, G4):

1. Every outgoing edge of the completed node is evaluated **in declaration order**.
2. An edge with no `when:` and no `else:` is **unconditional** — always taken.
3. An edge with `when:` is taken iff its guard evaluates true.
4. An edge with `else: true` is taken iff **no guarded sibling edge** from the
   same source was taken. At most one `else` edge per source node.
5. An edge whose `max_iterations` budget is exhausted (§7.4) is not taken,
   regardless of its guard.
6. **All** taken edges fire. Two or more taken edges are concurrent branches
   (Decision [D17](#d17-edge-selection-is-multicast-with-else-not-first-match-wins));
   branches that write the same state channel MUST target a channel with a
   declared `reduce` policy (§10.2) — the same rule maps obey (PRD 5.6). What
   those branches then do — when they converge, what a convergence sees, and
   when the instance is finished — is §7.6.
7. If no edge is taken, the execution fails with a "no viable route" error naming
   the node. This applies to `start` as well: an execution whose `start` guards
   are all false fails before its first step. The validator rejects
   statically-provable instances (exhaustiveness, §7.3.1; escape edges, §7.4;
   the guaranteed `start` edge and the `on_error: skip` escape, §7.6.3).

**An `else:` edge needs a guarded sibling.** An edge carrying `else: true` MUST
have at least one sibling outgoing edge from the same node carrying `when:`.
With none, rule 4's suppression clause can never fire — no guarded sibling can
be taken — so the edge is taken on every pass, which is exactly what an edge
carrying neither keyword already is (rule 2). The keyword then states nothing,
while the author who wrote it to mean "only if the other edge did not fire" gets
multicast to both targets. It is a compile error naming the node and the edge
(Decision [D107](#d107-an-else-edge-requires-a-when-guarded-sibling)), the same
inert key [D61](#d61-else-takes-the-literal-true) refuses in the other spelling.
The rule is decided from edge shapes alone, before any guard is read, and the
validator owns it: JSON Schema cannot relate two items of an `edges:` array
through a shared `from` value (Appendix B).

Every other site where this document offers "unconditional or `else: true`" as
the guaranteed-to-fire form sits beside a guarded edge by construction —
§7.3.1 clause 1 and §7.6.3 rule 3 are about nodes whose other out-edges carry
guards, and §7.4's escape is the sibling of a `when:`-guarded budgeted edge
(§7.2, [D90](#d90-max_iterations-is-legal-only-on-a-guarded-edge)). §7.6.3
rule 2 is the one worth naming: a `start` with a single outgoing edge satisfies
it with an unconditional edge, never with a lone `else: true`.

#### 7.3.1 Exhaustiveness

PRD 5.3's promise — "enum with 3 variants + node with guarded edges + optional
default → compile error if any variant is unroutable" — is decided over a
**closed set of guard shapes**, so that two conforming validators accept exactly
the same compositions (Decision
[D82](#d82-exhaustiveness-and-exclusivity-read-one-closed-set-of-guard-shapes)).

**Reading a guard against an enum field.** Let `n` be a node and `f` an
enum-typed field of `n`'s output with variant set `V`. A guard `g` on an outgoing
edge of `n` is read against `f` through exactly these shapes — `L` is a string
literal, and either operand order is accepted:

| Shape of `g` | `guaranteed(g, f)` | `possible(g, f)` |
|---|---|---|
| `n.output.f == L` | `{L}` | `{L}` |
| `n.output.f != L` | `V \ {L}` | `V \ {L}` |
| `n.output.f in [L₁, …, Lₖ]` | `{L₁ … Lₖ}` | `{L₁ … Lₖ}` |
| `!g₁` | `V \ possible(g₁, f)` | `V \ guaranteed(g₁, f)` |
| `g₁ && g₂` | `guaranteed(g₁, f) ∩ guaranteed(g₂, f)` | `possible(g₁, f) ∩ possible(g₂, f)` |
| `g₁ \|\| g₂` | `guaranteed(g₁, f) ∪ guaranteed(g₂, f)` | `possible(g₁, f) ∪ possible(g₂, f)` |
| **anything else** | `∅` | `V` |

`guaranteed(g, f)` is the set of variants for which `g` is true *whatever else is
true of the run*; `possible(g, f)` is the set for which `g` is not provably
false. Every literal compared against `f` MUST be a member of `V` — a comparison
against a non-variant is a type error (§4.1) raised before this check runs.

The last row is the load-bearing one. A term the table does not recognize —
`size(state.feedback) > 0`, any call, a comparison against a different field —
contributes **no** guarantee and excludes **no** variant. So
`n.output.f == 'a' && size(state.xs) > 0` is *guaranteed* for nothing (`{a} ∩ ∅`)
while remaining *possible* only for `a` (`{a} ∩ V`), which is exactly the
asymmetry the two checks below need.

**When the check fires.** For each enum-typed field `f` of a node's output: the
node is **routing on `f`** when at least one of its outgoing edges carries a
`when:` guard whose expression mentions `<node>.output.<f>` syntactically. A node
routing on at least one field MUST satisfy the rule below. Mixed-guard nodes are
in scope: a sibling edge whose guard never mentions `f` does not exempt the node
from the check, it simply contributes `∅` to `f`'s coverage.

**What satisfies it.** A node routing on one or more enum fields is exhaustive
when either:

1. it has an outgoing edge that is unconditional or carries `else: true` — that
   edge fires for every value of every field (§7.3 rules 2 and 4); or
2. there is **one** enum field `f` the node routes on whose variants are fully
   covered: `⋃ᵢ guaranteed(gᵢ, f) = V(f)` over the node's guarded outgoing edges.

Otherwise it is a compile error naming the node, the field with the largest
covered set, and that field's uncovered variants (PRD 5.3). One field suffices
because edges are multicast (§7.3 rule 6): a node may carry extra branches
guarded on a second field, and full coverage of the first already proves that no
combination of output values leaves the node with no edge to take.

**A guarantee cannot expire.** An edge whose `max_iterations` budget is exhausted
is not taken, whatever its guard says (§7.3 rule 5) — so `max_iterations` is
legal **only on an edge that also carries `when:`** (§7.2, Decision
[D90](#d90-max_iterations-is-legal-only-on-a-guarded-edge)). Clause 1's edge
therefore never expires, and neither does the `start` edge of §7.6.3 rule 2, the
skip escape of rule 3, or §7.4's cycle escape: every edge this document calls
guaranteed is one whose firing no budget can withdraw. A bounded edge still
counts toward clause 2's union, which costs nothing — §7.4 independently requires
the source of a bounded edge to carry a clause-1 edge, so such a node satisfies
clause 1 already.

**Worked example.**

```yaml
# agent.reviewer output: verdict: { enum: [approve, revise, escalate] }
nodes:
  review: { agent: agent.reviewer }
edges:
  - { from: review, to: publish, when: "review.output.verdict == 'approve'" }
  - { from: review, to: rework,  when: "size(state.feedback) > 0" }
```

`review` routes on `verdict` — the first guard mentions it. `guaranteed` is
`{approve}` for the first edge and `∅` for the second (a `size()` call is the
table's last row), the union is `{approve}`, and there is no unconditional or
`else:` edge: **compile error**, naming `revise` and `escalate`. Adding
`- { from: review, to: rework, else: true }` satisfies clause 1; rewriting the
second guard as `review.output.verdict != 'approve'` satisfies clause 2.
Accepting the pair as written would leave the node dead-ending on §7.3 rule 7
at runtime whenever the verdict is `revise` and `state.feedback` is empty —
the outcome this check exists to make impossible.

This is why `enum` is string-only and closed (§3.3): `V` has to be finite, known
at compile time, and comparable by literal.

### 7.4 Cycles and termination

Back-edges are permitted (PRD 5.4). The compiler computes SCCs and requires:

- **Bounding**: every SCC with ≥ 1 edge MUST be *bounded*. An SCC is bounded when
  at least one of the following holds — PRD 5.4's "`max_iterations` and/or a CEL
  exit condition on at least one edge in the cycle", made decidable
  (Decision [D57](#d57-what-counts-as-a-cel-exit-condition)):

  1. **Counting bound** — some edge whose `from` *and* `to` are both in the SCC
     carries `max_iterations` (and therefore a `when:` guard, §7.2); or
  2. **CEL exit condition** — some node `n` in the SCC satisfies both of:

     - **2(i) — there is a way out**: at least one outgoing edge of `n`
       **leaves** the SCC; and
     - **2(ii) — there is a pass that takes it**: `n`'s outgoing edges admit a
       pass on which **no** edge staying inside the SCC is taken. Decided
       syntactically (Decision
       [D98](#d98-a-cel-exit-condition-is-about-the-in-scc-edges-going-false)):
       no outgoing edge of `n` that stays inside the SCC is unconditional, and
       if one of them carries `else: true`, then `n` also has a `when:`-guarded
       outgoing edge that **leaves** the SCC.

     Both halves are load-bearing. Without 2(i), the pass on which the in-SCC
     guards all go false takes no edge at all — that is §7.3 rule 7's dead end,
     not an exit. Without 2(ii) the loop re-enters whatever the guards say: an
     unguarded in-SCC edge is unconditional (§7.3 rule 2) and fires every pass,
     and an in-SCC `else: true` edge fires unless a **guarded sibling was
     taken** (§7.3 rule 4) — so only a taken guarded sibling *that leaves the
     SCC* can suppress it without re-entering.

     Which edge carries the guard is not fixed, because it is the in-SCC edges
     going false that ends the loop. Both spellings below are bounded, and they
     are the same loop:

     ```yaml
     # guarded back-edge + guarded exit
     - { from: review, to: write, when: "review.output.verdict == 'revise'" }
     - { from: review, to: end,   when: "review.output.verdict != 'revise'" }

     # guarded back-edge + else: escape — the shape §7.4's escape rule,
     # §7.3.1 clause 1 and D19 all call the usual spelling
     - { from: review, to: write, when: "review.output.verdict == 'revise'" }
     - { from: review, to: end,   else: true }
     ```

  An SCC satisfying neither is a compile error naming the SCC's nodes.

  A counting bound is a **static termination proof**; a CEL exit condition is
  not — its guard is a runtime value, so a model that never emits the exit value
  keeps looping. PRD 5.4 accepts both, so the validator does too; only clause 1
  makes the loop provably finite, which is why the examples in this document use
  it.
- **Escape**: the source node of each `max_iterations`-carrying edge MUST have at
  least one outgoing edge that (a) leaves the SCC **and** (b) is unconditional
  (no `when:`, no `else:`) or carries `else: true`. Both halves are load-bearing,
  and (b) is what makes the guarantee hold: an unconditional escape fires on
  every pass, and an `else:` escape fires whenever no guarded sibling was *taken*
  — which includes the pass where the budget runs out, because an exhausted edge
  is not taken (§7.3 rule 5) and so cannot suppress it (§7.3 rule 4). The escape
  itself carries no budget to exhaust: `max_iterations` requires a `when:` guard
  (§7.2, [D90](#d90-max_iterations-is-legal-only-on-a-guarded-edge)) and an
  escape edge by definition has no `when:`. Where the escape is spelled
  `else: true`, §7.3's guarded-sibling requirement
  ([D107](#d107-an-else-edge-requires-a-when-guarded-sibling)) is discharged by
  that same budgeted edge, which carries a `when:` by D90. Exhausting
  a budget therefore always leaves the cycle instead of dead-ending on §7.3
  rule 7. A **guarded** escape is not enough: with
  `when: "…verdict == 'approve'"` as the only way out, the pass that exhausts the
  budget while that guard is false takes no edge at all. A bounded edge whose
  source has no escape of this form is a compile error naming the edge
  (Decision [D19](#d19-max_iterations-semantics-and-the-escape-edge-rule)).
  This rule is stated over the **source node of the bounded edge** and is checked
  there, whatever else bounds the SCC. Clause 2's exit edge always satisfies (a),
  and satisfies (b) only in its `else: true` spelling — so where clause 2's node
  `n` is also the source of a `max_iterations` edge, an `else:` exit discharges
  both rules at once and a `when:`-guarded exit discharges neither. Where they
  are different nodes, the two rules are simply independent.

`max_iterations` counts **traversals of that edge within one flow instance**.
Instances of the same flow (including `map`-dispatched ones) count independently.
Codegen emits one counter per bounded **edge** into the graph state — so an SCC
carrying two `max_iterations` edges carries two counters, one budget each, and
PRD 5.4's "an iteration counter … per bounded cycle" is that same statement for
the one-bounded-edge shape it describes
([D19](#d19-max_iterations-semantics-and-the-escape-edge-rule)); iteration
boundaries are checkpoint/resume points.

### 7.5 Instantiation, inputs, and outputs

- **Inputs**: `flow.<f>.inputs` is the module's parameter surface. Inside the
  flow, `input.<field>` is in scope everywhere (§4.1).
- **Outputs**: at quiescence (§7.6.3), each field of `outputs:` is read from the
  state channel of the same name; that channel MUST be declared in `state:`
  (§10) or it is a compile error. There is no `returns:` binding — use a node
  `writes:` remap to feed a differently-named channel
  (Decision [D53](#d53-flow-outputs-are-name-based-from-state)).

  The channel MUST also be able to **satisfy** the field: every value it can
  hold is a legal value of the field's declared type node, checked at the
  definition rather than left to fail at materialization (Decision
  [D111](#d111-name-based-wiring-is-type-checked-in-both-directions)). This is
  the same requirement a name-based *input* read carries (§8.0) and the read
  counterpart of the write typing §10.2 tabulates; as there, the relation itself
  is the validator's (see *Layer split*), and what this document fixes is that
  the surface is checked, in which direction, and against which two
  declarations. `outputs: { draft: {type: integer} }` over a channel
  `draft: { type: string }` is therefore a compile error naming the flow, the
  field, and the channel.

  Two things do not enter the comparison. A channel's `reduce:` decides what a
  *write* supplies (§10.2), while materialization always reads the channel's
  whole value — so an `append` channel of `items: { type: string }` satisfies a
  field declared `type: array, items: { type: string }`, never one declared
  `type: string`. A channel's `default:` is legal where the field's surface
  refuses one (§3.6) and constrains nothing new: it already validates against
  the channel's own type (§10.1). Whether a `merge` channel actually *holds* a
  property at quiescence is a runtime question, and it stays
  [D101](#d101-a-merge-channels-properties-are-unset-until-supplied)'s.
- **As a node**: `{ flow: flow.review_loop, input: {...} }` — §8.5. Bindings are
  **total**: every input field the subflow declares without a `default:` MUST be
  bound by the instantiating node's `input:`, and an unbound one is a compile
  error. Nothing falls through by name — subgraphs receive parent state only
  through explicit bindings (PRD 5.7, §8.0, Decision
  [D68](#d68-flow-node-bindings-are-total-nothing-falls-through-a-module-boundary)).
- **As a tool**: listing `flow.review_loop` in an agent's `tools:` makes its
  `inputs`/`outputs` the tool's parameter/result schemas. `description:` is then
  REQUIRED.
- **Recursion is forbidden**: a flow that reaches itself — in the sense §7.7
  fixes, so through `flow:` nodes, `map` dispatch targets, or tool attachment —
  is a compile error naming the cycle
  (Decision [D26](#d26-flow-defs-outputs-required-description-when-tool-no-recursion)).

### 7.6 Concurrency, convergence, and termination

§7.3 rule 6 makes branching first-class: every taken edge fires, so one node can
start two or more branches. This section defines what those branches do — when a
node with several predecessors runs, what a branch whose guard was false leaves
behind, in what order concurrent writes land, and when the flow instance is
finished. Every rule here is decided from the graph and from recorded node
outputs alone, so a replay reproduces the live run's schedule and values exactly
(PRD 5.6, 5.12).

**Steps.** A flow instance executes in **steps**. Step 0 runs the nodes targeted
by the taken edges leaving `start`. Each node's outgoing edges are evaluated
when **that node** completes (P1, §7.6.1), over the state the node started its
step on plus the node's own writes — a concurrent sibling's same-step write is
not visible to a guard, exactly as it is not visible to the node's own
execution. When every node of step *k* has completed, all of step *k*'s writes
are applied to state in canonical order (§7.6.4); the union of the targets of
all edges taken in step *k* is step *k+1*. The instance finishes when that
union is empty (§7.6.3).

A node **completes** when its own work is done. For the two composite kinds:

- a `flow:` node completes when its subflow instance reaches quiescence;
- a `map` node completes when every dispatched instance has completed, been
  resolved by `on_item_error`, or been detached — a detached dispatch is
  resolved the moment it is issued. This is the fan-out barrier (§8.6 rules 6
  and 7).

Two properties follow, and a conforming implementation MUST preserve both:

- **P1 — barrier**: a node's outgoing edges are evaluated only after that node
  has completed, never before.
- **P2 — one run per step**: a node targeted by two or more edges taken in the
  *same* step runs **once** in the next step.

*Codegen note.* This is LangGraph's superstep model, which supplies both
properties directly: a superstep is a barrier, and a node scheduled by several
triggers within one superstep is scheduled once (PRD 5.5, 5.12). They are stated
as properties rather than as a wiring recipe so codegen keeps its choice of
shaping — a conditional edge emitting `Send`s, a deferred join node — which is
M1's call.

#### 7.6.1 Exclusive edges, forks, and concurrent nodes

Two outgoing edges of the same node are **exclusive** when the validator can
prove they are never taken together:

1. one carries `else: true` and the other carries `when:` — §7.3 rule 4 makes
   those mutually exclusive by construction; or
2. both carry `when:` guards for which **some** enum-typed field `f` of the
   source node's output has `possible(g₁, f) ∩ possible(g₂, f) = ∅` — the same
   closed guard table §7.3.1 fixes, read for disjointness instead of coverage.
   `verdict == 'approve'` and `verdict == 'revise'` are exclusive
   (`{approve} ∩ {revise}`), and so are
   `verdict == 'approve' && size(state.xs) > 0` and `verdict == 'revise'`,
   because an unrecognized conjunct widens nothing (`{approve} ∩ V`).
   `verdict == 'approve'` and `size(state.xs) > 0` are **not**: the second guard
   is possible for every variant, which is the conservative answer.

Any other pair is **co-takeable**. Co-takeability is a relation on a **pair** of
sibling out-edges and is never a property of one edge on its own: an edge is not
"co-takeable" or "exclusive" in isolation, it is one or the other *with respect
to a named sibling*. Every use below therefore names the pair (Decision
[D99](#d99-co-takeability-is-a-relation-on-a-pair-of-sibling-edges)).

A node with two distinct outgoing edges that are co-takeable **with each other**
is a **fork**, and each such pair is a **co-takeable pair** of that fork. A fork
may have out-edges belonging to no co-takeable pair — an `else:` edge among
guarded siblings is the ordinary case — and those edges are, correctly, compared
with nothing: such an edge fires only on the passes where none of the others did.

Two nodes are **concurrent** when both are reachable from a common fork through
the two edges of **one co-takeable pair** of it — one node through each — and
neither is reachable from the other. Wherever this document says "concurrent
contexts" — the reduced-channel rules of §8.0, §10.2, and §8.6 rule 5 — it means
exactly that, plus the instances of one `map` node, which are concurrent with
each other.

The analysis is deliberately conservative: a guard pair it cannot prove
exclusive is co-takeable, so it may ask for a `reduce:` policy on a channel two
branches could not really both write. Declaring the policy is the cost;
[D32](#d32-reduce-policies-are-typed-and-last_wins-is-explicit) already holds
that a declared overwrite beats a silent race.

#### 7.6.2 Convergence

A node with two or more incoming edges is a **convergence**. P2 gives it AND-join
behavior for free within one step: when a fork's branches are the same length,
every branch that fired delivers in the same step, and the convergence runs once
with all of their writes already applied.

A branch whose guard was false delivers nothing and is not waited for — **a false
guard can never deadlock a convergence**, because nothing ever waits. What a
convergence *reads* is state, never its predecessors: node input bindings see
`input`, `state`, and `execution` only (§8.0,
[D42](#d42-node-outputs-are-readable-only-from-edge-guards-and-mapover)). So a
convergence reached by one of two branches sees the channels that branch wrote,
while the channels the other branch would have written hold whatever they held
before (§10.1). That is why a join here needs no data-arrival protocol at all.

Arrivals in **different** steps schedule the node again: a convergence reached at
step *k* and again at step *k+2* runs twice. Well-defined, rarely intended — so
the statically visible case is refused:

**Balanced convergence.** The check is per fork and per **co-takeable pair**,
because only two edges that can both be taken can deliver twice (§7.6.1,
[D99](#d99-co-takeability-is-a-relation-on-a-pair-of-sibling-edges)).

For a fork `f`, one of its co-takeable pairs `(e₁, e₂)`, and a node `n`, let
`dist(f, e, n)` be the set of step distances from `f` to `n` over paths that
leave `f` by the edge `e` and traverse no node belonging to a cycle (§7.4);
every edge counts as one step. The two edges are compared **against each
other**: if for some node `d` there are distances `a ∈ dist(f, e₁, d)` and
`b ∈ dist(f, e₂, d)` with `a ≠ b`, the convergence at `d` is **unbalanced** and
is a compile error naming `f`, `d`, the two edges, and the two distances
(Decisions
[D69](#d69-execution-is-stepwise-and-convergence-is-a-per-step-join-over-taken-branches),
[D99](#d99-co-takeability-is-a-relation-on-a-pair-of-sibling-edges),
[D112](#d112-balanced-convergence-compares-one-distance-from-each-edge-of-the-pair)).
Both sides have to supply a distance, so a `d` that only one edge of the pair
reaches is never this pair's error — whatever the paths on that side look like,
which is the next paragraph's question, not this one's.

**One side's own distances are not this pair's business.** `dist(f, e₁, d)` may
hold two values on its own, where two paths of different lengths run from `f`
through `e₁` to `d`. Those paths share a prefix and part at some node `g`, and
`g` is where the question belongs, asked of the two edges they leave it by: if
those two are co-takeable, `g` is a fork in its own right and this same check
compares its pair, over exactly the two suffix lengths that differ; if they are
exclusive (§7.6.1), at most one of the paths is taken on a pass, so `d` receives
one delivery from the branch and running twice was never possible. Taking the
union of the two sides instead would refuse that second shape — a guarded
shortcut inside one concurrent branch — while naming a pair that cannot deliver
the two distances the diagnostic reports (Decision
[D112](#d112-balanced-convergence-compares-one-distance-from-each-edge-of-the-pair)).

`end` is exempt: it is not a node, it retires branches instead of running, and
branches legitimately reach it at different depths (§7.6.3). Where a cycle lies
between the fork and the convergence the distance is not static, `dist` is not
computed there, and the runtime rule above is what governs.

`dist` counts **edges** only. A control transfer — `on_error: { fallback: … }`,
`human.on_timeout:` — fires *instead of* the node's outgoing edges (§9.2, §8.7),
never alongside them, so it can never add a second concurrent arrival at a
convergence; and where an error path does deliver to a convergence at some other
depth, the runtime rule above governs, exactly as it does around a cycle. The two
positions are reachability edges for §7.8 and are not steps here: different
question, different relation.

**Worked example — the diamond.**

```yaml
flow.diamond:
  outputs: { report: { type: string } }
  nodes:
    plan:     { agent: agent.planner }
    draft:    { agent: agent.writer }
    research: { agent: agent.researcher }
    merge:    { agent: agent.merger }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: draft,    when: "plan.output.need_draft" }
    - { from: plan, to: research, when: "plan.output.need_research" }
    - { from: draft,    to: merge }
    - { from: research, to: merge }
    - { from: merge, to: end }
```

- **Step 0**: `plan`.
- **Step 1**: whichever of `draft`/`research` the guards selected. The two `when:`
  guards are not provably exclusive, so `plan` is a fork and the two nodes are
  concurrent; with both guards true they run in one step and their writes land in
  the canonical order of §7.6.4.
- **Step 2**: `merge`, **once** — whether one branch or both delivered (P2). It
  reads the channels `draft` and `research` wrote; a channel the branch that did
  not run would have written still holds its previous value.
- **Step 3**: nothing is scheduled. The instance is quiescent and its `outputs:`
  are materialized (§7.6.3).

Adding `- { from: plan, to: merge, when: "plan.output.trivial" }` makes
`(plan→merge, plan→draft)` a co-takeable pair — neither guard excludes the other
— whose distances to `merge`, one from each edge of the pair, are `{1}` and
`{2}`: `merge` would run in step 1 and again in step 2. That is the
unbalanced-convergence compile error; the fixes are to route the short branch
through the same depth, or to make the pair exclusive
(`else: true` on the short edge, or guards §7.6.1 rule 2 can prove disjoint),
which removes it from the check because two edges that cannot both fire cannot
both deliver.

Both guards false is a *different* error — §7.3 rule 7's "no viable route" at
`plan` — which is what §7.4's escape rule and an `else:` edge exist to prevent.

#### 7.6.3 `end`, quiescence, and output materialization

`end` **retires the branch that reaches it**; it does not terminate the instance.
A flow instance is finished when it reaches **quiescence**: a step whose
scheduled set is empty, which is to say every live branch has reached `end`
(Decision [D70](#d70-end-retires-a-branch-and-a-flow-instance-ends-at-quiescence)).

- **Outputs are materialized exactly once, at quiescence**: each field of
  `outputs:` is read from the state channel of the same name (§7.5) after the
  final step's writes have been applied. Nothing is snapshotted at the moment a
  branch reaches `end`, so a value written by a branch that was still running is
  included rather than raced.
- Concurrent branches are **never cancelled**. Cancelling on first arrival would
  make the result depend on completion order — the replay hazard PRD 5.6 names —
  and would leave already-issued effects (store writes, sinks, HTTP calls)
  half-applied with no defined state. `on_error: fail` (§9.2) is how an execution
  is aborted; `end` is not.
- `end` in a control-transfer position — `on_error: { fallback: end }` (§9.2),
  `human.on_timeout: end` (§8.7) — retires that branch with the same meaning.
- Several branches MAY reach `end`; each retires.

**No silent dead ends.** Because a branch retires only at `end`, quiescence is
reachable only when every branch got there. Three static rules keep that true
(Decision [D71](#d71-no-silent-dead-ends-every-node-exits-and-every-run-starts)):

1. **Every node MUST have at least one outgoing edge.** A node with none would
   swallow its branch without reaching `end`. Writing `- { from: n, to: end }` is
   the one-line way to say "this branch is done here".
2. **At least one edge leaving `start` MUST be unconditional or carry
   `else: true`.** §2.4 requires an edge; this requires one that is guaranteed to
   fire, so an execution always has a first step instead of dying on §7.3 rule 7
   before doing any work.
3. **A node declaring `on_error: skip` MUST have an outgoing edge that is
   unconditional or carries `else: true`.** A skipped node produces no output, so
   every guard of its that *references that output* evaluates false (§9.2);
   guards over `input`/`state`/`execution` are unaffected and may still fire. A
   node whose outgoing edges are all guarded therefore has a pass on which none
   is taken — the one where the state-only guards happen to be false too — and
   it would dead-end on §7.3 rule 7, the same failure
   [D19](#d19-max_iterations-semantics-and-the-escape-edge-rule)'s escape rule
   removes for an exhausted cycle budget. One unconditional or `else: true` edge
   removes it: the first fires always, the second whenever no guarded sibling
   was taken (§7.3 rule 4), so some edge is always taken.

In rules 2 and 3 the *unconditional* edge is the form that needs nothing else.
The `else: true` spelling carries its own precondition — a `when:`-guarded
sibling leaving the same node (§7.3,
[D107](#d107-an-else-edge-requires-a-when-guarded-sibling)) — which rule 3's
node has by its own premise, and which a `start` with one edge does not.

#### 7.6.4 Canonical write order

Several writers can write one channel in one step: concurrent branches (§7.6.1)
and the instances of a `map`. Completion order among them is nondeterministic
(PRD 5.6), so it is never what decides the result. Writes are applied in a
**canonical order** computed from the graph alone (Decision
[D72](#d72-concurrent-writes-are-applied-in-a-canonical-order)):

1. The writers of a step are the nodes that completed in it, ordered by **node
   id**, ascending byte order over the identifier grammar (§2.1).
2. A `map` node's writers are its dispatched instances, ordered by **source-item
   index** ascending, occupying the map node's own place in that order. For a
   discriminator-routed map that is one order across every route, because the
   index is over the source array.
3. A `flow:` node is a **single** writer at this level, at its own node id: the
   subflow instance orders its internals by these same rules, and only its
   `outputs:`, materialized at its quiescence, cross the boundary.
4. Within one writer there is at most one write per channel, and that is
   guaranteed rather than assumed: a node's **effective** write map — every
   output field paired with the channel it writes, name-based destinations
   included — MUST be injective (§8.0, Decision
   [D93](#d93-the-effective-write-map-is-what-must-be-injective)), and a
   remapped field is not also written to its same-named channel. A
   non-injective effective map is a compile error.

The reduce policy is then applied in that order: `append` appends in it (which
for a `map` is source-item order, PRD 5.6's requirement, arrived at as a
consequence rather than a special case); `merge` merges in it, so the **last
writer in canonical order** wins per conflicting key; `last_wins` keeps the last
write in it. An unreduced channel has at most one writer per step by
construction — writing one from concurrent contexts is a compile error (§10.2).

Writes from different steps are ordered by step. The channel values entering step
*k+1* are therefore a pure function of the values entering step *k* and the
recorded outputs of step *k*'s nodes, which is what makes a replay reproduce the
live run rather than a plausible alternative to it.

### 7.7 Component reachability

Five static checks ask whether a flow — or one `map` dispatch target — can
*reach* something: session coherence (§11.3), sync-trigger interrupt-freedom
(§13.3, §8.7), detached-dispatch interrupt-freedom (§8.6 rule 7, §8.7),
placement colocation (§14.1 rule 4), and recursion (§7.5). They share **one**
relation, defined here once so that they cannot drift apart (Decision
[D86](#d86-component-reachability-is-one-relation-and-it-crosses-every-invocation-edge)).

A flow `F` **reaches** the components and `human` nodes named by the following,
transitively:

1. **its own nodes** — the `agent.*`, `tool.*`, `flow.*`, or `store.*` address a
   node's kind key names, and the `human` node itself for a `human:` node;
2. **its maps' dispatch targets** — `map.node`, `map.routes.<tag>.node`, and
   `map.default.node` (§8.6);
3. **the stores an agent it reaches attaches** — that agent's `stores:` list
   (§5.4);
4. **the tools an agent it reaches attaches** — that agent's `tools:` list,
   including its `flow.*` entries. Flow-as-tool attachment is a call, and PRD 5.1
   makes the two surfaces interchangeable, so it carries exactly the
   reachability a `flow:` node does;
5. everything every `flow.*` it reaches — by clause 1, 2, or 4 — reaches in turn.

Nothing else creates reachability. `on_error: { fallback: … }` and
`human.on_timeout` name flow-local nodes, already covered by clause 1; the deploy
layer names components without invoking them; and an edge guard reading a node's
output is not an invocation.

| Check | Quantifies over | Rejects when |
|---|---|---|
| session coherence (§11.3) | **declared** triggers (§13) | the trigger's flow reaches a `session`-scoped store and the trigger declares no `session_key:` |
| interrupt-freedom (§13.3, §8.7) | **declared** `http` triggers with `respond: sync` | the trigger's flow reaches a `human` node |
| detached-dispatch interrupt-freedom (§8.6 rule 7, §8.7) | dispatches declaring `detach: true` | the dispatch **target** reaches a `human` node |
| placement colocation (§14.1 rule 4) | the `flow.*` entries of an agent's `tools:` | the attached flow reaches an `agent.*` or `tool.*` placed elsewhere than the agent — clauses 1 and 2 only, since clause 4's attached tools are held by the same rule at their own agent |
| recursion (§7.5) | flow definitions | a flow reaches itself |

The relation is uniform across the five on purpose. An interrupt inside a
flow-as-tool is still an interrupt in the middle of a synchronous request, and
PRD 5.11's settled position is that a `respond: sync` flow is *statically*
interrupt-free; a pause inside a flow-as-tool of an agent a detached dispatch
targets is as unanswerable as one written in the target itself; a session-scoped
store reached through a map-dispatched flow still needs a session identity; a
component reached through a flow-as-tool runs in the calling agent's process, so
its placement is the calling agent's business; and recursion through a tool
attachment is still recursion. Clause 4 — traversal into `tools:` — is the one
every earlier per-check wording left unstated. The third row is the one whose
domain is a **target** rather than a flow: it asks the same question of the
address a dispatch names, which for an `agent.*` target is clauses 3 and 4 alone
(an agent holds no `human` node of its own). The fourth is the one that reads a
*part* of the relation rather than all of it, for a reason stated at §14.1 rule 4:
an attached tool colocates with its own agent by the same rule, so following
clause 4 there would answer one question twice.

This relation answers "can this flow *cause* that component to run". It is not
§7.8's relation, which asks whether a node of one flow is reachable from that
flow's `start`. The two are deliberately separate: different domains (components
across the composition versus node ids inside one flow) and different questions,
so neither is defined in terms of the other.

### 7.8 Node reachability

Every node of a flow MUST be reachable from that flow's `start`. A node that is
not is a compile error naming the flow and the node — the unreachable-node check
of PRD §7 M0 (Decision
[D95](#d95-node-reachability-counts-edges-and-the-two-control-transfer-positions)).

Reachability is computed per flow, over the flow's own **control-transfer
relation**: node `n` transfers to node `m` when

1. an edge (§7.2) declares `from: n, to: m` — `start` is the root, so an edge
   `from: start` makes its target reachable; or
2. `n` declares `on_error: { fallback: m }` (§9.2); or
3. `n` is a `human` node declaring `on_timeout: m` (§8.7).

A node is reachable when some chain of those transfers leads to it from `start`.
Guards are ignored: an edge with a `when:` transfers control for this purpose,
because whether it fires is a runtime question and this check is about whether a
node is *ever* addressable (§7.3.1 is where guard coverage is decided).

Clauses 2 and 3 are the load-bearing ones. They are the document's two
**control-transfer positions** (§2.4) and they schedule a node exactly as an
edge does (§9.2, §8.7), so a node addressed only by one of them is live code: a
dedicated `cleanup` node reached solely by `on_error: { fallback: cleanup }` is
an ordinary pattern, and an edge-only reading would reject it while accepting
the same node the moment an unrelated inbound edge appeared. `end` is not a node
and is never asked about; `start` needs no inbound transfer.

Reachability constrains *entry*, and §7.6.3's rules constrain *exit*: every node
must be reachable from `start` and must have an outgoing edge. A node targeted
only by a fallback still needs one — `- { from: cleanup, to: end }` is the usual
line — because retiring a branch is what `end` is for (§7.6.3 rule 1).

---

## 8. Node types

### 8.0 Input bindings, name-based wiring, and `writes`

**Reading.** How a node's input fields are resolved depends on whether the target
is in the same flow or behind a module boundary
(Decision [D15](#d15-node-level-input-is-the-one-binding-mechanism), Decision
[D68](#d68-flow-node-bindings-are-total-nothing-falls-through-a-module-boundary)).

*In-flow targets* — `agent:`, `exec:`, `http:`, `function:`, and `human:` nodes,
whose target is invoked inside this flow's own scope:

1. an explicit `input:` binding for that field, if present;
2. otherwise the state channel of the same name (§10);
3. otherwise the enclosing flow input of the same name;
4. otherwise the field's own `default:`, if it declares one — such a field is
   optional at its surface (§3.6), so it never forces a binding;
5. otherwise a compile error naming the unbound field.

*Module-boundary targets* — `flow:` nodes (§8.5) and `map` dispatch (§8.6),
which instantiate a component with its own scope:

1. an explicit binding for that field, if present;
2. otherwise the field's own `default:`, if it declares one;
3. otherwise a compile error naming the unbound field.

Steps 2 and 3 of the in-flow chain — the state channel and the enclosing flow
input of the same name — **do not apply across a module boundary**. Name-based
wiring is a convenience *within* one flow's scope; nothing crosses a module
boundary implicitly (PRD 5.7). A subflow that declares `{goal, draft}` and is
instantiated with `input: { goal: … }` is a compile error naming `draft`, even
where the caller happens to have a `draft` channel.

Neither chain applies to a **`store:` node**, which declares no `input:` at all:
a store op has no input schema, and its parameters are the per-op values of
§11.4, written out in the node — the CEL ones resolved directly against the
roots of §4.1, and the three that are literals (`top_k`, `limit`,
`content_type`) resolved not at all (§8.8). Nothing falls through by name there
— an omitted `key:` is a missing required parameter, never a lookup of a channel
named `key`.

**Types across a name-based read.** Step 1 puts a CEL expression between the
source and the field, so it is typed by its result (§4.1). Steps 2 and 3 put
nothing there, so the two declarations meet directly and the source MUST
**satisfy** the field: every value the channel — or the enclosing flow input —
can hold is a legal value of the field's declared type node, or it is a compile
error naming both. That is one requirement with a second site, a flow's
`outputs:` reading its channel (§7.5), and it is the read counterpart of the
write typing of §10.2 (Decision
[D111](#d111-name-based-wiring-is-type-checked-in-both-directions)).

```yaml
review:
  agent: agent.reviewer
  input:                       # explicit bindings (CEL, §4.1)
    goal:  "input.goal"
    draft: "state.draft"
```

Explicit `input:` MUST bind a subset of the target's declared input fields. On
`flow:` nodes `input:` is REQUIRED whenever the subflow declares an input field
with no `default:` (PRD 5.7 `passVariables` discipline). On `map` nodes the
per-item binding lives inside the `map:` block instead (§8.6).

**The two `input:` forms.** A **field map** binds declared input fields by name.
The **scalar** form (`input: "<CEL>"`) supplies one unnamed value, and is legal
only where a single unnamed value has a defined destination (Decision
[D88](#d88-the-scalar-input-form-is-legal-only-where-an-unnamed-value-has-a-destination)):

| Position | Scalar `input:` |
|---|---|
| `agent:` node | legal **iff** the agent is string-in — it declares no `input:` (§5.3, [D14](#d14-string-in-agents-bind-with-a-scalar-input-at-the-node)) |
| `exec:` node | legal — the value is passed on the child's stdin (§6.1, §8.2) |
| `map` per-item `input:`, and a route's | legal **iff** the dispatch target is a string-in agent (§8.6 rule 12, [D75](#d75-map-dispatch-bindings-take-both-input-forms)) |
| `http:`, `function:`, `flow:`, `human:` nodes | **ILLEGAL** |
| `store:` node | no `input:` key at all (above) |

An inline `http:` node builds its request out of named fields (§8.3); a
`function:` node's arguments are checked field-by-field against the tool's
declared `input` (§8.4); a `flow:` node binds the subflow's declared `inputs`
(§8.5); a `human:` node's `human.input` is a field map (§8.7). In each of those
a bare scalar names no destination, so it is a compile error rather than a
guessed one.

**Writing.** After a node completes, each field of its output **that the result
carries** is written to the state channel of the same name **if such a channel is
declared**; fields with no matching channel stay node-scoped and remain readable
as `<node>.output.<field>` by that node's outgoing edge guards and by `map.over`
(PRD 5.7 tier 1). `writes:` remaps the destination:

```yaml
review:
  agent: agent.reviewer
  writes: { feedback: reviewer_feedback }   # output field -> channel
```

- Keys MUST be output field names of the node; values MUST be declared channels.
- A node's **effective write map** is the whole picture of where its output
  goes: each output field paired with the channel it actually writes — the
  `writes:` value when the field is remapped, otherwise the same-named channel
  when one is declared, and nothing at all when neither applies (that field
  stays node-scoped). The effective write map MUST be **injective**: no two
  output fields may land on one channel, which would leave one node making two
  unordered writes to it (§7.6.4). A non-injective effective map is a compile
  error naming both fields and the channel (Decision
  [D93](#d93-the-effective-write-map-is-what-must-be-injective)).

  Both spellings of the collision are caught. Two remaps onto one channel
  (`writes: { a: c, b: c }`) is the obvious one; the other is a remap landing on
  a sibling's name-based destination — output `{a, b}` with `writes: { a: b }`
  and a declared channel `b`, whose effective map is `{a → b, b → b}`. That
  second form passes a check that reads `writes:` alone, which is why the rule
  is stated over the effective map rather than over the remap.
- Every write — name-based or remapped — is type-checked against the target
  channel by its reduce policy (§10.2): whole value for an unreduced or
  `last_wins` channel, one element for an `append` channel, a partial object for
  a `merge` channel.
- A remapped field is not also written to its same-named channel.
- A field the node's result does not carry performs **no write**: the channel it
  would have written — remapped or same-named — keeps whatever it held. Only a
  `kv`/`blob` `get` on a miss can produce one (§11.4, Decision
  [D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)),
  and a channel that has never been written is unset, so the failure surfaces at
  the next read of it rather than here (§10.1). The **effective write map** above
  is unaffected: it is computed from declared schemas, so an absent field
  contributes no write on that pass and its writer takes no turn in that
  channel's canonical order (§7.6.4).
- Remapping is the fix for channel collisions between nodes and for renames
  across subgraph boundaries (PRD 5.7).
- Writing the same channel from concurrent contexts (§7.6.1: concurrent branches,
  `map` instances) requires a channel with a declared `reduce` policy (§10.2),
  and the writes are applied in the canonical order of §7.6.4.

### 8.1 `agent`

```yaml
review:
  agent: agent.reviewer
  input: { goal: "input.goal", draft: "state.draft" }
  writes: { feedback: reviewer_feedback }
  retry: { max: 2, backoff: 5s }
  timeout: 90s
  on_error: { fallback: escalate }
```

Kind key `agent:` takes an `agent.*` reference. The node's input/output schemas
are the agent's. All common node keys are legal (PRD 5.5).

### 8.2 `exec`

Inline subprocess step. Use a `tool.*` with an `exec:` binding instead when the
implementation is shared or LLM-facing (§6).

```yaml
run_tests:
  exec:
    command: npm
    args: ["test", "--silent"]
    cwd: "${REPO_ROOT}"
    env: { CI: "true" }
    expect_exit: [0, 1]        # 1 = tests failed: data, not a node error
    output:
      exit_code: { type: integer }
      stdout:    { type: string }
  input: { pattern: "state.test_filter" }
  on_error: skip
```

`exec:` block keys: `command` (required), `args`, `cwd`, `env`, `expect_exit` as
in §6.1, plus:

| Key | Type | Required | Default |
|---|---|---|---|
| `output` | field map (result surface, §3.5) | no | `{ exit_code: {type: integer}, stdout: {type: string} }` |

The node-level `input:` bindings produce the object passed to the child as
environment variables — binding keys are identifiers (§2.1) and are
upper-snake-cased on the way into the environment, so `pattern:` above arrives as
`PATTERN` — or on stdin for a scalar binding, per the §6.1 convention. The
in-block `env:` map writes into the same environment, so an `env:` key equal to
the upper-snake-cased name of an input binding is a collision in which one of the
two values would be silently discarded, and is a compile error naming both
(Decision [D66](#d66-an-inline-nodes-input-never-competes-with-its-block-for-the-same-slot)).

**Failure.** §6.1's predicate applies here unchanged: an exit status outside
`expect_exit` (default `[0]`) is a **node error** subject to §9, and every other
status completes the node. Declaring `exit_code:` in `output` is a *decoding*
choice and never by itself turns a failure into data — widening `expect_exit`
is what does, which is why the two keys are separate (D84). Under the default
`expect_exit` the envelope's `exit_code` can therefore only ever hold `0`; it
stays in the kind default because widening the accepted set is a one-key edit
and `exit_code` is then the routing surface, as `run_tests` above shows.

**Result binding.** An inline `exec:` node wraps a *process*, so its result is
the process envelope, not a decoded payload
(Decision [D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads)).
`exit_code` (`{type: integer}`), `stdout` (`{type: string}`) and `stderr`
(`{type: string}`) are **envelope fields**: declaring one in `output` binds it
from the child process directly, and it is never decoded from stdout. Declaring
an envelope name with any other type is a compile error. Every *other* declared
field is decoded from stdout as JSON — **always**, whatever its type and however
many fields there are. §6.1's single-string-property exception is a
`tool.*`-surface rule and does not apply here: raw stdout already has a name on
this surface, `stdout`, so a non-envelope string field is a decoded field rather
than a second spelling of the raw stream (Decision
[D91](#d91-the-single-string-property-decode-exception-is-a-tool-surface-rule)).
When `output` declares only envelope fields — as the default does — stdout is
never parsed.

### 8.3 `http`

```yaml
notify:
  http:
    method: POST
    url: "https://${HOOKS_HOST}/notify"
    headers: { authorization: "Bearer ${HOOKS_TOKEN}" }
    body: { draft: "state.draft" }
    expect_status: [200, 202]
    output:
      status: { type: integer }
      body:   { type: string }
  timeout: 10s
  retry: { max: 3, backoff: 1s }
```

`http:` block keys are §6.1's plus:

| Key | Type | Required | Default |
|---|---|---|---|
| `output` | field map (result surface, §3.5) | no | `{ status: {type: integer}, body: {type: string} }` |

An inline node's `query:`/`body:` CEL is *flow*-scoped — `input`, `state`,
`execution` (§4.1) — unlike the same keys inside a `tool.*` binding, which see
only the tool's own `input` (§6.1).

**Failure.** §6.1's predicate applies here unchanged: a response status outside
`expect_status` (default: any 2xx) is a **node error** subject to §9, and every
other status completes the node, so a declared `status:` field records an
accepted status. Declaring `status:` is a *decoding* choice; widening
`expect_status` is what makes a 404 routable data instead of a failure (D84).

**Request payload.** An inline `http:` node has no declared input schema of its
own, so the node-level `input:` bindings (§8.0) build an ad-hoc object which
§6.1's convention sends as the JSON body (body-bearing methods) or as query
parameters (`GET`/`HEAD`) — but only when the in-block key that would carry it is
absent. Those bindings take the **field-map form only**: the ad-hoc object is
built out of named fields, so a scalar `input:` names nothing and is a compile
error (§8.0,
[D88](#d88-the-scalar-input-form-is-legal-only-where-an-unnamed-value-has-a-destination)).

Declaring both is a compile error rather than a silently ignored key
(Decision [D66](#d66-an-inline-nodes-input-never-competes-with-its-block-for-the-same-slot)):
`input:` with `body:` on a body-bearing method, or `input:` with `query:` on
`GET`/`HEAD`. The non-competing combinations stay legal — a `POST` may carry
`query:` for its parameters and let `input:` become the body, and either method
may drop `input:` and write the request out in full.

**Result binding.** As with `exec:` (§8.2), an inline `http:` node's result is
the response envelope. `status` (`{type: integer}`) and `body` (`{type: string}`,
the raw response text) are **envelope fields**: declared, they bind from the
response directly and are never decoded from it, and declaring either with
another type is a compile error. Every other declared field is decoded from the
response body as JSON — **always**, on the same rule §8.2 states for `exec`:
§6.1's single-string-property exception is a `tool.*`-surface rule, and `body`
is how an inline node names the raw response text (D91). So a node declaring
only `status:` records the HTTP status and never parses the body (this is what
the `escalate` node of
[`examples/triage-fanout`](../examples/triage-fanout/flows/triage.yml) does),
while a node that also wants the created ticket id declares
`{ status: {type: integer}, id: {type: string} }` and gets `id` from the decoded
body — one declared string field beside an envelope field, decoded like any
other (Decisions
[D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads),
[D91](#d91-the-single-string-property-decode-exception-is-a-tool-surface-rule)).

### 8.4 `function`

Deterministic, graph-invoked use of a tool definition — the use half of PRD 5.5's
def/use split. The tool's implementation binding (`exec`/`http`/`function`) is
irrelevant here; what matters is that its `input` is a checked signature.

```yaml
lookup:
  function: tool.web_search
  input: { query: "state.goal", max_results: "3" }
  writes: { results: search_results }
```

- `function:` takes a `tool.*` reference.
- `input:` values are CEL and are checked field-by-field against the tool's
  `input` schema (arity and types), unlike agent-attached tool use where the
  model chooses arguments at runtime — which is also why a mismatch here fails
  the node rather than being answered back to anybody (§6, Decision
  [D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node)).

### 8.5 `flow`

Subgraph instantiation — Terraform-style module use (PRD 5.1).

```yaml
sub:
  flow: flow.review_loop
  input: { goal: "state.topic" }
  writes: { draft: section_draft }
  context: isolated
  policy: { timeout: 120s, on_error: skip }
```

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `flow` | `flow.*` ref | yes | — | no recursion |
| `input` | map field→CEL | yes when the subflow declares an input without a `default:` | — | explicit bindings only; unbound non-defaulted fields are a compile error, never a name-based fallthrough (§8.0, D68) |
| `writes` | map output-field→channel | no | name-based | keys are the subflow's `outputs` fields |
| `context` | `isolated` \| `inherit` | no | `isolated` | conversation-history scoping (PRD 5.7) |
| `policy` | `{ retry, timeout, on_error: fail \| skip }` | no | — | override for the nodes *inside*, §9.3. The `fallback` form of `on_error` is ILLEGAL here — its target is flow-local and this level names no flow (§9.2, D103) |
| `retry` | block | no | — | policy for *this* node, §9.1 |
| `timeout` | duration | no | — | policy for *this* node, §9.2 |
| `on_error` | `fail` \| `skip` \| `{ fallback: … }` | no | — | policy for *this* node, §9.2 |

`context: inherit` shares the caller's conversation-history channel with the
subflow; `isolated` (the default) gives the subflow a fresh one. Nothing else
crosses a module boundary implicitly (PRD 5.7). `context:` is legal on `flow:`
nodes only — a `map` dispatch is a module boundary for history too, and an
unconditional one: its instances always run on a fresh, discarded history and
there is no key to say otherwise (§10.4, §8.6 rule 13,
[D105](#d105-a-map-dispatch-isolates-conversation-history-per-instance)).

**`policy:` and the node's own policy keys are different things**, and a `flow:`
node MAY carry both:

- `policy:` is level 1 of the resolution chain (§9.3) for **every node inside**
  the instantiated subflow, propagated into nested instantiations — except that a
  `human` node inside takes no `timeout` and no `retry` from it, at this level or
  any other (§9.3,
  [D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level)); its
  `on_error` resolves here like any node's. It never
  applies to the instantiating node itself. When two or more instantiation-site
  overrides reach the same node for the same policy field — flow A instantiates
  B with `policy: {timeout: 30s}` and a node inside B instantiates C with
  `policy: {timeout: 10s}` — the **outermost** wins, so C's nodes get 30s
  (Decision [D79](#d79-the-outermost-instantiation-site-policy-wins)).
- `retry`/`timeout`/`on_error` at node level are level 2 for **this node**, which
  treats the whole subgraph instance as one activity: `timeout` bounds the entire
  instance, `on_error` fires when the instance fails, and `retry` re-executes the
  instance from its entry as a fresh instance (iteration counters and item
  indexes reset).

So `{ flow: flow.f, policy: { timeout: 30s }, timeout: 10s }` means "no node
inside may run longer than 30s, and the whole instance may not run longer than
10s" — legal, and the tighter outer bound is what ends the instance first.

### 8.6 `map`

Agent-controlled cardinality with deterministic dispatch (PRD 5.6). An agent
never spawns work; it emits an array, and `map` fans out over it.

**Homogeneous:**

```yaml
work:
  map:
    over: plan.output.tasks        # path expression, §4.2
    as: task                       # per-item binding name
    node: agent.worker             # agent.* | tool.* | flow.*
    input: { goal: "task.summary" }
    max_concurrency: 5
    on_item_error: skip
    writes: { result: results }    # results must be a reduced channel
```

**Heterogeneous (discriminator-routed):**

```yaml
dispatch:
  map:
    over: triage.output.findings
    as: finding
    route_by: kind                 # literal discriminator field, v0
    max_concurrency: 8
    routes:
      auto_fixable: { node: agent.fixer, max_concurrency: 5, writes: { patch: patches } }
      needs_human:  { node: tool.review_queue, detach: false }
    default: { node: tool.dead_letter }
```

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `over` | path expression | yes | — | MUST resolve to an array schema with `max_items` |
| `as` | identifier | no | `item` | names the item in `input:` CEL and in traces |
| `node` | `agent.*`/`tool.*`/`flow.*` | homogeneous only | — | mutually exclusive with `route_by` |
| `route_by` | identifier | heterogeneous only | — | MUST equal the item union's `discriminator` |
| `routes` | map tag→route, **≥ 1 entry** | with `route_by` | — | keys MUST be variant tags; an empty `routes:` would dispatch a union to one target and give up narrowing (D30) |
| `default` | route | no | — | catch-all; legal only with `route_by` |
| `max_concurrency` | integer 1..256 | **yes** | — | node-wide **admission** bound over every in-flight dispatch, detached included (D28; a detached dispatch waits for a permit to *start* — the join still never waits on its outcome, D94) |
| `on_item_error` | `fail` \| `skip` \| `{ retry: <retry block, §9.1> }` | no | `fail` | per item (PRD 5.6); rule 10. The one per-item key that stays map-wide |
| `input` | map field→CEL, or scalar CEL | homogeneous form only | whole item | per-item input binding; rules 7, 12 |
| `writes` | map output-field→channel | homogeneous form only | name-based | target channels MUST be reduced; rule 7 |
| `detach` | boolean | homogeneous form only | `false` | fire-and-forget dispatch; rule 7 |

A **route** object takes `node` (required) plus optional `max_concurrency`,
`input`, `writes`, `detach` — same meanings, scoped to that route. A route's
`max_concurrency` MUST be ≤ the map's.

**Rules** (PRD 5.6; all are compile errors when violated):

1. **Bounding is mandatory**: `over` resolves to an array with `max_items`, and
   `max_concurrency` is declared on the map node. Route-level values may only
   tighten it.
2. **Exactly one dispatch form**: `node:` XOR (`route_by:` + `routes:`).
3. **Union items require routing**: if the item schema is a discriminated union,
   `route_by` is REQUIRED; if it is not a union, `route_by` is ILLEGAL.
4. **Exhaustiveness and narrowing per variant**: every variant of the item union
   has an entry in `routes:`, or `default:` is present. Route tags that are not
   variant tags are errors. A named route's target is type-checked against **its
   variant's payload only** (narrowing, PRD 5.6). The `default:` route's target
   is type-checked against the **unrouted variants** — those with no entry in
   `routes:`: its per-item `input:` CEL may select the discriminator field and
   any field declared by *every* unrouted variant, and nothing else. When exactly
   one variant is unrouted, that is precisely that variant's payload. A
   `default:` with no unrouted variant is unreachable and is a compile error
   (Decision [D30](#d30-union-items-require-route_by-non-union-items-forbid-it-default-is-the-catch-all)).
5. **Reduced writes**: anything a dispatched instance writes to shared state MUST
   target a channel with a declared `reduce` policy. Instances are concurrent
   writers, so their writes land in the canonical order of §7.6.4 — by
   **source-item index**, never by completion order. Appended results are
   therefore index-tagged and reordered before the join; a `merge` channel
   written by several instances resolves each conflicting key to the
   highest-indexed item's write, and a `last_wins` channel to the highest-indexed
   item's write outright. Nothing here depends on which instance finished first,
   so replay is deterministic.
6. **Join**: the map node completes when every instance has completed, been
   resolved by `on_item_error`, or been **detached** (rule 7); its outgoing edges
   are evaluated in the next step (§7.6, P1). Sink routes are waited on like any
   other route — that is PRD 5.6's default, and `detach: true` is the only opt-out
   of it. A **detached** instance is *resolved at dispatch*: the join counts it
   the moment the dispatch is issued and never waits for its outcome (Decision
   [D94](#d94-a-detached-dispatch-is-resolved-at-dispatch)). A dispatch of
   **zero** instances — an empty source array, or a producer that was skipped
   (rule 11) — completes immediately, writes nothing, and its outgoing edges fire
   exactly as if every instance had finished; a map every one of whose dispatches
   is detached completes in the same way, for the same reason.
7. **`input:`, `writes:`, and `detach:` describe a dispatch target**, so each is
   declared in exactly two positions: as a map-block key on the **homogeneous**
   form (`node:`), or on an individual **route**. A map block that declares
   `route_by:` MUST NOT declare a map-level `input:`, `writes:`, or `detach:`
   (Decisions [D31](#d31-detach-rules),
   [D85](#d85-a-routed-maps-input-writes-and-detach-are-declared-per-route)). The
   routes of a heterogeneous map are independently typed dispatch targets, each
   narrowed to its own variant (rule 4) with its own input and output schemas: a
   map-level `input:` would have to type-check against every variant at once,
   which is the lowest-common-denominator item type PRD 5.6 rejects; a map-level
   `writes:` would have to name output fields every route's target declares; and
   a blanket `detach:` would silently detach sinks that were written to be
   joined. `max_concurrency:` and `on_item_error:` stay map-wide because neither
   is typed against a target — one bounds the node, the other is a strategy
   (rules 1, 10).

   A detached dispatch is fire-and-forget, and **that is a statement about the
   join**, not only about state: it MUST NOT declare `writes:` and MUST NOT
   write reduced state, and it is **resolved at dispatch** for every purpose
   this section defines (Decision
   [D94](#d94-a-detached-dispatch-is-resolved-at-dispatch)) —

   - the map's join (rule 6) counts it as resolved when the dispatch is issued,
     so the map node can complete — and its outgoing edges fire — while the
     delivery is still in flight;
   - `on_item_error` (rule 10) never applies to it: there is no observed item
     outcome to apply a strategy to, so a detached delivery that fails is not an
     item error, is not retried by the item policy, and does not reach the map
     node's own `on_error:`. `on_error:` on the map still covers the node's own
     failures, including a dispatch that could not be issued at all;
   - nothing it does can fail or delay the enclosing flow instance, which is
     precisely what an author asks for by writing the key — and precisely why
     the default is `false` and PRD 5.6 makes a failed enqueue a surfaced
     failure otherwise.

   A detached dispatch's target MUST NOT **reach** a `human` node, in the sense
   §7.7 fixes — which includes a `human` node inside a flow the target
   instantiates and inside a flow attached to an agent it reaches. The three
   clauses above are why: the join counts the dispatch resolved the moment it is
   issued, so the execution can finish while the instance is still in flight, and
   a pause inside it is a question whose answer nothing is left to receive — the
   wait belongs to an execution that is not waiting for it and is dropped when
   that execution ends. This one is **target-independent**, unlike the
   checkpointing rule below (Decision
   [D118](#d118-a-detached-dispatch-reaches-no-human-node)).

   In v0, `detach: true` is a
   validation error under any target whose execution state is durably
   checkpointed — every target except `local` (§14) — pointing at the roadmap
   (the outbox-pattern delivery is not v0 work). Checkpointing is a property of
   the target, not a spec construct, so this is a **target-dependent** check like
   backend alias resolution (§11.3): the same composition is legal under
   `--target local` and rejected under `--target staging`. Detached dispatches
   receive an `idempotency_key` derived from the execution id and the dispatch's
   flattened instance path — the form §9.4 fixes; delivery is at-least-once and
   sinks are documented to dedupe on it (PRD 5.6), which is what makes an
   unobserved outcome a defensible trade rather than a lost message.
8. **`route_by` is a literal field name**, never a CEL expression — this keeps
   exhaustiveness decidable (PRD 5.6).
9. Node-level `input:` and node-level `writes:` are both ILLEGAL on a `map` node:
   a map node has no input or output of its own, only dispatched instances. The
   per-item binding lives in the `map:` block (or on a route), and so does the
   write remap. Node-level `retry`/`timeout`/`on_error` are legal and apply to
   the map node as a whole, while `on_item_error` governs individual items.
10. **`on_item_error` and per-item policy.** `on_item_error:` takes `fail`,
    `skip`, or `{ retry: <retry block> }`, where the retry block is §9.1's
    verbatim (`max` and `backoff` required, `multiplier`/`max_backoff`/`jitter`
    optional) — the same enum-or-single-key-object shape `on_error:` uses (§9.2).
    A bare `on_item_error: retry` is a compile error: a retry with no bound is
    the unbounded loop PRD 5.6 exists to prevent, and there is nowhere for it to
    inherit one from (Decision
    [D73](#d73-on_item_error-carries-its-retry-policy-inline)).

    Where the policy comes from is fixed, with no implicit chain:

    - `on_item_error:` is read from the `map:` block alone and defaults to
      `fail`. `defaults:` (§9.3 level 3) applies to **nodes**, and a dispatched
      instance is not a node of this flow ([D29](#d29-map-targets-are-component-references-not-flow-local-node-ids)),
      so nothing supplies an item policy behind the author's back. Routes do not
      carry `on_item_error:`; the map-block value governs every route whose
      outcome is observed — which excludes a **detached** route, whose instances
      are resolved at dispatch and have no outcome for a strategy to act on
      (rule 7, [D94](#d94-a-detached-dispatch-is-resolved-at-dispatch)).
    - A retry re-executes the whole dispatched instance from its entry as a fresh
      attempt — iteration counters and the item's own state reset, exactly as a
      `flow:` node's `retry:` re-executes a subgraph instance (§8.5). Nothing the
      instance path is built from changes — the item keeps its index and the
      re-executed instance starts its traversal ordinals over — so every attempt
      derives the same idempotency key (§9.4, PRD 5.6, 5.8).
    - When retries are exhausted the item **fails**, resolving as `fail` does. To
      absorb that, put `on_error:` on the map node itself (rule 9): it applies to
      the fan-out as a whole, so `on_item_error: { retry: {...} }` with
      `on_error: skip` reads "retry each item, and if one still fails, skip the
      fan-out".
    - Nodes *inside* a dispatched `flow.*` are ordinary nodes and resolve
      `retry`/`timeout`/`on_error` through §9.3 with **level 1 absent** — a `map`
      has no `policy:` key, so a dispatched subflow's nodes see node →
      `defaults:` → built-in. A `human` node among them is exempt from `timeout`
      and `retry` at every remaining level, as it is anywhere else (§9.3,
      [D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level)).
11. **`over` reads a dominating node.** In `over: <node>.output.…`, `<node>` MUST
    **dominate** the map node: every path from the flow's `start` to the map node
    passes through `<node>`. Mere path-existence is not enough — a producer
    sitting on a guarded sibling branch may not have run when the map dispatches,
    leaving `over` with no value at all. Dominance is computed on the same flow
    graph §7.4's SCC analysis already builds and stays decidable with cycles
    present, because a back-edge adds no new path from `start`
    (Decision [D76](#d76-mapover-reads-a-node-that-dominates-the-map-node)).
    When the dominating node produced no output on this pass because it was
    skipped (§9.2), the map dispatches zero instances per rule 6.
12. **Per-item bindings take both `input:` forms** — a `map` dispatch is one of
    the three positions where the scalar form has a destination (§8.0,
    [D88](#d88-the-scalar-input-form-is-legal-only-where-an-unnamed-value-has-a-destination)).
    A field map binds the target's declared input fields from the item; a bare
    scalar CEL string binds a **string-in** agent's single unnamed input
    ([D14](#d14-string-in-agents-bind-with-a-scalar-input-at-the-node)), so
    `input: "item.summary"` is how an object item feeds a string-in target. The
    two forms are not interchangeable: a string-in target takes the scalar form
    (or no `input:` at all, when the item's own type is `string` and the
    whole-item default already supplies it), and a target with a declared input
    object takes the field-map form (or no `input:`, when the whole item is
    schema-compatible with that object). The other pairing is a compile error
    naming the target's input contract (Decision
    [D75](#d75-map-dispatch-bindings-take-both-input-forms)). `tool.*` and
    `flow.*` targets always declare an input object, so only an agent target can
    be string-in.
13. **History is per instance.** A dispatched instance runs on a fresh
    conversation history that is discarded when it completes: it neither reads
    nor appends to the enclosing flow instance's `messages` channel (§10.4).
    A `map` block accepts no `context:` key — the opt-in that shares a caller's
    history is a `flow:`-node key (§8.5) — so the isolation is not waivable at a
    dispatch, which is what keeps concurrent instances from interleaving their
    turns into one history (Decisions
    [D105](#d105-a-map-dispatch-isolates-conversation-history-per-instance),
    [D29](#d29-map-targets-are-component-references-not-flow-local-node-ids)).

### 8.7 `human`

Human-in-the-loop pause (PRD 5.5). Grammar **and** runtime are active: a
compiled project stops the execution at the node, reports it as `interrupted`,
and publishes what the human is shown and the schema their answer is held to. A
wait lives in the **process** that is holding it: durable execution is a later
milestone, so a `serve` restarted while a human was thinking has lost it, and the
emitted `README.md` says so where a reader meets the resume route.

**Two surfaces deliver an answer**, and which one is available is a property of
the *invocation* rather than of the composition — the way §9.4 fixes one
idempotency-key delivery surface per binding kind (PRD §9.21). What differs
between them is delivery and nothing else: both address a pause by the instance
path of §9.4, both hold the answer to the node's `output:`, both refuse an answer
that does not fit **without consuming the wait**, and both leave the same trace
record.

| surface | available when | how the answer arrives |
|---|---|---|
| `POST /executions/:id/resume` | the execution was started by the generated app — the third verb of §13.3's invocation surface (PRD 5.11) | the request body, with `?wait=` naming which pause where the execution holds more than one |
| the terminal of an `agent-compose run` | the run's standard input is a terminal, or `AGENT_COMPOSE_INTERACTIVE=1` | one JSON value per line, at a prompt naming the pause it belongs to |

A `run` with **neither** — standard input is not a terminal, or
`AGENT_COMPOSE_INTERACTIVE=0` forces it — has no way to answer, so a run that
reaches a pause reports it and exits on a code of its own rather than waiting or
carrying on. So does one whose terminal goes away.

Four details of the terminal surface are its own, because a stream of typed lines
is not a request, and PRD §9.21 fixes the same four. The framing is **one JSON
value per line** — a value spanning lines has no terminator a prompt could
recognize without either guessing or hanging on a malformed one — and a line that
is not JSON, or that the `output:` refuses, re-prompts. An execution holding more
than one pause is asked **one at a time**, and each question is the **lowest-id
pause open when it is asked** — §13.3's status-route order, so pauses waiting
together are asked in the order the composition fixes rather than the one the
scheduler happened to park them in. It is that rather than a total order over the
run's pauses because a question already on the screen is not taken back: a pause
that opens while one is being asked is asked after it, whatever its id sorts as.
`AGENT_COMPOSE_INTERACTIVE` is what a *script* answers a pause with, and its only
values are `1` and `0`; anything else is refused before the run starts, as a
command that could not be run rather than a setting nobody read
([D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)).
And standard input **ending** withdraws the surface: every pause still waiting,
and every one the run opens after it, becomes the same outcome a run with no
surface has, so a script that answered too few questions ends where it stood
instead of parking for ever.

A `timeout:` is not one of the four. It keeps running while the prompt is on the
screen exactly as it would under `serve`, and an expiry routes through
`on_timeout:` there and then — taking the question with it, so the prompt is
withdrawn and the next pause is asked. The emitted `README.md` documents the
whole loop.

```yaml
approve:
  human:
    input:                                  # what the human is shown
      draft:    { type: string }
      feedback: { type: string }
    output:                                 # what they return; routable
      decision: { enum: [approve, reject, revise] }
      note:     { type: string }
    timeout: 24h
    on_timeout: escalate
  input: { draft: "state.draft", feedback: "state.feedback" }
  writes: { decision: human_decision }
```

| Key (inside `human:`) | Type | Required | Notes |
|---|---|---|---|
| `input` | field map (input surface) | yes | rendered for the human |
| `output` | field map (result surface, §3.5) | yes | routable structured output; resume payloads are validated against it (PRD 5.11) |
| `timeout` | duration | no | wall-clock wait budget |
| `on_timeout` | flow-local node id, or `end` | with `timeout`, never without | route taken on expiry; same targets as `on_error.fallback` (§2.4, §9.2) |

`timeout:` and `on_timeout:` are **jointly optional and jointly required**:
declare both or neither. `timeout:` alone leaves the expiry with no route, and
`on_timeout:` alone declares a route that can never be taken — the same silent
no-op key that [D61](#d61-else-takes-the-literal-true) rejects for `else: false`.
Either half alone is a compile error naming the missing one
(Decision [D52](#d52-human-node-shape)).

Node-level `timeout:` and `retry:` are ILLEGAL on a `human` node — a wait is not
an activity timeout and re-prompting a human is not a retry
(Decision [D52](#d52-human-node-shape)). The exemption is the **whole chain's**,
not this level's: a `human` node also resolves no `timeout` and no `retry` from a
flow node's `policy:` (§9.3 level 1) or from `defaults:` (level 3), so a
composition-wide budget can never cut a wait short (§9.3, Decision
[D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level)).
`on_error:` remains legal and resolves through all four levels (it covers
delivery failures).

Two constructs may not **reach** a `human` node, in the sense §7.7 fixes — which
includes one inside a `map`-dispatched flow and inside a flow attached to an
agent's `tools:`:

- the flow a `respond: sync` http trigger targets, because a pause in the middle
  of a synchronous request has no answer the request can wait for (§13.3,
  PRD 5.11);
- a **detached** `map` dispatch's target, because the join never observes the
  instance and the execution can end while it is still in flight, leaving the
  pause with nothing to deliver an answer to (§8.6 rule 7, Decision
  [D118](#d118-a-detached-dispatch-reaches-no-human-node)).

### 8.8 `store`

Deterministic, graph-invoked store operation — the second consumption surface of
PRD 5.8.

```yaml
load_prefs:
  store: store.user_prefs
  op: get
  key: "execution.session_key"
  writes: { value: prefs }
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `store` | `store.*` ref | yes | |
| `op` | enum | yes | legal set depends on the store's `kind` (§11.4) |
| `key`, `value`, `query`, `prefix`, `top_k`, `limit`, `filter`, `metadata`, `content_type` | per op | per op | §11.4 |

A store-op node takes **no `input:` key**: its parameters are exactly the op's
own row in §11.4, written out in the node, and none of them resolves by name
from a channel or a flow input (§8.0). The op's output schema is derived from the
store definition (§11.4) and is written by name like any other node output.

**Which parameters are CEL.** Six of the nine are, and they are exactly the row
§4.1 scopes to `input`, `state`, and `execution`: `key`, `query`, and `prefix`
are one expression each; `value` is one expression on a `vector upsert` and a
`blob put` and a field map of expressions on a `kv set`; `filter` and `metadata`
are maps of one expression per entry (§11.4). The other three are **literals**,
not expressions: `top_k` and `limit` are integers in the ranges §11.4 fixes, and
`content_type` is a media type string, which §4.3 puts in class 3 so it carries
no env refs either. A CEL string where §11.4 declares an integer is a type
error, not a computed bound — a fan-out's cardinality is a schema bound (§3.5)
and a store op's is a written-down one, and both are meant to be readable
without running the graph. All three are decidable in one file, so the published
schema types them too (Appendix B).

`get` is the one op whose result may omit a field. On a miss it returns
`found: false` and **no `value`**, so the `writes: { value: prefs }` above
performs no write on that pass and `prefs` keeps what it held; a guard wanting
to know reads `load_prefs.output.found`, because reading the absent `value`
fails the execution (§11.4, §4.1, Decision
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)).

---

## 9. Error policy

The compiled graph is the *workflow* (deterministic, replayable); nodes are
*activities* (effectful, retryable) — PRD 5.5.

### 9.1 `retry`

```yaml
retry: { max: 3, backoff: 2s, multiplier: 2.0, max_backoff: 30s, jitter: true }
```

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `max` | integer 1..10 | yes | — | additional attempts after the first |
| `backoff` | duration | yes | — | initial delay |
| `multiplier` | number ≥ 1 | no | `2.0` | exponential factor |
| `max_backoff` | duration | no | — | cap on a single delay |
| `jitter` | boolean | no | `true` | full jitter |

Retries are exhausted **before** `on_error` applies. Retry attempts consume the
node's `timeout` budget and a sync trigger's timeout budget with no special
casing (PRD 5.11).

### 9.2 `timeout` and `on_error`

- `timeout:` — a duration bounding one node execution (all retry attempts
  included).
- `on_error:` — the strategy applied after retries are exhausted:

| Value | Semantics |
|---|---|
| `fail` | abort the execution with the node's error (built-in default); sibling branches stop with it |
| `skip` | the node produces no output and writes nothing; **routing then proceeds exactly as §7.3 specifies**, with one substitution: a guard that references the missing output evaluates to `false`. A node declaring `skip` MUST have an unconditional or `else: true` outgoing edge (§7.6.3 rule 3) |
| `{ fallback: <node id or end> }` | the node's own outgoing edges are **not** evaluated; the fallback target is scheduled in the next step instead (§7.6), or the branch retires if the target is `end`. A node in the same flow only; never `start` |

**Routing after a `skip`.** There is no second routing algorithm. A skipped node
routes through §7.3 unchanged — declaration order, multicast, `else:` last, an
exhausted budget untakeable — and the *only* thing the skip changes is the value
of one class of guard (Decision
[D97](#d97-a-skip-changes-guard-values-not-the-routing-algorithm)):

- a `when:` guard that references the skipped node's output (`<n>.output.…`)
  evaluates to **`false`**: there is no output object to read, and failing the
  execution the way an unset channel read does (§10.1) would make `skip` a
  synonym for `fail`;
- a `when:` guard that references only `input`, `state`, or `execution` — legal
  on any edge (§4.1) and used in §7.3.1's own worked example — evaluates
  **normally**. The skip says nothing about those values;
- an **unconditional** edge fires, as always (§7.3 rule 2);
- an **`else: true`** edge fires iff no guarded sibling was **taken** (§7.3
  rule 4) — so a state-only guard that came out true suppresses it, exactly as
  it would after a successful run.

§7.6.3 rule 3 is what makes this total: with an unconditional or `else:` edge
present, at least one edge is always taken, so a skip can never dead-end the
branch on §7.3 rule 7. Note which of the two does the work in each case — the
unconditional edge fires unconditionally; the `else:` edge fires unless a
state-only guard already routed the branch somewhere.

A node that *ran* and whose result legally omits a field is the other case and
resolves the other way: there is an output object, so a guard reading a field it
does not carry **fails the execution** rather than routing `false` (§4.1, §11.4,
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)).
The substitution above belongs to `skip` alone, and only because the node
produced no output at all.

`fallback` targets a **flow-local node id or `end`** (§2.4), keeping error
routing inside one graph where reachability analysis can see it — a fallback
target is reachable *because* it is one (§7.8 clause 2) — never `start`,
and never a node of another flow (Decision
[D21](#d21-on_error-strategies-and-fallback-targets)). `human.on_timeout` (§8.7)
accepts the same targets.

**The `fallback` form is a node's own key only.** `on_error:` at a
composition-wide level — the `defaults:` section (§9.3 level 3) and a `flow:`
node's `policy:` (level 1) — takes `fail` or `skip`, and `{ fallback: … }` there
is a compile error naming the section or the node (Decision
[D103](#d103-fallback-is-a-node-level-on_error-form-only)). A fallback target is
flow-local by construction, while neither of those levels names a flow:
`defaults:` reaches every node of every flow in the composition, and a `policy:`
reaches every node of the instantiated subflow and of everything it instantiates
in turn — so one id would have to resolve separately inside each of them, in
flows that need not declare it at all. "Retry, then give up" stays expressible at
both levels, because `retry` is a field of its own and `fail`/`skip` are the
strategies that need no target; a node that needs a fallback declares one at
level 2, which is the only place the target is in scope. Both halves are
decidable in one file, so the published schema refuses the form at both levels
too (Appendix B).

### 9.3 Resolution chain

For each policy field (`retry`, `timeout`, `on_error`) independently, highest
precedence first (PRD 5.5's `flow override > node > defaults > fail`):

1. **Flow override** — `policy:` on the `flow:` node that instantiated the
   enclosing flow, propagated into nested instantiations. It applies to the nodes
   *inside* that instance only; the instantiating `flow:` node resolves its own
   policy from levels 1–4 in its own flow (§8.5). When several instantiation
   sites in a nesting chain set the same field, the **outermost** wins — a
   caller's hardening of a module it does not own cannot be undone by that
   module's own instantiation of a deeper one
   (Decision [D79](#d79-the-outermost-instantiation-site-policy-wins)). A `map`
   contributes no level 1: it has no `policy:` key (§8.6 rule 10).
2. **Node** — the node's own `retry`/`timeout`/`on_error`.
3. **Defaults** — the composition's `defaults:` section.
4. **Built-in** — no retry, no timeout, `on_error: fail`.

**A `human` node resolves no `timeout` and no `retry`, at any level.** §8.7 makes
those two keys illegal at level 2; levels 1 and 3 are skipped for them on a
`human` node for the same reason, which is a property of the construct and not of
the file the value was written in — a wait is not an activity timeout and
re-prompting a human is not a retry (Decision
[D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level)). A
`human` node's wait semantics live exclusively in its `human:` block, where
`timeout:` and `on_timeout:` are jointly optional and jointly required (§8.7), so
`defaults: { timeout: 90s }` beside a node declaring `human: { timeout: 24h,
on_timeout: escalate }` leaves that wait running for 24 hours; with no
`human.timeout:` the wait is unbounded. `on_error` is **not** exempt and resolves
through all four levels as on any other node: it covers delivery failures, which
are ordinary node errors (§8.7).

**Nor does the budget of a node *above* a wait cut it short.** A `human` node
reached inside a subflow sits under a `flow:` node or a `map`, and those are not
`human` nodes: they resolve `timeout:` at every level like anything else, and
§8.5 makes a node-level one bound the entire instance. A budget that ran while
the instance was parked at a pause would cap every wait below it at the
composition's default, which is the reading D102 refuses, reached one construct
further out — so the enclosing node's deadline is **held still** for as long as a
wait inside it is open and resumes with the time it had left. §9.2's budget
bounds the work a node execution does; waiting on a human is not work it is
doing. A `timeout: 60s` on a `flow:` node whose subflow pauses for an hour still
means sixty seconds of running.

**Levels 1 and 3 take no `fallback`.** `on_error:` in `defaults:` and in a
`flow:` node's `policy:` is `fail` or `skip` only; the
`{ fallback: <node id or end> }` form is legal at level 2 alone, because its
target is a node id of the flow the declaring node sits in and those two levels
name no flow (§9.2, Decision
[D103](#d103-fallback-is-a-node-level-on_error-form-only)).

```yaml
# main.yml
defaults:
  timeout: 60s
  retry: { max: 1, backoff: 1s }
  on_error: fail
```

`defaults:` accepts exactly `retry`, `timeout`, `on_error`, appears at most once
per composition (§1.5), and applies to every node in every flow — with the one
exemption above, which withholds `timeout` and `retry` from `human` nodes
(Decision [D20](#d20-the-policy-resolution-chain-has-exactly-four-levels),
[D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level)).

### 9.4 The idempotency key

Retries (§9.1) and at-least-once delivery make one effect issuable more than
once, so every effect this grammar delivers without observing its outcome carries
an **idempotency key** that the receiver dedupes on. There are exactly two:
a **detached** `map` dispatch (§8.6 rule 7) and a **store write** (§11.4). PRD 5.6
and 5.8 name the key `execution_id + node + item_index`; this section fixes what
"node" is, because the constructs above make one node id ambiguous within a
single execution (Decision
[D104](#d104-the-idempotency-key-is-the-flattened-instance-path)).

An **effect site** is where an effect is issued: a store node, or a `map` node
paired with one source-item index on a detached route. Its **frame** is

```
<flow-local node id> "/" <traversal ordinal> [ "/" <source-item index> ]
```

where the **traversal ordinal** is how many times that node has already begun
executing in its own flow instance (`0` the first time, `1` on the second
traversal of a bounded cycle, §7.4), and the third component is present only for
a `map` node, naming the instance it dispatches.

The key is `execution.id` (§4.1) followed by the frames of every node crossed
from the **root flow instance** — the one the invocation started (§13) — down to
the effect site, **outermost first**, joined with `/`. A `flow:` node and a `map`
node each contribute their own frame on the way in; integers are decimal; node
ids are identifiers (§2.1), so no component can contain a separator.

**A flow-as-tool call contributes a frame too**, and it is the one frame no node
id names. A `flow.*` in an agent's `tools:` (§5.4) is a call, and a model may
make it more than once in a single execution of the agent node, so the node's own
frame does not address the instances apart. Beneath that frame, each invocation
contributes

```
<tool name> "/" <call ordinal>
```

where the **tool name** is the flow's local name — the name the model calls it
by — and the **call ordinal** counts how many times *that* flow-tool has already
been invoked within *this* execution of the agent node (`0` on the first). Both
components are of the same shapes a node frame is built from: a local name is an
identifier (§2.1) and the ordinal is decimal, so no component can contain the
separator, and a tool frame can collide with no node frame — the frame beneath an
`agent:` node's is a tool call's or there is none, because an agent has no nodes.

**Invoked**, not called, and the difference is one call wide: §5.4 hands a call
whose arguments the flow's `inputs:` refuses back to the model rather than
running it
([D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node)),
and it never reached the flow, so it counts for nothing and the next call takes
the ordinal it was offered. A loop that called wrongly and then corrected
derives exactly the keys a loop that called correctly the first time derives, and
that holds across a re-run too: an attempt whose model needed no correction
re-derives the keys of an attempt whose model did.

Two consequences, and the second is a compromise stated openly rather than left
to be discovered:

- **distinct calls in one tool loop get distinct instance paths.** Two calls to
  one flow-tool are two instances with two sets of effects, and a bare node frame
  would give them one key — the collision this section exists to prevent;
- **a re-run of the tool loop restarts the ordinals**, so the Nth invocation of
  the second run reuses the Nth key of the first. That reuse is **positional, not
  semantic**: the emitter is a nondeterministic model, and what it asks for the
  Nth time on a second run need not be the work it asked for the Nth time on
  the first. It is accepted because the alternatives are worse — a provider's
  tool-use id is fresh on every retry, so key reuse dies and every agent retry
  re-fires every child flow's side effects, while an argument hash merges two
  intentional identical calls into one (PRD resolved q19).

  **Two policies re-run a loop**, and the rule is one rule over both. An
  agent-node `retry:` (§9.1) re-executes the node's activity, loop and all. A
  `map`'s `on_item_error: { retry: … }` (§8.6 rule 10) re-executes a dispatched
  `agent.*` from its entry at the **same** source index, so nothing the item's
  frames are built from changes and the ordinals restart under an unchanged
  prefix — the same at-least-once compromise, reached through a policy that is
  not the node's own. An author who writes either is choosing it.

Like every other frame, this one is **derived**: no spec construct sets or
overrides a tool name or an ordinal, so there is nothing here for `validate` to
reject either.

**Delivery surface.** How the key reaches the sink is fixed per binding kind, so
sinks can be written against a stable contract: an `http:`-bound target receives
it as the `Idempotency-Key` request header; an `exec:`-bound target receives it
as the `IDEMPOTENCY_KEY` environment variable; a `function:`-bound target
receives it as the `idempotency_key` field of its invocation context. The key is
delivery metadata, never part of the target's declared input schema.

**Where a key is observable.** Both carriers record theirs in the run's trace —
a dispatch record's `idempotencyKey` and a store record's — which is the one
place the flattened instance path of a nested fan-out can be read back at all
(§4.1's `execution.item_index` exposes only the innermost index). See
[`docs/trace.md`](trace.md) §8.

```
exec_01/a/0/save/0          # store node `save`, in flow.ingest instantiated by node `a`
exec_01/b/0/save/0          # …and by node `b`: a different write, a different key
exec_01/outer/0/3/inner/0/0/save/0   # `save` under item 0 of `inner`, itself item 3 of `outer`
exec_01/dispatch/0/7        # the detached dispatch of item 7 by map node `dispatch`
exec_01/ask/0/condense/1/save/0      # `save`, inside the second `condense` call agent node `ask` made
```

Two properties follow, and they are the whole point of fixing the form:

- **distinct effects get distinct keys.** A flow instantiated twice in one
  execution, and the repeating item indexes of a nested map, are exactly the
  cases a bare node id collides on — and a collision under at-least-once
  semantics is a real write the sink drops as a duplicate;
- **a repeated attempt at one effect reuses its key.** A node-level retry
  (§9.1), an item retry (§8.6 rule 10), and a `flow:` node's re-execution of an
  instance (§8.5) all re-run the *same* attempt: the traversal ordinal counts
  executions of the node within its instance and a re-executed instance starts
  over, so every attempt derives one key. That is what makes delivery
  at-least-once rather than a stream of distinct writes.

The key is derived, never authored: no spec construct sets or overrides it, so
there is nothing here for `validate` to reject. The rules that *are* checked
belong to the two carrying constructs — `detach: true` under a checkpointed
target (§8.6 rule 7) and the map-write keying rule (§11.4). Nor is the key
writable in CEL: `execution.item_index` exposes only the innermost enclosing
map's index (§4.1), while a key needs every one of them.

---

## 10. State

The `state:` section generates the graph's state schema with per-channel
reducers (PRD 5.7 tier 2).

```yaml
state:
  draft:    { type: string, default: "" }
  feedback: { type: string, default: "" }
  patches:
    type: array
    items: { type: string }
    reduce: append
  totals:                       # no `default:`, so `totals` starts as `{}` and
    type: object                # each property is unset until a write supplies
    properties: { fixed: { type: integer }, skipped: { type: integer } }
    reduce: merge               # it — see §10.1, D101
```

### 10.1 Channels

A channel is a type node (§3.2) plus two channel-only keys:

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `reduce` | `append` \| `merge` \| `last_wins` | no | *unreduced* | concurrency policy |
| `default` | literal validating against the channel's type | no | see below | initial value; ILLEGAL on a union channel (§3.6) |

Channel names are identifiers and MUST NOT be reserved (§2.5). The channel set is
composition-global in **shape**; each flow instance holds its own **values**, so
two flows may both use `draft` without interfering, and a subgraph sees only what
its `input:` bindings and `writes:` remaps carry across (PRD 5.7).

**Initial values.** A channel starts each flow instance at its `default:`. With
no `default:` declared (Decision
[D78](#d78-channel-initial-values-and-reading-an-unset-channel)):

- an `append` channel starts as `[]` and a `merge` channel as `{}` — the identity
  element of the reduce, which is what makes a fan-out that produced zero items
  (§8.6 rule 6) read as "nothing yet" rather than as an error;
- every other channel is **unset**. Reading an unset channel — from a CEL
  expression, a name-based input binding, or a flow's `outputs:` materialization
  (§7.5) — fails the execution naming the channel and the reader. Declaring a
  `default:` is how a channel becomes readable before its first write, which is
  what a convergence reached by only one of two branches (§7.6.2) usually wants.

**Partially-supplied `merge` channels.** A `merge` channel is the one channel
form whose *value* need not validate against its own declared type while the
flow runs: §10.2 lets each write supply a subset of the channel's properties, so
it holds `{}` before the first write and, say, `{fixed: 3}` after a partial one.
Presence is therefore a per-property, per-instant fact, and this document
answers it one level below D78 (Decision
[D101](#d101-a-merge-channels-properties-are-unset-until-supplied)):

- reading a property the channel does not currently hold — from a guard, a
  name-based or explicit `input:` binding, or `outputs:` materialization —
  **fails the execution**, naming the channel, the property, and the reader. It
  is D78's unset-channel rule, one level down, for D78's reason: inventing a
  zero for a declared `integer` manufactures data;
- **type-checking is unaffected**. `state.totals.skipped` is an `integer`
  wherever it is legal to read, because presence is a runtime property exactly
  as an unset channel's is (§4.1 type-checks against declared schemas, never
  against runtime values);
- two ways to make a property readable unconditionally. Declare a `default:` on
  the channel — an object default supplies **every** required property (§3.6,
  [D77](#d77-default-is-legal-on-every-type-node-form-except-a-union)), so one
  `default:` makes the whole channel total from step 0 — or ask first with CEL's
  `has()` (§4.1):
  `when: "has(state.totals.skipped) && state.totals.skipped > 0"`.

```yaml
state:
  totals:
    type: object
    properties: { fixed: { type: integer }, skipped: { type: integer } }
    reduce: merge
    default: { fixed: 0, skipped: 0 }   # both properties readable from step 0
```

A flow `outputs:` field fed by a `merge` channel reads every property the field
declares, so the same rule applies to it at quiescence (§7.6.3) — which is why a
`merge` channel that a flow returns is the case that most wants the `default:`.

### 10.2 Reduce policies

| Policy | Requires | Semantics |
|---|---|---|
| `append` | `type: array` | each write contributes **one element**, appended in canonical write order — which for a `map` is source-item index order (§7.6.4, PRD 5.6) |
| `merge` | `type: object` | shallow key-wise merge of the written object into the channel, applied in canonical write order, so the **last writer in that order** wins a conflicting key |
| `last_wins` | any | the **last write in canonical write order** wins, explicitly declared as concurrency-safe |

Every one of these is defined against the canonical write order of §7.6.4, never
against completion order: the writers of one step are ordered by node id, and a
`map`'s instances by source-item index. Two runs of the same composition over the
same inputs therefore leave every channel holding the same value, and a replay
reproduces it — the guarantee PRD 5.6 asks for when it calls unordered reduces a
silent break of replay.

A channel **without** `reduce:` is *unreduced*: single-writer, sequential. Writing
an unreduced channel from inside a `map` — or from two concurrent nodes (§7.6.1) —
is a compile error (PRD 5.6). Declaring `reduce: last_wins` is how an author opts
into concurrent overwrite explicitly.

**What a write supplies, and how it is type-checked.** The reduce policy decides
whether a write carries the channel's whole value or a contribution to it. This
is the **write** half of the schema-compatibility rule for state wiring (§8.0,
§10.3) — the read half is
[D111](#d111-name-based-wiring-is-type-checked-in-both-directions)'s, at the two
places a channel meets a declaration with no expression between them (§8.0
steps 2–3, §7.5) — and it is decided from declared types alone
(Decision [D58](#d58-append-channels-take-one-element-per-write)):

| Channel | A write supplies | Accepted written type |
|---|---|---|
| unreduced | the channel's whole value | the channel's type |
| `last_wins` | the channel's whole value | the channel's type |
| `append` | one element | the channel's `items` type |
| `merge` | a partial object | an object whose properties are a subset of the channel's, with matching types — which is why a property may be absent when someone reads it (§10.1, [D101](#d101-a-merge-channels-properties-are-unset-until-supplied)) |

Consequences worth stating outright:

- A node whose output field is an **array** may set an unreduced or `last_wins`
  array channel wholesale — `writes: { matches: matches }` where both sides are
  `array<string>` — because that write replaces the value.
- An `append` channel of `items: {type: string}` accepts a write of a **string**,
  never of an `array<string>`. This is what makes fan-in work: one dispatched
  `map` instance contributes exactly one element (§8.6 rule 5).
- Appending several values in one write is deliberately not expressible. Fan out
  with a `map` so each element is its own write, or declare the channel
  `last_wins` and set it whole.

### 10.3 Wiring

Default wiring is name-based in both directions (§8.0): node output field →
channel of the same name; node input field → channel of the same name, falling
back to the flow input of the same name. `writes:` remaps the write side;
`input:` bindings remap the read side.

Both directions are type-checked, each against the reduce policy or the
declaration at its far end: a write supplies what §10.2's table accepts (D58),
and a name-based read requires its source to **satisfy** the field it lands in —
a node input field resolved by name (§8.0 steps 2–3) and a flow `outputs:` field
read from its channel (§7.5), the two places nothing stands between the two
declarations (Decision
[D111](#d111-name-based-wiring-is-type-checked-in-both-directions)).

Every field of a flow's `outputs:` MUST have a declared channel of that name
(§7.5). A `writes:` value or `state.*` CEL reference naming an undeclared channel
is a compile error ("undefined state channel").

### 10.4 Conversation history

An implicit append-only channel named `messages` carries conversation history for
agent nodes (PRD 5.7 tier 3). It MUST NOT be declared in `state:` and MUST NOT be
named in `writes:`. It is **isolated across flow boundaries by default**; a
`flow:` node opts into sharing the caller's history with `context: inherit`
(§8.5).

**A `map` dispatch is a module boundary for history, with no opt-in.** Every
dispatched instance runs on a **fresh** history that is **discarded** when the
instance completes: it never reads the enclosing flow instance's `messages` and
its turns never append to it (Decision
[D105](#d105-a-map-dispatch-isolates-conversation-history-per-instance)). The
rule is the same for every dispatch form and every target — an `agent.*` instance
is one call with an empty history, a `flow.*` instance starts as any isolated
flow instance does, a `tool.*` has no history at all, and a routed map's routes
are no different — so `max_concurrency: 5` can never interleave five
conversations into one channel. `context:` is a `flow:`-node key (§8.5,
[D27](#d27-context-inherit-is-a-flow-node-key-only-default-isolated)) and a `map`
block does not accept it, so the sharing opt-in does not exist at a dispatch. It
still exists *within* one: a `flow:` node inside a dispatched flow may declare
`context: inherit` and share **that instance's** fresh history with the subflow
it instantiates. Inheritance never reaches back across the dispatch.

---

## 11. Stores

Durable, attachable storage as first-class components (PRD 5.8).

```yaml
store.user_prefs:
  kind: kv
  scope: session
  value_schema:
    theme:     { type: string }
    verbosity: { enum: [low, high] }

store.docs:
  kind: vector
  scope: global
  description: Project documentation, chunked.
  embed: { model: text-embedding-3-small, provider: provider.openai, dimensions: 1536 }
  metadata_schema:
    source: { type: string }
  backend: docs_db
  agent_access: read
```

### 11.1 Definition keys

| Key | Type | Required | Notes |
|---|---|---|---|
| `kind` | `kv` \| `vector` \| `blob` | yes | relational/SQL deliberately excluded (PRD 5.8) |
| `scope` | `execution` \| `session` \| `global` | yes | explicit lifetimes (D35) |
| `value_schema` | field map (result surface, §3.5) | `kv` REQUIRED; `vector`/`blob` illegal | stored value shape |
| `metadata_schema` | field map (result surface, §3.5) | `vector` optional; `kv`/`blob` illegal | filterable metadata |
| `embed` | block | `vector` required; others illegal | §11.2 |
| `backend` | identifier (bare alias) | no | abstract slot; never provider config |
| `description` | string | no | LLM-facing for agent-attached stores |
| `agent_access` | `read` \| `read_write` | no (default `read_write`) | narrows the synthesized tool surface |

**`metadata_schema` is a `vector` key.** Nothing in this grammar reads a `blob`
store's metadata: no `blob` op takes a `metadata` or a `filter` parameter — `put`
takes `key`/`value`/`content_type`, `get` returns `value`/`found`, `list`
returns `keys` (§11.4) — and none of the tools a `blob` attachment can
synthesize (`<name>_get`, `<name>_list`, `<name>_put`, §11.5) takes or returns
it either. Declared there it would be a key every write and every read ignores,
so it is a compile error naming the store and its kind rather than silence — the
posture
[D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects) and
[D61](#d61-else-takes-the-literal-true) take on an inert key, and the one §14
already takes on a `storage_backends:` under `local` (Decision
[D113](#d113-metadata_schema-is-a-vector-only-key)). Metadata *on* blobs is an
additive change if a real case appears — parameters on the `blob` rows of §11.4
and a place in §11.5's synthesized surface — and its absence is what says that
is not v0. Until then a blob's filterable attributes live in a `kv` store keyed
by the same key.

### 11.2 `embed` (vector only)

| Key | Type | Required | Notes |
|---|---|---|---|
| `model` | string (non-empty) | yes | provider-native embedding model id (a bare string, not a `model.*` ref — D36); no env refs, exactly as a model `id` (§4.3) |
| `provider` | `provider.*` ref | **yes** | which connection computes the vectors; never the storage backend (below) |
| `dimensions` | integer ≥ 1 | no | asserted against the backend's index |

**Embeddings are served by a `provider.*`, and the storage backend never
computes them** (Decision
[D116](#d116-embedprovider-is-required-and-no-backend-serves-embeddings)). The two
layers answer different questions: `backend:` (§11.3) says *where the vectors
live* and forks per target, while `embed.provider` says *what turns text into a
vector* and does not — PRD 5.9 settles providers as logical-layer, not
per-target. A backend-derived default would invert that, and there is nothing
for it to derive from: §14.3's storage vocabulary declares no embedding
capability, and under `--target local` no alias and no per-kind default is
consulted at all (§14). Naming the connection is what keeps one store's
embeddings identical under `local` and under `staging`, with only the vectors'
home changing.

The referenced provider MUST be able to serve embeddings, checked against the
provider plugin's published capabilities — the same mechanism §12.2 applies to
an agent's model, run at the store definition. `provider:` is a reference
position, so it accepts `provider.*` and nothing else (§2.3).

### 11.3 Backends and scope

- `backend:` names an **abstract alias** defined per target in
  `deploy/<target>.yml` → `storage_backends.aliases` (§14.3). Resolution order:
  explicit alias → per-kind `defaults:` → target built-in. `--target local`
  substitutes local storage for every store unconditionally (PRD 5.8), so under
  `local` no alias and no per-kind default is consulted at all (§14).
- An alias referenced by a store but undefined in the active target is a compile
  error naming the target. `local` is exempt for the reason above: it resolves no
  aliases, so it cannot fail to find one (§14).
- `scope: session` requires the execution to have a session identity, and that
  identity comes from the trigger. The check quantifies over **declared**
  triggers (§13): a declared `http`, `schedule`, or `event` trigger whose target
  flow **reaches** a session-scoped store — in the sense §7.7 fixes, so through
  its own nodes, through a `flow:` node, through a `map` dispatch target, through
  an agent's `stores:` list, or through an agent's `tools:` attachment — MUST
  declare `session_key:`, or it is a compile error naming the trigger and the
  store (PRD 5.8, 5.11).
  `manual` triggers, declared or implicit, carry `session_key: "payload.session"`
  by default (§13.2) and therefore satisfy the check statically; supplying the
  value is a run-time requirement (`--session`), checked at run start like
  env-ref presence (§4.3) rather than at validate time.

### 11.4 Store-op nodes

Legal ops per kind, with their parameters and derived output schema. `V` is the
store's `value_schema` object, which a `kv` store always declares (§11.1); `M`
is its `metadata_schema` object, which a `vector` store MAY leave undeclared —
see the `M`-is-absent rule below.

| kind | `op` | Parameters | Output |
|---|---|---|---|
| `kv` | `get` | `key` (CEL string) | `{ value: V (optional), found: boolean }` |
| `kv` | `set` | `key`, `value` (map field→CEL matching `V`) | `{ key: string }` |
| `kv` | `delete` | `key` | `{ deleted: boolean }` |
| `kv` | `list` | `prefix` (CEL string, optional), `limit` (integer 1..1000, required) | `{ keys: array<string> }` |
| `vector` | `search` | `query` (CEL string), `top_k` (integer 1..100, required), `filter` (map metadata-field→CEL, optional) | `{ matches: array<{ id: string, score: number, text: string, metadata: M }> }` |
| `vector` | `upsert` | `key`, `value` (CEL string — the text), `metadata` (map field→CEL, optional) | `{ id: string }` |
| `vector` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `put` | `key`, `value` (CEL string), `content_type` (literal media type, not CEL; optional) | `{ key: string }` |
| `blob` | `get` | `key` | `{ value: string (optional), found: boolean }` |
| `blob` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `list` | `prefix` (CEL string, optional), `limit` (integer 1..1000, required) | `{ keys: array<string> }` |

Rules (PRD 5.8):

- The parameter list of a row is **exact**: a parameter the op does not take is
  an error, exactly like an unknown key
  (Decisions [D34](#d34-the-store-op-catalog-is-normative-including-derived-output-schemas),
  [D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)).
  `op: get` with `top_k:` is rejected, not ignored. Because `op:` is a literal in
  the node itself, this is decidable per file and the published JSON Schema
  enforces it too (Appendix B).
- `value` on a `kv` `set` is schema-checked against `value_schema`; `filter` and
  `metadata` keys are schema-checked against `metadata_schema`.
- **A store with no `M`.** `metadata_schema:` is optional on a `vector` store
  (§11.1), and a store that declares none has no metadata at all rather than
  empty metadata (Decision
  [D114](#d114-a-vector-store-with-no-metadata_schema-derives-matches-with-no-metadata-field)):

  - the derived item type of a `search` is `{ id: string, score: number,
    text: string }` — the `metadata` field is **absent**, so
    `find.output.matches[0].metadata` is an unknown field and a compile error
    like any other, and the Zod type codegen emits (§3.8) carries three
    properties;
  - `filter:` on a `search` and `metadata:` on an `upsert` have no legal key, so
    either of them naming any key is a compile error naming the store: that is
    the schema check of the bullet above, run against a store that declares no
    metadata schema for a key to be checked against.

  Declaring `metadata_schema: {}` is a *declaration*, not an omission: `M` is
  then the empty closed object (§3.1), the `metadata` field is present and can
  only ever hold `{}`, and the set of legal `filter:`/`metadata:` keys is the
  same empty one. The difference between the two spellings is exactly whether a
  match carries the field.
- **A `get` that misses.** `value` is the one output field in this catalog its
  op may not return: the `kv get` and `blob get` rows are
  `{ value: … (optional), found: boolean }`, and a miss returns `found: false`
  with no `value` at all. Both halves are what §8.0 states for writing an absent
  field and §4.1 for reading an absent value (Decision
  [D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)):

  - **nothing is written** for `value` — the channel a `writes: { value: … }`
    names, or the same-named channel when there is no remap, keeps whatever it
    held (§8.0). A channel that was never written is unset, so a later read of
    it fails naming *that* channel (§10.1,
    [D78](#d78-channel-initial-values-and-reading-an-unset-channel)); a
    `default:` on the channel is what makes a miss readable downstream as
    "nothing stored yet";
  - **reading `<node>.output.value` fails the execution** — from an edge guard
    or a `map.over`, the only two positions that read a node's output (§4.1,
    [D42](#d42-node-outputs-are-readable-only-from-edge-guards-and-mapover)).
    Route on the companion instead — `when: "load_prefs.output.found"` — or ask
    with `has(load_prefs.output.value)`.

  This is not §9.2's `skip`, where a guard over the missing output routes
  `false` because the node never ran: a `get` *ran*, `found: false` is its
  answer, and a guard that reads `value` anyway is reading a value the op
  reported it does not have.
- Store ops are **effects**: reads are recorded and replay consumes history, not
  the live store; writes are at-least-once carrying the idempotency key of §9.4 —
  the execution id plus the store node's flattened instance path, which is what
  keeps two instantiations of one flow, and the repeating item indexes of nested
  maps, from deriving one key for two different writes.
- A store **write** performed inside a `map`-dispatched instance — `kv set`,
  `kv delete`, `vector upsert`, `vector delete`, `blob put`, `blob delete` — MUST
  be one of two forms (PRD 5.8, Decision
  [D67](#d67-store-writes-inside-a-map-item-derived-key-or-a-keyed-kv-write)):

  1. **item-derived key** — the `key:` expression is *item-derived* in the sense
     fixed below, so concurrent instances address disjoint keys. Legal for every
     kind.
  2. **a keyed `kv` write** — `op: set` or `op: delete` on a `kv` store with any
     legal `key:` expression, including one constant across instances.

  Anything else — a `vector` or `blob` write whose key is not item-derived — is a
  compile error naming the node and the store.

  **Item-derivation.** A store node lives inside a `flow.*`, so it is inside a
  fan-out exactly when that flow is a `map` dispatch target, directly or through
  `flow:` nodes. Derivation is computed **per dispatch site**, because one flow
  may be dispatched by several maps and instantiated outside every map as well;
  each site is checked on its own, and a store node no map reaches is not subject
  to this rule at all (Decision
  [D83](#d83-item-derivation-is-traced-through-the-dispatch-binding)).

  For one dispatch site, an expression is **item-derived** when it references
  `execution.item_index`, or when it references `input.<field>` for a field whose
  binding *at that site* is item-derived. Bindings resolve per §8.6 rule 12:

  - the map (or route) declares **no `input:`** — the whole item is the
    instance's input, so **every** `input.<field>` is item-derived;
  - the map declares `input: { <field>: <CEL>, … }` — `input.<field>` is
    item-derived **iff** that CEL references the item binding (the `as:` name,
    `item` by default) or `execution.item_index`. A field bound from `state.*`,
    from the enclosing flow's `input.*`, or from a literal is **not**
    item-derived, however item-derived its *name* looks;
  - the map declares the scalar form (`input: "<CEL>"`) — its target is a
    string-in agent (D75), and an agent contains no store nodes, so there is
    nothing to resolve.

  Derivation propagates through nested instantiation: inside a dispatched flow, a
  `flow:` node's `input:` bindings are ordinary CEL in that flow's scope, so the
  nested flow's `input.<field>` is item-derived iff its binding expression is
  item-derived there. A nested `map` re-roots the computation at its own item.
  `state.*` is never item-derived — it is one object shared by the whole instance
  set — and neither is `execution.session_key`, which is per execution.

  ```yaml
  work:
    map:
      over: plan.output.tasks
      as: task
      node: flow.ingest
      max_concurrency: 8
      input: { doc_id: "state.topic", text: "task.body" }
  ```

  Inside `flow.ingest`,
  `{ store: store.docs, op: upsert, key: "input.doc_id", value: "input.text" }`
  is a **compile error**: `doc_id` is bound from `state.topic`, which holds the
  same value in every instance, so N documents would land on one vector key —
  the hazard form 1 exists to prevent, reached through a binding that merely
  *looks* item-derived at the store node. `key: "input.text"`, a `key:` that
  *reads* the index from inside a string-valued expression — say
  `key: "state.tasks[execution.item_index]"` — or dropping the map's `input:` so
  the whole item is passed all satisfy it. The bare
  `key: "execution.item_index"` does **not**, and the reason is a type rather
  than a derivation: item-derivation asks what an expression *reads*, and the
  index answers it, but the index is an `int` (§4.1) while every op's `key` is a
  CEL **string** (the table above), so that spelling clears this rule and then
  fails type-checking with `type-mismatch`.

  The per-site reading cuts the other way too, and an index-reading `key:` is
  where it shows. A site that **no** map encloses is not subject to
  this rule at all (above) — and at such a site the index has no value, so a
  `key:` written that way fails the read there rather than keying anything
  (§4.1, Decision
  [D115](#d115-executionitem_index-is-absent-where-no-map-dispatch-encloses-the-expression)).
  A `flow.ingest` that is dispatched by a map *and* instantiated by a plain
  `flow:` node is therefore checked at the first site and fails at the second,
  which is the same one-flow-two-sites fact stated once for the static rule and
  once for the runtime one.

  The `kv` exemption is not a loophole: a `kv` write replaces the whole value at
  a slot the author named, so concurrent instances writing one key are a declared
  overwrite, the store-side counterpart of `reduce: last_wins` (§10.2). Its final
  value depends on completion order and is therefore *not* deterministic — which
  is sound here and would not be for a channel, because store reads are recorded
  and a replay consumes history rather than the live store (PRD 5.8), so no store
  value ever feeds the graph's own scheduling. A `blob` or `vector` write has no
  such story: it is bulk content at a document id, so a shared key from N
  instances is N documents' worth of data landing in one slot.

### 11.5 Agent-attached stores

Listing a store in an agent's `stores:` synthesizes LLM-facing tools from the
store's kind and schema (nondeterministic, agent-invoked, recorded as tool
calls — PRD 5.8):

| kind | `agent_access: read` | `agent_access: read_write` (default) |
|---|---|---|
| `kv` | `<name>_get` | `<name>_get`, `<name>_set` |
| `vector` | `<name>_search` | `<name>_search`, `<name>_upsert` |
| `blob` | `<name>_get`, `<name>_list` | `<name>_get`, `<name>_list`, `<name>_put` |

`<name>` is the store's local name (`store.user_prefs` → `user_prefs_get`). A
synthesized tool name that collides with an attached `tool.*`/`flow.*` tool name
is a compile error.

Each tool's arguments are its row of §11.4 with the expressions replaced by what
the model supplies, and they are held to that schema before the store sees them.
A call the schema refuses — a `top_k` outside `1..=100`, a `value` the store's
`value_schema` does not admit — is **returned to the model** as an error tool
result, the way it is at the other two tool surfaces (§5.4, §6, Decision
[D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node)),
and reaches no backend: a refused call leaves no store record. A **backend**
that could not answer is the other half of that split and fails the agent node,
with §9's chain deciding the run.

---

## 12. Providers and models

Two namespaces, split because the layers change for different reasons: providers
hold *connection*, models hold *behavior* (PRD 5.9). Agents reference only
`model.*`.

```yaml
# providers.yml
provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

provider.local:
  kind: openai_compatible
  base_url: ${LOCAL_LLM_URL}
  api_key: ${LOCAL_LLM_KEY}

provider.gateway:
  kind: anthropic           # the same plugin, reached through a corporate
  base_url: ${LLM_GATEWAY}  # gateway that injects the vendor key server-side:
                            # no `api_key:`, and no auth header on the wire
```

### 12.1 Provider definitions

Two keys are legal on every provider; every other key belongs to the `kind` rows
that name it.

| Key | Type | Required | Notes |
|---|---|---|---|
| `kind` | enum (below) | yes | selects the provider plugin and its config schema — and with it the rest of this definition's key set |
| `description` | string | no | documentation only (D54); legal on every kind |
| `api_key` | env-ref value | per kind (below) | never a literal (§4.3) |
| `base_url` | env-ref value | per kind (below) | never a literal (§4.3) |
| `headers` | map name→string (interpolable) | optional on the kinds that take it | extra request headers |
| `server_tools` | array of wire config objects | optional on the kinds that take it | tools the **provider** runs, appended to every request it serves (below) |
| kind-specific keys | per plugin | per kind (below) | validated against the plugin's published schema |

v0 provider kinds and the keys each one takes, `kind:` and `description:` aside:

| `kind` | Required | Optional |
|---|---|---|
| `anthropic` | `api_key` — **or** a `base_url` naming the gateway that holds one (below) | `api_key` (beside a `base_url`), `base_url`, `headers`, `server_tools` |
| `openai` | `api_key` — **or** a `base_url` naming the gateway that holds one (below) | `api_key` (beside a `base_url`), `base_url`, `headers`, `organization`, `server_tools` |
| `openai_compatible` | `base_url` | `api_key`, `headers`, `server_tools` |
| `azure_openai` | `base_url`, `api_key`, `api_version` | `headers` |
| `bedrock` | `region` | `access_key_id`, `secret_access_key`, `session_token`, `profile` |
| `vertex` | `project`, `location` | `credentials_json` |

`region`, `location`, `project`, `organization`, `profile`, and `api_version` are
plain strings and MAY be interpolated; the credential keys listed in §4.3 MUST be
env-ref values.

**`api_key` is required where the connection points at the vendor.** `anthropic`
and `openai` are the two kinds with a **default endpoint** — omit `base_url:` and
the connection reaches `https://api.anthropic.com` or `https://api.openai.com` —
and nothing but a key authenticates there. So on those two kinds, and on those
two alone, `api_key:` is required when `base_url:` is absent and optional when it
is present. A provider declaring neither is a compile error naming both repairs
(`missing-credential`, Decision
[D120](#d120-a-keyless-anthropic-or-openai-provider-names-its-endpoint)); a
provider declaring a `base_url:` and no key is a **gateway** connection, and a
compiled graph sends **no** authentication header at all for it — not an empty
one — because the gateway injects the vendor credential server-side. A gateway
that wants a token of its *own* takes it through `headers:`, whose values
interpolate (§4.3 class 2), so `authorization: "Bearer ${PROXY_TOKEN}"` reaches
the wire as a declared header rather than as a vendor credential. The other four
kinds are unchanged: `azure_openai` has no default endpoint and keeps all three
of its keys required, `openai_compatible` was already the fully flexible row, and
the two SDK-reached kinds authenticate through their cloud's own credential
chain.

**A kind's row is closed.** Beyond `kind:` and `description:`, a provider MAY
declare exactly the keys its own row names. A key that belongs to another kind's
row is a compile error naming the key and the kind — not an ignored setting
(Decision [D106](#d106-a-provider-kinds-key-row-is-closed)). `region:` on an
`anthropic` provider and `api_key:` on a `vertex` one are both refused: the
plugin never reads them, so accepting one would leave the author believing a
connection setting is in effect that is not, which is the silent no-op
[D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)
refuses for a key belonging to no row at all. This is the same shape as §11.4's
exact per-op parameter list, and it is decidable in one file for the same reason
— `kind:` is a literal in the same object — so the published schema enforces it
too (Appendix B).

The two SDK-reached kinds are where the rows differ most visibly from the rest:
`bedrock` and `vertex` take neither `base_url` nor `headers`, because a
connection made through a cloud SDK has no bare endpoint to point at and no
request the spec composes headers onto. A deployment that genuinely needs either
is reaching a compatible HTTP endpoint, which is what `openai_compatible` is for.

**`server_tools` is a provider-side tool suite.** A *server tool* runs on the
provider's side, inside the model call: the compiled runtime dispatches nothing,
and the results arrive woven into the assistant's turn. The key holds an array
of config objects written in **that provider's own wire vocabulary**, and the
runtime appends them to the `tools` of every request that provider serves, after
the agent's own tools.

```yaml
provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search_20250305
      name: web_search
      max_uses: 5
```

Each entry MUST carry a string `type`, which is the key the provider's
vocabulary is looked up under and is never interpolated. Every other key is the
provider's, travels verbatim, and is grammar 4.3 class 2 — non-secret provider
config, so its string values MAY interpolate. A key the curated table below
types as anything but a string is read at compile time and takes a literal: an
`${ENV}` there is a `type-mismatch` naming the rule, since interpolation
produces a string and the wire is given the value. A key the table pins to a
**single** value — the Messages wire's `name:`, and a nested object's `type:`
(`user_location:`'s `approximate`, `cache_control:`'s `ephemeral`,
`container:`'s `auto`) — takes that value: it is decided by the entry's own
`type:` and the service refuses any other spelling, so an `${ENV}` there is
`unexpected-env-ref` rather than a value read at process start. A closed set of
*several* values is an ordinary class 2 string and interpolates
(`search_context_size: ${SEARCH_DEPTH}`).

**The array a request carries is one namespace.** A suite is appended to the
`tools` of every request the connection serves, beside the agent's own tools, and
the provider surfaces refuse a request offering two tools under one name. So two
entries that reach the wire as one tool, and a server tool whose name an agent's
attached or synthesized tool already takes, are a compile error
(`tool-name-collision`) — §11.5's rule and §11.5's reason, reached from the
connection's side. On the Messages wire the name is the one the table pairs with
each dated `type`, which is what makes `code_execution_20250522` beside
`code_execution_20250825` two types with one name; the within-a-suite half holds
on every kind, since two entries of one array under one name are one tool twice
by the author's own reckoning. The agent-side half is stated over the Messages
wire **alone**: a Responses built-in is addressed by its `type` and a function
tool by its `name`, Chat Completions nests a function tool's name inside its own
object, and no table could say what a gateway keys its vocabulary on — so a
`tool.*` whose name an `openai` or `openai_compatible` connection's suite also
spells is not refused.

The checking is **two-tier**, and the constraint behind it is that a server tool
a vendor ships tomorrow must be usable the day it ships:

- a `type` in the compiler's **curated table** for that kind is validated
  strictly **against the fields that table models** — a mistyped value, a value
  outside a stated range or a closed set, a missing required field, or a
  constraint violation is an error naming the repair. On the Messages wire that
  closed set includes the required `name`, which the API pairs with each dated
  `type` and refuses a request that spells otherwise: `web_search_20250305` is
  `web_search`, `web_fetch_20250910` is `web_fetch`, and either
  `code_execution_*` is `code_execution`;
- a `type` outside it is a **warning** (`unknown-server-tool`) naming exactly
  what could not be verified, and the entry then travels to the wire as written.
  The composition still builds and still runs;
- a **key** outside the row of a `type` that is in the table is the same
  warning one level down (`unknown-server-tool-field`), and travels the same
  way. A row is keyed on `type` alone and is a snapshot of that tool taken at
  the compiler's release, so a parameter the vendor adds afterwards would
  otherwise block every author of a tool the table names — the treadmill again,
  at field granularity, with no entry-level way out. The compiler cannot tell
  such a key from a misspelling, so the diagnostic names the near miss where
  there is one and claims nothing where there is not.

The key is legal on the three kinds whose rows name it and is an error
(`unsupported-server-tools`) on `azure_openai`, `bedrock` and `vertex`, whose
wires this release has not been taught to carry it on. On `openai_compatible`
every entry is second-tier: a gateway may honour any vocabulary at all.

**The key can move the connection's wire, and §12.2's settings row moves with
it.** An `openai` provider that declares `server_tools:` speaks the Responses
API for all of its calls (D122), and two of §12.2's published `settings:` keys
have no equivalent there: `stop:` and `seed:` are Chat Completions'. A model
bound to such a provider that declares either is a compile error
(`unknown-key`) naming the wire, rather than a request the service refuses on
the first call — the same reasoning as the strict tier above. Both keys stay
legal on an `openai` provider that declares no suite.

**A suite belongs to a connection**, so every agent whose model resolves to that
provider holds it; scoping a suite to one agent is done by defining a second
provider. And because a failover route's members each name their own provider,
which tools were on offer depends on which member answered — a route whose
members declare different suites is a warning (`mismatched-server-tools`), not a
refusal. Decision
[D122](#d122-server-tools-are-provider-side-config-checked-in-two-tiers).

### 12.2 Model definitions

A model definition is **either** a direct binding **or** a route — never both
(Decision [D39](#d39-model-defs-are-direct-xor-route)).

```yaml
# models.yml
model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-6
  settings:
    max_tokens: 8000
    thinking: { budget_tokens: 4000 }

model.fast:
  provider: provider.anthropic
  id: claude-haiku-4-5
  settings: { temperature: 0.2 }

model.default:
  route: [model.smart, model.fast]
  route_on: [rate_limit, overloaded, timeout]
```

**Direct form:**

| Key | Type | Required | Notes |
|---|---|---|---|
| `provider` | `provider.*` ref | yes | |
| `id` | string (non-empty) | yes | provider-native model id; no env refs |
| `settings` | object | no | validated against the provider plugin's settings schema |
| `description` | string | no | |

**Route form:**

| Key | Type | Required | Notes |
|---|---|---|---|
| `route` | array of `model.*` refs, ≥ 2, **distinct** | yes | ordered fallback; members MUST be direct models (no nested routes). A repeated member is a fallback to the model that just failed — an inert entry, and a compile error (D80) |
| `route_on` | **non-empty** array of distinct enum values | no (default `[rate_limit, overloaded, timeout]`) | `rate_limit`, `overloaded`, `timeout`, `server_error`. `route_on: []` would declare a route that never fails over — write a direct model instead (D80) |
| `description` | string | no | |

Rules (PRD 5.9):

- `settings:` is the one **open** object in the logical layer: known keys are
  typed for editors (`temperature`, `top_p`, `top_k`, `max_tokens`, `stop`,
  `seed`, `thinking`, `reasoning_effort`, `parallel_tool_calls`), and
  plugin-specific keys are accepted here and checked by the compiler against the
  provider plugin's schema. `thinking:` on an OpenAI provider is a compile error
  at the model definition (Decision [D40](#d40-settings-is-the-only-open-object-in-the-logical-layer)).
- **No inline settings overrides on agents.** Different settings ⇒ another named
  model.
- **Capability checking**: every model an agent references MUST come from a
  provider declaring structured-output/tool-use capability (agents require
  structured output, PRD 5.2).
- **Route capability equivalence**: all members of a route MUST be
  capability-equivalent, so failover cannot silently break structured output.
  Failover conditions are infrastructure conditions only; content-based routing
  is out of scope — that is what graph edges are for.

---

## 13. Triggers

Triggers define what causes an execution to exist. The `triggers:` section
declares them, and the entrypoints of a project — the surface a deployment
exposes — are exactly the flows its **declared** triggers point at (PRD 5.11).
There is no `entrypoint:` key.

**One entry exists without being declared.** PRD 5.11 also makes `manual`
invocation universal, so **every flow is runnable from the CLI** —
`agent-compose run flow.<name> [--input k=v ...] [--session <key>]` — whether or
not a `manual` trigger names it. (The PRD states this for every flow *with an
input schema*, which is where `--input` has anything to bind; a flow with no
`inputs:` takes no `--input` arguments and is otherwise identical.) Writing the
`manual` trigger out, as [`examples/review-loop`](../examples/review-loop) does,
documents the intended entry and lets it carry a `description:` or a
`session_key:` remap; it neither enables nor restricts anything the CLI would
otherwise do
(Decision [D64](#d64-implicit-manual-invocation-is-a-cli-property-not-a-declared-trigger)).

Implicit invocation is **not** a trigger, and the difference is normative:

- it contributes no entry to the IR's trigger table and has no name;
- every rule that quantifies over triggers — input-binding compatibility
  (§13.1), session coherence (§11.3), `respond: sync` interrupt-freedom (§13.3),
  `event` source binding (§13.5) — quantifies over **declared** triggers only;
- a flow that no declared trigger names is still a complete, legal definition:
  it is reachable through the CLI, through `flow:` nodes, and through
  flow-as-tool attachment, so there is no unreachable-*flow* error. Unreachable
  *node* analysis is §7.8's: per flow, computed from that flow's `start`
  pseudo-node, and unaffected by which flows have triggers.

```yaml
# triggers.yml
triggers:
  cli:
    type: manual
    flow: flow.review_loop

  on_request:
    type: http
    flow: flow.review_loop
    path: /review
    method: POST
    input:
      goal: "payload.body.goal"
    session_key: "payload.headers['x-session-id']"
    respond: async
    callback: "payload.body.callback_url"
```

The `triggers:` section maps a trigger name (identifier) to a trigger object.
Every trigger has `type` and `flow`.

### 13.1 Common keys

| Key | Type | Required | Notes |
|---|---|---|---|
| `type` | `manual` \| `http` \| `schedule` \| `event` | yes | |
| `flow` | `flow.*` ref | yes | the execution's entry module |
| `input` | map flow-input-field→CEL over `payload` | `http`/`schedule`/`event` only | ILLEGAL on `manual` (§13.2); compile-checked against the flow's `inputs` |
| `session_key` | CEL over `payload` → string | no (on `manual`, defaults to `"payload.session"`) | supplies session identity for session-scoped stores and history (PRD 5.8) |
| `description` | string | no | |

**Input binding.** `input:` is legal on `http`, `schedule`, and `event` triggers
and ILLEGAL on `manual` ones
(Decision [D44](#d44-manual-triggers-carry-no-input-bindings)). Where it is
legal, it is compile-checked against the target flow's `inputs` (PRD 5.11):

- every field the flow declares as REQUIRED — one with no `default:`, §3.6 —
  MUST be bound; a field with a `default:` MAY be omitted and takes its default;
- a binding for a field the flow does not declare, or whose CEL result type the
  field cannot accept, is a compile error.

A `manual` trigger binds nothing, so there is nothing to check at compile time.
The identical check runs against the `--input k=v` arguments at run start,
against the same flow input schema (§13.2) — the one place this check is
deferred, for the same reason env-ref presence is (§4.3): the values do not exist
until the command runs.

### 13.2 `manual` (active in v0)

```yaml
cli:
  type: manual
  flow: flow.review_loop
```

CLI/SDK invocation: `agent-compose run flow.review_loop --input goal=...`. A
manual trigger MUST NOT declare `input:` — CLI arguments are validated directly
against the flow's input schema, at run start
(Decision [D44](#d44-manual-triggers-carry-no-input-bindings)). An unknown
argument name, a missing REQUIRED field, or a value that does not fit the
declared type fails the run naming the field; fields with a `default:` (§3.6) may
be omitted.

`session_key:` is legal and defaults to `"payload.session"`. The manual payload
has exactly one member — `payload.session`, the CLI's `--session <key>` — so a
manual trigger always has a session identity available, which is what makes the
session-coherence check of §11.3 satisfiable without a binding. Supplying the
value is a run-time requirement: `--session` is mandatory for a run whose flow
reaches a session-scoped store, and omitting it fails at start naming the store.

Declaring a manual trigger is optional: the same CLI entry, with the same
defaulted `session_key:`, exists implicitly for every flow (§13 preamble).

### 13.3 `http` (active in v0)

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `path` | string starting `/`, no whitespace | no | `/triggers/<name>` | route of the generated app; router parameter syntax (`/reviews/:id`) passes through unexamined; no env refs (§4.3) |
| `method` | `POST` \| `PUT` \| `GET` | no | `POST` | |
| `input` | map field→CEL over `payload` | no | — | |
| `respond` | `sync` \| `async` | no | `async` | |
| `timeout` | duration | `sync` only | `60s` | the response budget; ILLEGAL with `respond: async` (explicit or defaulted) |
| `callback` | CEL over `payload` → string | no | — | completion webhook; `async` only |
| `auth` | block; exactly one of `bearer:`/`hmac:` | no | — | how an inbound call is authenticated; absent leaves the route open |
| `callback_auth` | block; at least one of `bearer:`/`hmac:`, both legal | no | — | how a delivery identifies itself; requires `callback:`, and makes `callback_allow:` MANDATORY |
| `callback_allow` | non-empty list of URL patterns, each naming a scheme and a host | with `callback_auth` | — | where a callback may point; requires `callback:` |

`payload` shape: `payload.body` (decoded JSON object), `payload.query` (map of
string), `payload.headers` (map of string, lowercase names), `payload.path`
(string), `payload.method` (string).

**`payload.body` on a request that carries none.** `method: GET` is legal and a
`GET` has no body, so the object has to be defined for that case rather than
left to each generated app (Decision
[D117](#d117-payloadbody-is-an-empty-object-on-a-bodyless-request)). The
generated app decodes **no** body on a `GET`, and `payload.body` is then `{}` —
present and readable, so `has(payload.body.goal)` answers `false` instead of
erroring, and reading a member it does not carry fails that read like any other
absent value (§4.1, D110). A body-bearing method sent with an empty body
presents `{}` on the same rule, while a body that is present but is **not** a
decodable JSON object is rejected at request time (400) and starts no execution
— there is no partially-decoded payload for a binding to read.

Consequently a `method: GET` trigger MUST NOT read **through** `payload.body` in
any of its CEL — `input:`, `session_key:`, `callback:`. The object is `{}` on
every request such a trigger can receive, so `payload.body.goal` fails on every
one of them: a statically visible guaranteed runtime failure, refused at compile
time naming the trigger and the expression, the posture §7.6.3 takes on a
guaranteed dead end. Bind from `payload.query` instead, which is where a `GET`'s
parameters are. Reading `payload.body` *whole* stays legal and binds `{}`, where
a flow input field can accept it.

- `respond: async` returns an execution id immediately; the optional `callback:`
  webhook fires on completion.
- `respond: sync` blocks and returns the flow's outputs. A flow exposed
  synchronously MUST be statically **interrupt-free**: it MUST NOT reach a
  `human` node under the reachability relation of §7.7 — its own nodes, its maps'
  dispatch targets, the flows it instantiates, and the `flow.*` entries in the
  `tools:` list of any agent it reaches (PRD 5.11). On timeout expiry the
  response **upgrades to async** (HTTP 202 + execution id + status URL); the
  execution continues durably.
- `callback:` with `respond: sync` is a compile error: a synchronous response
  already carries the outputs, so the webhook would have nothing to deliver.
- `timeout:` with `respond: async` is a compile error, for the mirror-image
  reason: an async trigger has already responded, so there is no response for a
  budget to bound and the key changes nothing. `60s` is the default of the
  *effective sync* budget, not a value an async trigger carries
  (Decision [D81](#d81-timeout-is-illegal-on-an-async-http-trigger)).
- Generated apps expose `start`, `resume`, and `status` routes; resume payloads
  are validated against the interrupting `human` node's output schema (PRD 5.11).

**`auth:`, `callback_auth:` and `callback_allow:` are enforced by the generated
app.** A served trigger declaring `auth:` verifies its caller before it reads a
payload; a delivery carries the identity `callback_auth:` declares and goes only
where `callback_allow:` admits it. Everything the rest of §13.3 states in the
present tense is what a built project does, and the launch-time environment
check refuses a deployment missing any credential these blocks name (§4.3).

**Authenticating the caller: `auth:`.** v0's posture was "deploy behind your own
gateway". Webhook-style events make the generated app the thing a vendor calls
directly, so it verifies callers itself (PRD resolved q32). Auth is declared
**per trigger, never server-wide**, for the reason a built-in is declared per
node (§5.5): who may invoke this flow must be readable off the trigger that
exposes it.

```yaml
triggers:
  intake:
    type: http
    flow: flow.support
    respond: async
    callback: "payload.body.callback_url"
    auth:                            # exactly ONE of bearer | hmac
      hmac:
        secret: ${WEBHOOK_SECRET}    # required; env-ref value form only
        header: X-Hub-Signature-256  # default X-Signature
        algorithm: sha256            # sha1 | sha256 | sha512; default sha256
        encoding: hex                # hex | base64; default hex
        prefix: "sha256="            # default "" (empty)
    callback_auth:                   # at least one of bearer/hmac; BOTH legal
      bearer:
        token: ${CALLBACK_TOKEN}     # required; env-ref value form only
        header: Authorization        # default Authorization
        prefix: "Bearer "            # default "Bearer "
      hmac:
        secret: ${CALLBACK_SECRET}   # required; env-ref value form only
    callback_allow:
      - "https://hooks.example.com/*"
```

- **`bearer`** compares a static secret against a named header — `Authorization`
  with a `Bearer ` prefix by default. The comparison is **constant-time**: a
  byte-by-byte early return leaks the secret to a caller who can time it.
- **`hmac`** verifies a signature over the **raw request body bytes**, before any
  JSON decoding and after none of it — a re-serialized body is a different byte
  string and would fail every signature a vendor computed. `algorithm:`,
  `encoding:`, `header:` and `prefix:` together spell the GitHub-shaped family
  most webhook vendors speak. This comparison is **constant-time** as well, and
  the requirement is *not* the weaker one it looks like beside `bearer`'s: a
  check that returned on the first differing byte would hand a caller who can
  time it the expected digest for a body of their choosing, one byte at a time,
  and a forged request signed with a digest recovered that way is accepted
  without the caller ever holding the secret.
- **The secrets are `${ENV}` references** and nothing else, in both blocks and
  both directions: a literal is a compile error, because a secret never lives in
  the spec text (§4.3, Decision [D41](#d41-env-ref-forms-and-the-secret-field-list)).
  A reference that resolves to the **empty string** refuses the app at launch,
  naming the variable. §4.3's presence check counts an empty variable as set,
  which is right for a `base_url:` and wrong for a credential: an empty expected
  token compares equal to the empty token every anonymous caller can send, and an
  HMAC key of no bytes signs a body anybody can sign — so an unexpanded `${TOKEN}`
  in a launch wrapper is a route that is open and says nothing about it, which
  the refusal turns into one sentence on the first start.
- **Exactly one scheme.** A block declaring neither, and a block declaring both,
  are both compile errors: one request carries one credential, and a route that
  verified either would be exactly as open as its weaker half.
- **`header:` is one header name** — letters, digits, `_` and `-`, the form a
  provider's `headers:` keys take (§12.1) — and **`prefix:` carries no control
  character**. Outbound both resolved values are written onto a request as they
  stand, so a colon or a newline in either would forge a second header rather
  than name or introduce this one; inbound the same two values are the name a
  header is looked up by and the text expected ahead of the credential, and a
  name or a prefix no caller could have sent matches nothing. Anything else is
  `invalid-value`.
- **A header name is matched case-insensitively.** The name is recorded with the
  author's capitalisation and *looked up* without it: header names are
  case-insensitive by definition, HTTP/2 lowercases every one on the wire, and
  `payload.headers` above presents them lowercased for the same reason. So
  `header: X-Hub-Signature-256` finds the header a vendor sent as
  `x-hub-signature-256`, and an inbound check that compared the spelling would
  reject every genuine delivery over HTTP/2 while passing a `curl` that happened
  to preserve case. Outbound the resolved name is *written* as authored —
  capitalisation is the receiver's to read, never to match.
- Schemes whose signed payload is more than the body — Stripe's timestamped
  `t.body` with a tolerance window — are **deferred**, not forgotten: each is a
  vendor-specific shape, and genericizing them now is the support treadmill
  resolved q30 refused. Vendor presets can grow later as a curated table on q30's
  terms.

**`auth:` covers three routes, not one.** The `resume` and `status` routes are
per-execution, and resume *injects data into a parked run* — strictly more
sensitive than starting one. So both enforce the auth of **the trigger that
started that execution**: an execution an authenticated trigger began never
answers an unauthenticated poll or resume, and an execution a no-auth trigger
began keeps open routes (PRD resolved q32). `run` is untouched — no server, no
caller to verify.

**What an `hmac` trigger asks of those two routes** is worth spelling out,
because the two schemes do not cost a client the same thing. A `bearer`
trigger's three routes all take one header carrying one token. An `hmac`
trigger's do not: the signature is over **that request's own raw body bytes**,
so a `GET /executions/:id` of such an execution is signed over the *empty* body
a `GET` carries — `HMAC(secret, "")`, written under the trigger's `header:`,
`prefix:`, `algorithm:` and `encoding:` — and a `POST /executions/:id/resume` is
signed over the resume payload exactly as sent, byte for byte, never over a
re-serialization of it. That is the start route's rule applied to two more
routes rather than a second rule; what makes it worth writing down is that "the
auth of the trigger that started this execution" reads, for `hmac`, as a
per-request signature a poller has to compute rather than as a credential it
holds.

**Identifying the delivery: `callback_auth:` and `callback_allow:`.** Outbound
auth is opt-in and mirrors the inbound pair, so one verification recipe serves
both directions. `bearer` sends a static token on every delivery; `hmac` signs
the delivered body. Unlike `auth:`, **both together are legal** — a receiver that
checks a token and a receiver that verifies a signature are two receivers, and
one trigger may deliver to a receiver that does both (PRD resolved q33).

**Declaring `callback_auth:` makes `callback_allow:` mandatory**, and that is a
compile error rather than a warning
(Decision [D126](#d126-callback_auth-makes-callback_allow-mandatory)). The
callback URL comes from the trigger payload
([D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing))
and is attacker-controlled by construction, so a deployment careful enough to
authenticate its deliveries must not hand them — credential and all — to whatever
host a payload named. A URL outside the list is refused **when it is read**, at
parking or at settle rather than at start, and recorded as a refused delivery
rather than as anybody's failure.

**A delivery follows no redirect**, which is the same guarantee read one step
further. The list is matched against the URL the trigger produced, so a receiver
answering `3xx` must not be able to pass this report — with its signature, and a
`callback_auth: bearer` token written under the header name its author chose — on
to a `Location:` the list admits nowhere: what a cross-origin redirect strips is
a fixed list of standard credential headers, never a name a composition chose and
never the body's signature. The quieter
half is that a `301`, `302` or `303` rewrites the request to a bodyless `GET`, so
an allowlisted receiver redirecting to itself would answer `2xx` to a request
carrying no report at all. A `3xx` is a failed attempt like any other non-2xx
status (`docs/durability.md` §3.7); a receiver that has moved is a `callback:`
naming where it moved to.

**A callback URL that carries userinfo is refused**, which is the same guarantee
read one step *back*. The list is matched against the URL as text, and userinfo —
the `user:pass@` an authority may put before its host — is where the text and the
destination part company: `http://hooks.example.com:9000@attacker.test/hook`
begins with `http://hooks.example.com:`, so the entry
`http://hooks.example.com:*/hook` — what an author writes for a receiver whose
port the operating system chose — admits it while the host the request reaches is
`attacker.test`. So a URL with an `@` in its authority is a refused delivery like
any other, recorded and never sent, and it is refused **whether or not the
trigger declares a list**: the two JavaScript runtimes a built project runs under
disagree about such a URL — one drops the userinfo and delivers to the host after
the `@`, the other refuses to construct the request at all — and a wire contract
that turned on which one `serve` found would be no contract. An `@` after the
authority, in a path or a query, is an ordinary character and means nothing here.

**A trigger with a `callback:` and no `callback_auth:` is a documented test
posture**: it signs nothing, claims nothing, and may POST anywhere. That is the
shape a localhost receiver wants, it needs no allowlist, and it is stated here
because shipping it is a choice rather than an oversight. `http` URLs stay legal
in the allowlist for the same reason. Private-IP and DNS-rebinding hardening is
**out of v1 scope**: SSRF-hardened egress is the gateway's job in the deployments
that need one.

**Allowlist patterns** (Decision
[D127](#d127-a-callback-allowlist-entry-is-a-wildcard-url-and-the-delivery-wire-is-fixed)):
each entry is an absolute URL whose scheme is `http` or `https` and which names
a host, with `*` meaning "any run of characters" — one wildcard kind, matched
against the **whole** callback URL string, with no `**` distinction (a URL is
not a path tree). An entry naming no scheme, an unsupported scheme, no host at
all (`https:///deliveries`), or an empty or whitespace-bearing value is a
compile error, and so is an **empty list**: an allowlist that admits
nothing refuses every delivery, which is a webhook that can never fire. The
scheme is written **lowercase**, because an entry is matched as written: a URL
scheme is case-insensitive to a browser and `HTTPS://hooks.example.com/*` is
still a string no lowercase callback URL matches, so it is refused with the
spelling as the repair rather than admitted as an allowlist that admits nothing.
`callback_auth:` or `callback_allow:` on a trigger with **no `callback:`** is a
compile error too, the mirror of `timeout:` on an async trigger
([D81](#d81-timeout-is-illegal-on-an-async-http-trigger)): the key describes a
delivery this trigger never makes.

**Write the host out, and read a wildcard in it for what it is.** `*` is *any*
run of characters, and it crosses `/` and `?` like any other — there is no
delimiter it stops at. So a wildcard reaching the host constrains no host:
`https://*.hooks.example.com/*` is matched by
`https://attacker.test/collect?x=.hooks.example.com/y`, where the leading `*`
consumed a host, a path and a query on its way to the literal after it, and
`https://hooks.example.com*` is matched by
`https://hooks.example.com.evil.test/collect`, where the trailing one simply
continued the name. Both are **legal entries** that admit far more than their
author means, so an allowlist that is a guarantee rather than a ceremony is one
whose entries write their hosts out — `https://hooks.example.com/*`,
`https://hooks.example.com:9000/*` — with a second subdomain getting a second
entry.

The compiler refuses the *shape* and not the *breadth*, and the difference is a
question the PRD owns. `https://*.hooks.example.com/*` is the entry an author
arriving from any other allowlist writes first, and there is a reading of `*`
under which it means what they intend: a wildcard that stops at `.`, `:`, `@`
and `/` inside the authority constrains the host to one label of a named tree,
and neither match above survives it. That reading is a **second wildcard kind** —
one meaning inside the authority, another after it — which is a language
decision this grammar has not taken. Refusing the entry until it is taken would
be taking it: on a trigger whose `callback_auth:` makes the list mandatory, the
only other compiling repair is dropping the outbound auth, which is the posture
[D126](#d126-callback_auth-makes-callback_allow-mandatory) exists to prevent. So
the entry compiles, the reading it compiles under is stated here, and a resolved
question that bounds the wildcard narrows a meaning rather than unbanning a
shape (Decision [D127](#d127-a-callback-allowlist-entry-is-a-wildcard-url-and-the-delivery-wire-is-fixed)).

**The delivery wire.** A callback fires on lifecycle events — every quiescence
that opened new pauses, and settle — carrying the status route's report plus
delivery metadata (PRD resolved q34, q35). A quiescence is the moment every
branch of the execution has parked or finished, so a `map` over a flow with
`human` nodes is **one** delivery listing all of its pauses rather than one per
item. Every delivery carries these headers, and they are normative:

| Header | Value |
|---|---|
| `X-AgentCompose-Event` | `parked` or `settled` |
| `X-AgentCompose-Delivery` | the delivery id, `<execution_id>:<ordinal>` |
| `X-AgentCompose-Ordinal` | the event ordinal, an integer |
| `X-AgentCompose-Timestamp` | ISO-8601 |
| `X-AgentCompose-Signature` | `sha256=<hex hmac-sha256 of the body>` — with `callback_auth.hmac` only |

With `callback_auth.bearer`, the configured `header:` carries `prefix:` followed
by the token — and that header may **not** be one the delivery already writes:
the `X-AgentCompose-` namespace belongs to the wire contract, and a delivery is
a POST of a JSON body to the host the allowlist admitted, so it writes
`Content-Type`, `Content-Length` and `Host` on its own request too. A
`callback_auth.bearer.header:` naming any of them is a compile error
(`invalid-value`). A token written under a name the delivery already writes
arrives joined to that value or in place of it: a receiver following this table
then fails its signature check on every legitimate delivery — or passes on one
whose signature it never read — and a receiver reading a `Content-Type` that is
a credential answers 415 and never sees the report at all. The three transport
names are matched **whole** (`X-Content-Type` and `Content-Type-Signature` are
headers of the receiver's own and stay legal); the namespace is matched as a
prefix, so a sixth `X-AgentCompose-` header on the wire needs no second rule. The
reservation is **outbound only**: an inbound `auth:` may name
`X-AgentCompose-Signature` freely, which is exactly how a trigger that *receives*
another deployment's callbacks verifies them.

Outbound `hmac:` takes **no** keys but `secret:`: signing is fixed at
HMAC-SHA256 written in hex, so one receiver-side recipe verifies every
agent-compose deployment. Deliveries are journaled and at-least-once with bounded
retry, so a parking delivery and a settle delivery **can arrive out of order**:
receivers order by `X-AgentCompose-Ordinal`, never by arrival, and dedupe on
`X-AgentCompose-Delivery` (PRD resolved q35).

The retry schedule, what a refused or exhausted delivery leaves behind, and what
a restarted `serve` picks up are `docs/durability.md` §3.7's, which is normative
for the delivery ledger the way §13.3 is normative for the wire.

### 13.4 `schedule` (RESERVED grammar — parsed and validated, no-op in v0)

```yaml
nightly:
  type: schedule
  flow: flow.triage
  cron: "0 3 * * *"
  timezone: UTC
  input: { scope: "'full'" }
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `cron` | string, 5-field POSIX cron | yes | validated for shape at compile time, so no env refs (§4.3) |
| `timezone` | IANA tz name | no (default `UTC`) | no env refs (§4.3) |
| `input` | map field→CEL over `payload` | no | `payload.scheduled_at` (RFC 3339 string), `payload.trigger` (name) |

Everything is parsed, type-checked, and carried into the IR; nothing schedules an
execution in v0 (M3 owns the scheduler process — PRD 5.11).

### 13.5 `event` (RESERVED grammar — parsed and validated, no-op in v0)

```yaml
ingest:
  type: event
  flow: flow.triage
  source: bug_reports          # logical name bound in deploy/<target>.yml
  input: { report: "payload.body" }
  session_key: "payload.body.reporter"
  dedupe_key: "payload.id"
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `source` | identifier | yes | logical name; bound to infrastructure by `event_sources:` (§14.4) |
| `input` | map field→CEL over `payload` | no | `payload.id`, `payload.body`, `payload.attributes`, `payload.source` |
| `dedupe_key` | CEL over `payload` → string | no (default `payload.id`) | inbound at-least-once dedupe (PRD 5.11) |

A `source:` not defined in the active target's `event_sources:` is a compile
error naming the target — target-dependent, like backend alias resolution
(§11.3), and satisfied vacuously under `local`, which runs no consumer process
and binds no event infrastructure (§14,
[D87](#d87-local-is-a-reserved-target-and-deploylocalyml-carries-no-storage_backends)).

---

## 14. Deploy layer

`deploy/<target>.yml`, selected by `--target <name>`, never imported. Only this
layer forks per environment (PRD 5.8 per-target invariant); `agents/`, `flows/`,
`stores/`, `tools/` never do.

**Checkpointing is a property of the target, not a spec construct.** v0 declares
no grammar for configuring a checkpointer — that lands with the distributed
target in M3 (PRD 5.10). The rule the grammar depends on is fixed instead:
`--target local` runs with an in-memory checkpointer and is therefore **not**
durably checkpointed; every other target is. Exactly one static check keys off
this — `detach: true` (§8.6 rule 7) — which makes it target-dependent in the same
way backend alias resolution is (§11.3). Commands that name no target
(`agent-compose validate main.yml`) resolve the target as `local`; target-dependent
rules are then checked against `local`, and `validate --target <t>` checks them
against `<t>`
(Decision [D59](#d59-checkpointing-is-a-target-property-and-detach-is-checked-per-target)).

**`local` is a reserved, built-in target.** It is the target when `--target` is
omitted, it runs with an in-memory checkpointer (above), and it substitutes
SQLite/local disk for **every** store unconditionally — PRD 5.8's zero-infra
guarantee. Four consequences follow, and they are what make `deploy/local.yml`
well-defined rather than a file the grammar half-recognizes (Decision
[D87](#d87-local-is-a-reserved-target-and-deploylocalyml-carries-no-storage_backends)):

- `deploy/local.yml` is OPTIONAL. When present it MAY declare `hub:`,
  `placements:` and `event_sources:`. The first two are **live** static grammar
  (§14.1, §14.2) and are checked under every target including `local`, which is
  what a mesh looks like on one machine: this process is the hub, and an
  `agent-compose worker` beside it is the spoke. `event_sources:` is reserved
  grammar (§15), parsed, type-checked, and carried into the IR under every
  target, so it is not inert there either.
- It MUST NOT declare `storage_backends:`. That section is *active* grammar which
  `local` overrides unconditionally: no alias and no per-kind default is ever
  consulted, so the block could only be an inert key whose author expected a
  substitution — a compile error naming the file and the target, not a silent
  no-op ([D61](#d61-else-takes-the-literal-true),
  [D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)).
  A store that wants a real backend locally is a `--target` of its own.
- Under `local`, the two **target-dependent binding checks are satisfied
  vacuously**: a store's `backend:` alias needs no definition (§11.3) because
  `local` resolves none, and an `event` trigger's `source:` needs no
  `event_sources:` entry (§13.5) because `local` is one process running no
  consumer. This is what lets a project with production infrastructure in
  `deploy/staging.yml` still `validate` and `run` locally with no infrastructure
  at all, which is the point of the zero-infra guarantee.
- `--target <name>` for any name other than `local` REQUIRES `deploy/<name>.yml`
  to exist: a missing file is a compile error naming the expected path, never a
  silent fall-back to built-ins.
- `--target local` with no `deploy/local.yml` is the zero-config path and is not
  an error.

```yaml
# deploy/staging.yml
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}
  public_url: "https://hub.example"

placements:
  mac:
    members: [agent.signer, tool.xcodebuild]
    description: the machine with the signing keys
  gpu:
    members: [agent.embedder]

storage_backends:
  defaults:
    kv: { provider: redis, url: "${REDIS_URL}" }
  aliases:
    docs_db: { provider: chroma, url: "${CHROMA_URL}" }

event_sources:
  bug_reports:
    kind: redis_streams
    url: ${REDIS_URL}
    stream: bug-reports
    consumer_group: agent-compose
```

### 14.1 `placements`

A **placement** is a logical name a worker claims at an authenticated join, and
the components that claim runs. Nothing here is an address: which physical
machine satisfies a claim is decided by whoever shows up, so a placement is
*capability affinity* — the machine with the signing keys, the GPU, the one
licensed tool — rather than load assignment (PRD resolved q38, Decision
[D128](#d128-a-placement-is-a-named-claim-and-its-members-are-components)).

Keys are identifiers (§2.1).

| Key | Type | Required | Notes |
|---|---|---|---|
| `members` | list of `agent.*` / `tool.*` addresses | yes | non-empty, each named once; every address MUST resolve in the composition |
| `description` | string | no | documentation only (D54) |

```yaml
placements:
  mac:
    members: [agent.signer, tool.xcodebuild]
    description: the machine with the signing keys
  gpu:
    members: [agent.embedder]
```

The rules, each a compile error (Decision
[D129](#d129-placement-members-are-agents-and-tools-disjoint-and-colocated-with-what-attaches-them)):

1. **`members:` is required and non-empty**, and names each component **once**.
   A placement with no members is a claim nothing is ever dispatched under; a
   repeated entry says what the first entry said, and is refused the way a
   repeated `tools:` entry is (§5.4).
2. **v1 members are `agent.*` and `tool.*`.** A `flow.*` member is refused with
   a message naming the deferral: a flow is a subgraph the hub schedules, and
   placing one is out of v1's scope, named (PRD resolved q44). Place the nodes
   it reaches instead.
3. **Placements are DISJOINT.** One component named by two placements is an
   error naming both: two claims are two answers to which worker runs it, and
   the choice is the author's rather than the hub's.
4. **An attachment colocates with the agent that attaches it.** The whole
   generated artifact reaches every worker (PRD resolved q40), so a placement
   decides which *process* runs a node rather than which code exists there — and
   an agent's `tools:` list is the one construct that runs another component
   inside the agent's own process instead of handing it back to the hub's
   scheduler. Two forms, one rule:
   - an attached **`tool.*`** is called from inside the agent's tool loop, so a
     tool whose placement differs from that of an agent attaching it is an
     error, and so is a placed tool attached to an agent with **no** placement,
     which would run on the hub. A tool with the same placement, or with none,
     is fine.
   - an attached **`flow.*`** starts an instance in that same loop (§5.4). The
     flow carries no placement of its own — placing one is deferred — but every
     `agent.*` and `tool.*` its instance **reaches** (§7.7 clauses 1 and 2) runs
     where the agent runs, so each of them is held to the same rule. A placement
     reachable only that way would be one the compiler accepted and the
     deployment ignored.

   A placed component's own placement governs it wherever it is reached without
   an attaching agent: a `function:` node (§8.4), an `agent:` node, or a flow
   instantiated by a `flow:` node (§8.5), all of which the hub schedules.
5. **A component in no placement executes on the hub.** That is the default and
   is never a diagnostic: `placements:` names the exceptions.

`--target local` needs no placement at all, and admits them: a `local` target
with placements is the hub and its workers on one machine, which is how a mesh
is developed.

**This section replaced a reserved one.** Until the claims model was resolved,
`placements:` was keyed by component address and carried `runtime:
isolated|colocated` plus a reserved `network:`. Reserved grammar is parsed,
checked, and carried into the IR, and *may be re-shaped before its first
execution* — that is what reserved means (§15). Decision
[D128](#d128-a-placement-is-a-named-claim-and-its-members-are-components) records
why the old shape could not survive the resolution.

### 14.2 `hub`

The **hub** is the process that owns the graph: scheduler, journal, wait board
and triggers (PRD resolved q37). Workers dial out to it and never the other way
round, which is what makes a laptop behind NAT a first-class placement. This
block is what a deploy file says about it.

| Key | Type | Required | Notes |
|---|---|---|---|
| `join_token` | `${ENV}` reference | conditionally — see below | the bearer credential a worker presents at join |
| `public_url` | absolute `http`/`https` URL | no | the ingress base every URL this deployment hands out derives from |

```yaml
hub:
  join_token: ${MESH_JOIN_TOKEN}
  public_url: "https://hub.example"
```

The rules (Decision
[D130](#d130-the-hub-block-and-the-conditional-join-token)):

1. **`join_token:` takes an `${ENV}` value-form reference and nothing else.** A
   literal is a compile error, like every other credential in the grammar
   (§4.3, PRD resolved q32). One kind only: bearer.
2. **`join_token:` is REQUIRED when `placements:` is non-empty**, and its
   absence is a compile error naming both repairs — declare the token, or remove
   the placements. A `hub:` block with neither key, or one present without any
   placements, is legal: `public_url:` is useful alone.
3. **`public_url:` is an absolute URL naming a host**, its scheme `http` or
   `https` written lowercase, with **no wildcard**: this is the deployment's own
   base rather than an allowlist pattern, so a `*` in it is a character in a
   hostname that resolves nowhere. Shape errors follow §13.3's conventions,
   which is the other URL surface an author meets.
4. **Unknown keys are errors**, as everywhere outside a plugin-config object
   (D50).

**What the token means is the v1 trust model, and it is worth stating plainly:
holding the join token is being trusted with the mesh** — the whole artifact,
the right to claim any placement, and the journal's effect stream (PRD resolved
q38). The per-placement environment manifest checked at join is self-reported
presence, not proof: proving a secret would mean sending it, which resolved q41
exists to forbid. Per-placement credentials are an additive later hardening,
not something this one key pretends to be.

**The protocol these two sections describe is normative in
[`docs/distributed.md`](distributed.md)**, which fixes the wire contract the
runtime is written against. The keys are live static grammar today: every rule
above is enforced by `validate`, and the `worker` verb that reads them lands
with the runtime.

### 14.3 `storage_backends`

| Key | Shape | Notes |
|---|---|---|
| `defaults` | map kind (`kv`/`vector`/`blob`) → backend config | per-kind fallback |
| `aliases` | map alias identifier → backend config | the slots stores name via `backend:` |

A backend config requires `provider` and accepts provider-specific keys (checked
against the storage plugin's published schema at compile time); connection
strings and credentials are env-ref values only (§4.3).

| Kind | v0 `provider` values |
|---|---|
| `kv` | `memory`, `sqlite`, `redis`, `postgres` |
| `vector` | `sqlite_vec`, `chroma`, `pgvector`, `qdrant` |
| `blob` | `local_fs`, `s3`, `gcs` |

Capability checks apply at the alias definition: a `vector` store bound to a
non-vector-capable provider is a compile error (PRD 5.8).

### 14.4 `event_sources` (RESERVED — parsed and validated, no-op in v0)

Maps the logical `source:` names used by `event` triggers to infrastructure.

| Key | Type | Required | Notes |
|---|---|---|---|
| `kind` | `redis_streams` \| `sqs` \| `nats` | yes | consumer plugin |
| kind-specific | per plugin | per plugin | e.g. `url`, `stream`, `consumer_group`, `queue_url`, `subject` |

---

## 15. Reserved grammar summary

Reserved constructs are **fully specified, parsed, type-checked, and carried into
the IR**, but execute as no-ops in v0. Using one is never an error; relying on
its runtime effect is a documented no-op (PRD 5.10, 5.11).

| Construct | Status in v0 | Lands in |
|---|---|---|
| `event_sources` | parsed + validated, no-op | M3 |
| `triggers.<t>.type: schedule` | parsed + validated, no-op | M3 |
| `triggers.<t>.type: event` | parsed + validated, no-op | M3 |

**Three constructs have left this list, and each left it a different way.**

`human` nodes were here until M2: a compiled project really pauses, publishes
the question, and resumes (§8.7), and its waits now survive a restart as well —
a resumed execution re-parks under the same wait id and reads its answers out of
the journal (`docs/durability.md`).

**`placements:` left it by being re-cut**, which is the one departure that is
not a runtime landing, and it is what the reserved posture is *for*. The old
shape keyed placements by component address and gave each a `runtime:
isolated|colocated` and a reserved `network:`. Resolving how a machine becomes a
placement (PRD resolved q38) settled a different model — a named claim a worker
asserts at an authenticated join — and the address-keyed shape could not express
it. Reserved grammar is fully specified and carried into the IR precisely so
that it *may be re-shaped before its first execution*: nothing had run, so
nothing was broken, and the alternative would have been shipping a second
placement grammar beside a dead first one. §14.1 and §14.2 are the result;
Decision [D128](#d128-a-placement-is-a-named-claim-and-its-members-are-components)
records the re-cut, and the keys are **live static grammar** — every rule about
them is enforced by `validate` today, while the `worker` verb that reads them
lands with the runtime. `crates/compose-core/tests/placement_surface_inertness.rs`
is what holds that middle state honest: it asserts that no placement or hub
material reaches a generated project yet, pins the sentences that say so, and
enumerates what the runtime pass must unwind.

**The three authentication keys were here until the http-native events pass**,
and they are the ones that read differently from every other row, which is why
their retraction is bound rather than remembered. A no-op `schedule` runs
nothing, which is visible the first morning it does not fire; a no-op `auth:`
would **serve every caller** and be indistinguishable, from outside, from a
guarded route — a wrong claim about a security control is worse than a missing
one, in both directions. So `crates/compose-core/tests/trigger_auth_surface.rs`
now asserts the opposite of what it used to: that an authenticated trigger's
material really does reach the generated project, that its credentials reach the
environment manifest `src/env.ts` builds, and that the documents which once
called the surface inert say it is enforced. A change that made these keys inert
again fails there rather than shipping a `docs triggers` that promises a
guarantee the app does not keep.

The environment manifest is the half of that with no sentence to bind:
`crates/compose-core/src/codegen/env.rs` walks `ir.triggers` beside the
definitions and the deploy layer, so an authenticated trigger's four `${ENV}`
references — `auth.bearer.token`, `auth.hmac.secret`,
`callback_auth.bearer.token`, `callback_auth.hmac.secret` — are in the list
`readEnvironment()` checks at process start. §4.3's promise is that the
variables a deployment needs are computable from the artifact statically, and a
runtime reading `process.env.WEBHOOK_TOKEN` that the walk did not know about
would let a deployment missing the variable start clean and then refuse every
real call.

---

## Appendix A — Decisions

Every entry is a place the PRD left the *grammar shape* open. Semantics stay
inside what the PRD settles; the cross-reference names the section each decision
must remain consistent with.

Two entries are different in kind and are labelled **PRD-extending**:
[D37](#d37-agent_access-narrows-the-synthesized-store-tool-surface) and
[D51](#d51-agents-carry-max_tool_iterations-default-8) each added a key answering
a question the PRD did not ask. Both went through CLAUDE.md's PRD discipline as
Open Questions and both were **ratified** into the PRD's Resolved Questions log
(PRD §9.13 and §9.14), so they now stand on the same footing as every other
entry. No other entry in this appendix introduces a construct the PRD does not
already imply.

### D1. Imports are entrypoint-only and non-transitive

Only the entrypoint may declare `imports:`; an imported file that declares
`imports:` is an error. **Rationale**: PRD 5.1 requires that "what is in this
graph" be unambiguous. A flat, single-level list is the whole answer to that
question, is greppable, and makes resolution order irrelevant. *PRD 5.1.*

### D2. Singleton sections are declared in exactly one file

`state:`, `triggers:`, and `defaults:` may each appear in at most one file of a
composition; two occurrences are an error naming both files. **Rationale**:
implicit cross-file merging of a section is exactly the action-at-a-distance the
PRD rejects for deploy overrides; a single site keeps the merged IR trivially
explainable. *PRD 5.1, 5.7, 5.11.*

### D3. Spec files and deploy files are disjoint document kinds

Deploy keys are illegal in spec files and definition/spec keys are illegal in
deploy files. **Rationale**: mechanically enforces the per-target invariant —
only the deploy layer forks per environment. *PRD 5.8, 5.10.*

### D4. `version` is required in the entrypoint and deploy files, optional elsewhere

When present in an imported file it MUST match the entrypoint's. **Rationale**:
one authoritative version per composition, with a cheap redundancy check for
files copied between projects; per-file requirement would be noise. *PRD §9.5.*

### D5. One identifier class, lowercase snake_case

Definitions, node ids, channels, fields, tags, and trigger names share the
identifier grammar of §2.1. **Rationale**: one error message, no mangling in any
codegen target, and no case-collision ambiguity across languages. *PRD 5.1, 5.12.*

### D6. Field-map shorthand only at declaration surfaces

The top level of a schema surface is a field map; nested objects are written
explicitly with `type: object` + `properties`. **Rationale**: a nested mapping is
otherwise ambiguous with a type node — the discriminating keys (`type`, `enum`,
`discriminator`) would have to be treated as reserved field names, which is worse.
*PRD 5.2.*

### D7. Properties are required by default; `optional:` lists the exceptions

**Rationale**: structured-output engines behave best with fully-required schemas,
and edge schema compatibility is decided on required sets; making the common case
implicit keeps the surface compact. *PRD 5.2, 5.3.*

### D8. Objects are closed

No `additional_properties` knob exists. **Rationale**: open objects break
discriminated-union narrowing and make edge compatibility undecidable; PRD 5.6's
narrowing guarantee requires closed variants. An escape hatch would be used
accidentally. *PRD 5.2, 5.6.*

### D9. Enums are string-only

`enum:` members are unique non-empty strings; `enum` implies `type: string`.
**Rationale**: routing exhaustiveness (PRD 5.3) and discriminator tags are
defined over string variants; numeric enums add no expressiveness and complicate
guard typing. *PRD 5.3.*

### D10. `max_items` is required on result schemas and fanned-out arrays

Required inside every result schema — the surfaces §3.5 lists, which include an
inline `exec:`/`http:` node's `output`
([D96](#d96-an-inline-exechttp-nodes-output-is-a-result-schema)) — and on any
array a `map.over` resolves to; optional on every input surface.
**Rationale**: PRD 5.6
makes fan-out bounding mandatory and enforces it at structured-output validation
so the model *cannot* return more. Extending the requirement to every result
schema — one syntactic rule instead of "wherever a map might later consume it" —
also keeps every edge payload finite, which is what makes an edge safely
promotable to a network boundary (PRD 5.7, 5.10). Inputs are exempt because
their bound is the producer's. *PRD 5.6, 5.7, 5.10.*

### D11. Discriminated union shape

`discriminator` + `variants` with ≥ 2 variants; variants MUST NOT redeclare the
discriminator field (the compiler synthesizes it as a constant); unions are legal
as array `items:` or as a property type, never as a top-level surface.
**Rationale**: matches the PRD 5.6 sketch, keeps the agent's contract legible in
its own schema, and guarantees every declaration surface is an object so routing
fields have names. *PRD 5.6.*

### D12. `pattern` is RE2; `format` is a closed list

**Rationale**: patterns must mean the same thing in the Rust validator, in
generated JS validation, and in provider-side structured-output engines — RE2 is
the common denominator, and an open `format` vocabulary would silently vary.
*PRD 5.2, 5.12.*

### D13. `prompt` is required and literal

Agents MUST declare a non-empty `prompt:`; no interpolation syntax exists.
**Rationale**: the PRD's §6 sketch omits prompts, but an agent with no
instructions is not a specifiable component; literal prompts keep the "spec
contains no executable code" posture and stay diffable. Dynamic content travels
through the input schema, which is serialized into the user turn. *PRD 5.5, 5.2;
§6 of the PRD is explicitly illustrative.*

### D14. String-in agents bind with a scalar `input:` at the node

Omitting `input:` on an agent means a single unnamed string input, which MUST be
bound at the node with the scalar form (`input: "input.goal"`). **Rationale**:
PRD 5.2 keeps string-in as the default, but name-based wiring needs a name; a
one-line explicit binding is the smallest honest answer and avoids magic
"single-input flow" rules. *PRD 5.2, 5.7.*

### D15. Node-level `input:` is the one binding mechanism

Every node kind (except `map`, which binds per item inside its block) accepts
`input:`; on `flow:` nodes it is required whenever the subflow declares an input
field with no `default:`, and unbound fields do **not** fall through by name
([D68](#d68-flow-node-bindings-are-total-nothing-falls-through-a-module-boundary)
owns that rule and §8.0 states the two resolution chains).
**Rationale**: PRD 5.1 calls for Terraform-style module bindings and PRD 5.7
requires subgraphs to receive parent state only through explicit bindings. Using
one keyword for both instead of a separate `bindings:` keeps the surface small.
This is *not* the deferred data-edge feature: bindings read `input`/`state`/
`execution`, never another node's output. *PRD 5.1, 5.7.*

### D16. `writes:` remaps output field → channel

Keys are output field names, values are declared channels; a remapped field is
not also written to its same-named channel. **Rationale**: the exact two failure
modes PRD 5.7 names (collisions, cross-boundary renames), no more. *PRD 5.7.*

### D17. Edge selection is multicast with `else`, not first-match-wins

All edges whose guard passes fire; an `else: true` edge fires only when no guarded
sibling fired. **Rationale**: LangGraph's superstep model makes concurrent
branches natural, and PRD 5.3 needs an "optional default" for exhaustiveness. A
first-match rule would make static parallel branching inexpressible; a bare
unguarded edge cannot serve as the default because it must remain unconditional.
Concurrent branches obey the same reduced-channel rule as maps; what they do
after the fork — convergence, write ordering, termination — is §7.6
([D69](#d69-execution-is-stepwise-and-convergence-is-a-per-step-join-over-taken-branches),
[D70](#d70-end-retires-a-branch-and-a-flow-instance-ends-at-quiescence),
[D72](#d72-concurrent-writes-are-applied-in-a-canonical-order)). *PRD 5.3, 5.6.*

### D18. Exhaustiveness is computed over enum-typed output fields

A node whose outgoing guards compare an enum field of its own output must cover
every variant or declare `else:`;
[D82](#d82-exhaustiveness-and-exclusivity-read-one-closed-set-of-guard-shapes)
fixes which guard shapes are read and what "cover" means. **Rationale**: this is
the decidable core of PRD 5.3's promise; guards over non-enum values are
unconstrained and simply require an `else:` or an unconditional edge to avoid a
dead end. *PRD 5.3.*

### D19. `max_iterations` semantics and the escape-edge rule

The budget counts traversals of one edge within one flow instance; an exhausted
edge is untraversable; the source node of a bounded edge MUST have an outgoing
edge that leaves the SCC *and* is unconditional or `else: true`.
**Rationale**: PRD 5.4 requires bounded cycles, but a bound with no escape merely
converts an infinite loop into a runtime dead end. Requiring merely *an* exit
edge does not remove that dead end — if the exit is guarded, the pass that
exhausts the budget with a false guard takes no edge and fails on §7.3 rule 7 —
so the rule names the two edge forms that are guaranteed to fire when the budget
runs out. `else: true` is the usual spelling, and it doubles as the exhaustive
route for the enum variant that leaves the loop (§7.3.1), so the constraint costs
an author one keyword, not an extra edge. Per-instance counting keeps
`map`-dispatched subgraph instances independent. *PRD 5.4, 5.6.*

### D20. The policy resolution chain has exactly four levels

Flow-node `policy:` override > node > `defaults:` > built-in `fail`; when a
nesting chain sets one field at several instantiation sites, the outermost wins
([D79](#d79-the-outermost-instantiation-site-policy-wins)). Two refinements sit
on top of the four levels:
[D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level) exempts
a `human` node from `timeout` and `retry` at every level, leaving its `on_error`
to resolve normally, and
[D103](#d103-fallback-is-a-node-level-on_error-form-only) confines the `fallback`
form of `on_error` to level 2.
**Rationale**: PRD 5.5 names exactly these four; the override is placed at the
*instantiation site* so a caller can harden a reused module — the only reading
under which "flow override" beating a node's own declaration makes sense. A
flow-definition-level default layer was considered and rejected to keep the chain
as settled. *PRD 5.5.*

### D21. `on_error` strategies and `fallback` targets

`fail` | `skip` | `{ fallback: <flow-local node id or end> }`, applied after
retries; `skip` suppresses writes and then routes through §7.3 with the
skipped node's own output reading as false
([D97](#d97-a-skip-changes-guard-values-not-the-routing-algorithm) owns exactly
what a skip changes). The third form is a node's own key only —
[D103](#d103-fallback-is-a-node-level-on_error-form-only) refuses it in
`defaults:` and in a flow node's `policy:`, where no flow is named for the target
to be local to.
**Rationale**: PRD 5.5 lists the strategies; restricting `fallback` to a
flow-local node keeps error routing visible to reachability analysis instead of
creating an invisible cross-module edge. `end` is admitted because "give up and
finish this instance" is the commonest error route and is already expressible as
an edge; `start` is not, because re-entering a flow from an error has no defined
input. `human.on_timeout` (D52) accepts exactly the same targets — the two
control-transfer positions are deliberately identical (§2.4). *PRD 5.5.*

### D22. Durations are single-segment `<int><unit>`

**Rationale**: the PRD writes `5s`; compound forms invite parser divergence
between the validator and generated code for no expressive gain. *PRD 5.5.*

### D23. `store` is a node kind alongside PRD 5.5's seven

Node kinds are `agent`, `exec`, `http`, `function`, `flow`, `map`, `human`,
`store`. **Rationale**: PRD 5.8 defines store-op nodes with a flat
`{ store:, op:, key: }` shape but the 5.5 taxonomy table predates it; the node
kind list must be the union. *PRD 5.5, 5.8.*

### D24. `function:` nodes reference tool defs; inline `exec`/`http` nodes stay for one-offs

`function: tool.x` is the graph-invoked use of a tool definition; `exec:`/`http:`
node blocks carry an inline implementation with default output schemas.
**Rationale**: PRD 5.5 lists `exec`/`http` as node types *and* unifies the tool
definition with function-node use. Both must exist; the guidance is to define a
`tool.*` when the implementation is shared or LLM-facing. `exec.args` stays
literal (no CEL) because PRD 5.5 specifies input passing by env vars/stdin.
*PRD 5.5.*

### D25. Tool defs require `description`, `input`, and `output`, and exactly one binding

**Rationale**: the description is the LLM's selection signal (PRD 5.5), the input
is the checked signature for function-node use, and the output is what makes the
edge serializable (PRD 5.2, 5.7). Two bindings would make the call semantics
ambiguous. Result decoding (JSON, or raw text where the whole `output` is one
string-typed property —
[D109](#d109-the-single-string-property-exception-counts-the-whole-property-set))
is specified so `output` is always honored. *PRD 5.5.*

### D26. Flow defs: outputs required, description when tool, no recursion

`outputs:` is required; `description:` is required only when the flow is used in
an agent's `tools:`; a flow reaching itself — §7.7's relation, so through `flow:`
nodes, `map` targets, or tool attachment
([D86](#d86-component-reachability-is-one-relation-and-it-crosses-every-invocation-edge)) —
is an error. **Rationale**: PRD 5.1
makes a flow's I/O surface interchangeable with a tool's, which requires a
declared output; recursion has no termination proof analogous to the SCC rule and
would break the fan-out bounding guarantees. *PRD 5.1, 5.4.*

### D27. `context: inherit` is a `flow:`-node key only, default `isolated`

**Rationale**: PRD 5.7 settles isolation by default with opt-in inheritance at
the instantiation site; putting it on the definition would make a flow's
reusability depend on its own declaration rather than its caller. A `map`
dispatch is the other module boundary and takes no such key:
[D105](#d105-a-map-dispatch-isolates-conversation-history-per-instance) makes its
isolation unconditional. *PRD 5.7.*

### D28. `max_concurrency` is required on the map node; routes may only tighten it

**Rationale**: PRD 5.6 makes execution bounding mandatory and calls unbounded
fan-out a compile error. Requiring it on the node gives every dispatch a bound
even when routes omit it (as in the PRD's own heterogeneous example, where only
one route declares one). *PRD 5.6.*

### D29. Map targets are component references, not flow-local node ids

`node:` accepts `agent.*`, `tool.*`, or `flow.*`. **Rationale**: matches the PRD
example (`node: agent.worker`, `node: tool.review_queue`) and reflects that map
instances are isolated per-item invocations, not nodes of the enclosing graph —
which is also what makes an `agent.*` or `tool.*` target the natural unit for a
placement (§14.1): the dispatch is already the thing a hub hands to a worker,
and several workers claiming one name are the pool it fans out across, with no
change to the logical definition.
[D105](#d105-a-map-dispatch-isolates-conversation-history-per-instance) draws the
same conclusion for conversation history, and
[D73](#d73-on_item_error-carries-its-retry-policy-inline) for policy: what a
dispatched instance is *not* is a node of this flow. *PRD 5.6, 5.10.*

### D30. Union items require `route_by`; non-union items forbid it; `default:` is the catch-all

The `default:` route's target is narrowed to the **unrouted** variants — the
discriminator field plus the fields every unrouted variant declares — and a
`default:` with no unrouted variant is an unreachable-route error.
**Rationale**: PRD 5.6 offers exactly two modes (homogeneous, discriminator-routed)
and requires per-variant exhaustiveness with an explicit default. Allowing a union
to be dispatched to a single target would silently give up narrowing. Narrowing
`default:` to the *whole* union would make it strictly less typed than a named
route, forcing every catch-all sink to accept a lowest-common-denominator item —
the exact shape PRD 5.6 rejects; narrowing to the unrouted variants keeps the
common single-unrouted-variant case fully typed (as in
[`examples/triage-fanout`](../examples/triage-fanout), whose `default:` sees the
`duplicate` variant's `of` field). *PRD 5.6.*

### D31. `detach` rules

Legal in exactly two positions — as a map-block key on the homogeneous form, and
on an individual route — so a map block carrying `route_by:` may not carry a
map-level `detach:` (§8.6 rule 7);
[D85](#d85-a-routed-maps-input-writes-and-detach-are-declared-per-route) extends
the same confinement to the other two per-target keys. A detached dispatch MUST
NOT declare `writes:`
or write reduced state, and `detach: true` under a durably checkpointed target is
a v0 validation error (D59 fixes which targets those are).
**Rationale**: the restrictions are verbatim from PRD 5.6's settled position on
detach under durable execution; idempotency keys are supplied automatically, in
the form §9.4 fixes
([D104](#d104-the-idempotency-key-is-the-flattened-instance-path)).
Confining the key to those two positions removes the only reading question the
shape raises — whether a map-level `detach: true` means "detach every route" or
"detach the routes that do not say otherwise" — and it costs nothing, since the
routed form is precisely the one whose targets are heterogeneous enough that
"all of them" is rarely meant. *PRD 5.6.*

### D32. Reduce policies are typed and `last_wins` is explicit

`append` requires an array channel, `merge` an object channel; an unreduced
channel written from a concurrent context is a compile error. **Rationale**: PRD
5.6 requires a *declared* reduce policy for concurrent writes; making `last_wins`
something an author writes down turns silent overwrite into a reviewed decision.
*PRD 5.6, 5.7.*

### D33. Reserved channel names

`input`, `state`, `execution`, `item`, `messages`, `output`, `payload` may not be
channel names. **Rationale**: five are CEL roots and `messages` is the implicit
history channel, so a channel of any of those names shadows something that
already exists and makes expressions ambiguous. `output` is on the list for a
different reason, stated in §2.5 so a later editor does not read it as a root
and drop it: it is the fixed selector half of `<node>.output`, reserved to keep
that token to one meaning. Refusing the collision rather than resolving it by
precedence is [D74](#d74-reserved-roots-may-not-be-shadowed-by-node-ids-or-item-bindings)'s
argument. *PRD 5.5, 5.7.*

### D34. The store-op catalog is normative, including derived output schemas

Each `kind`/`op` pair fixes its parameters and its output shape (§11.4); the one
field the catalog marks optional — `value` on a `kv`/`blob` `get` — resolves
through
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing).
The shapes are fixed *given the store definition* the op names: `V` and `M` come
from it, and the one field whose very presence the definition decides is a
match's `metadata`, which
[D114](#d114-a-vector-store-with-no-metadata_schema-derives-matches-with-no-metadata-field)
fixes for a store that declares no `metadata_schema`.
**Rationale**: PRD 5.8 requires schema-checked ops and name-based wiring of their
results; without fixed output names, `writes:` and downstream guards would have
nothing stable to bind. *PRD 5.8.*

### D35. `scope:` is required on store definitions

**Rationale**: PRD 5.8 makes lifetime semantically load-bearing (session scope
requires a session-keyed trigger); defaulting it would hide a validation-relevant
choice. *PRD 5.8.*

### D36. `embed.model` is a bare provider-native id, not a `model.*` ref

**Rationale**: the PRD writes `embed: { model: text-embedding-3-small }`;
embedding models are not chat models and do not belong in `model.*`, whose
capability checks are about structured output. The companion `provider:` key —
which this entry originally called optional, leaving what an omission resolved
to unstated —
is [D116](#d116-embedprovider-is-required-and-no-backend-serves-embeddings)'s.
*PRD 5.8, 5.9.*

### D37. `agent_access` narrows the synthesized store tool surface

**PRD-extending** — see this appendix's preamble.

Default `read_write`, narrowable to `read`. **Rationale**: PRD 5.8 synthesizes
`get`/`set` pairs, so `read_write` is the settled default; a declarative way to
withhold writes costs one enum and serves the same least-privilege posture as
placement isolation. **Status**: ratified — PRD §9.13 accepts the key and its
default; 5.8 now records the least-privilege attachment rule. *PRD 5.8, 5.10,
§9.13.*

### D38. Provider kinds are a closed v0 set with per-kind required keys

`anthropic`, `openai`, `openai_compatible`, `azure_openai`, `bedrock`, `vertex`;
[D106](#d106-a-provider-kinds-key-row-is-closed) closes each kind's row over its
*optional* keys as well.
**Rationale**: PRD 5.9 describes provider plugins publishing config schemas; v0
ships the blessed set so the editor schema and the validator agree. New kinds
arrive with new plugins, additively — each publishing a row, since nothing in
§12.1 is a general-purpose provider key waiting to be honored by a plugin that
does not list it. *PRD 5.9.*

### D39. Model defs are direct XOR route

No nested routes; route members are direct models; `route_on` defaults to
`[rate_limit, overloaded, timeout]` and adds `server_error`. **Rationale**: PRD
5.9's two forms, kept disjoint so failover order is a flat, traceable list;
nesting would make "served by fallback #1" ambiguous.

**On `server_error`**: this is a shape decision *within* a settled class, in the
same sense
[D84](#d84-execs-and-https-failure-predicate-is-one-rule-on-both-surfaces-and-both-halves-are-configurable)
is — not a construct the PRD does not imply, which is what this appendix's
preamble reserves for D37 and D51. PRD 5.9 and Resolved Question 10 settle the
*class*: "ordered failover routes on infrastructure conditions only", with
content-based routing explicitly out of scope. The parenthetical "(rate limit,
overload, timeout)" names members of that class in prose, and a provider 5xx is
as plainly an infrastructure condition as an overload is — it is the one
infrastructure failure a caller can observe without the provider naming it.
Enumerating the class for the editor schema and the validator is exactly the job
[D38](#d38-provider-kinds-are-a-closed-v0-set-with-per-kind-required-keys) does
for provider kinds, and the enumeration has to be closed for a `route_on:` value
to be checkable at all. Omitting `server_error` would not have kept the
vocabulary smaller, only made the commonest failover trigger inexpressible in a
list PRD 5.9 requires to be one. *PRD 5.9, §9 Resolved Question 10.*

### D40. `settings:` is the only open object in the logical layer

**Rationale**: PRD 5.9 has provider plugins publish settings schemas the compiler
checks against; the editor schema cannot know them, so it types the common keys
and permits the rest. Everywhere else, unknown keys are errors. *PRD 5.9.*

### D41. Env-ref forms and the secret-field list

Value form (`^\$\{[A-Z_][A-Z0-9_]*\}$`) is mandatory for the credential and
connection fields listed in §4.3; interpolation is allowed in URLs, headers, and
exec env values; `$${` escapes, on every surface. Where refs are illegal
(prompts, descriptions, schemas, CEL, model `id`) an unescaped `${NAME}` token is
a compile error, not surviving literal text. **Rationale**: PRD 5.9 forbids
literals for secrets and keeps refs unresolved in the IR; interpolation is still
needed for host-templated URLs, so the two forms are separated by field rather
than banned outright. Making the token an error where it cannot be substituted
follows the same posture as
[D61](#d61-else-takes-the-literal-true) — the reading the author intended is
never silently discarded — and a uniform escape keeps a prompt that genuinely
discusses `${…}` syntax writable. *PRD 5.8, 5.9, G3.*

### D42. Node outputs are readable only from edge guards and `map.over`

Node configuration CEL sees `input`, `state`, `execution` (and item bindings
inside a map; inside a `tool.*` implementation binding, which is a definition
rather than node configuration, only the tool's own `input` —
[D65](#d65-a-tool-implementation-binding-sees-only-the-tools-own-input)).
**Rationale**: PRD 5.3 scopes guards to the source node's output,
and PRD 5.10 requires every edge to be a potential network boundary — a node
config reaching into an arbitrary other node's output would smuggle in an
undeclared data dependency that placement could not honor. Data that must travel
goes through a channel. *PRD 5.3, 5.7, 5.10.*

### D43. `map.over` is a path expression

Root + field selections + literal indexes; no calls. **Rationale**: the validator
must statically resolve the array schema to prove `max_items` bounding and to
narrow variants; an arbitrary CEL expression makes that undecidable. PRD 5.6
already restricts `route_by` for the same reason. *PRD 5.6.*

### D44. `manual` triggers carry no `input:` bindings

**Rationale**: PRD 5.11 defines manual invocation as `--input k=v` validated
against the flow's input schema; a second binding layer would create two ways to
supply the same values. The compatibility check §13.1 states for the other
trigger types is not dropped, only deferred to run start, where the arguments
exist. `session_key:` remains legal — and defaults to `"payload.session"` — so
CLI runs can join a session and so §11.3's session-coherence check has a static
answer for manual entries (D64). *PRD 5.11.*

### D45. `http` trigger defaults: path, method, `respond: async`, `timeout: 60s`

**Rationale**: PRD 5.11 settles async as the default and says sync "requires a
timeout (default 60s)" — reconciled as: a `respond: sync` trigger always has an
effective timeout, defaulting to 60s, and it drives the documented async
upgrade. The key itself belongs to that mode only
([D81](#d81-timeout-is-illegal-on-an-async-http-trigger)). Deriving the default
path from the trigger name keeps single-trigger projects zero-config. *PRD 5.11.*

### D46. Reserved trigger shapes are fully specified

`schedule` takes 5-field `cron` + `timezone`; `event` takes a logical `source:`
plus `dedupe_key` defaulting to `payload.id`. **Rationale**: PRD 5.11 requires
these to be parsed and validated now and executed in M3; the payload shapes and
dedupe rule are specified so the IR carries everything M3 needs without a grammar
change. *PRD 5.11.*

### D47. Placement entries take `runtime` plus a reserved `network:` — RETIRED

Superseded by [D128](#d128-a-placement-is-a-named-claim-and-its-members-are-components),
[D129](#d129-placement-members-are-agents-and-tools-disjoint-and-colocated-with-what-attaches-them)
and [D130](#d130-the-hub-block-and-the-conditional-join-token). The entry stays
at its number because a decision number is a stable citation; what it decided no
longer exists. It read: a placement is keyed by component address and takes a
required `runtime: isolated|colocated` plus a reserved `network:
none|egress|all`, on the reading that PRD 5.10's isolation covered sandbox,
credentials and network policy. PRD resolved q38 settled how a machine becomes a
placement, and the answer is a **claim a worker asserts**, not a runtime mode a
component declares. §14.1 is the shape that followed; §15 records why re-shaping
reserved grammar is what reserved is for. *PRD 5.10, resolved q38.*

### D48. Backend resolution order and provider vocabularies are fixed in the grammar

Explicit alias → per-kind `defaults:` → target built-in, with `--target local`
overriding unconditionally. **Rationale**: verbatim from PRD 5.8; naming the v0
provider vocabulary per kind lets the editor schema and capability checks agree.
*PRD 5.8.*

### D49. YAML profile: one document, mapping root, anchors yes, merge keys and tags no

**Rationale**: duplicate-key and tag rejection remove silent reinterpretation
(the same posture as version handling), while anchors stay because they are pure
YAML 1.2 and reduce repetition without adding spec semantics. *PRD 5.1, §9.5.*

### D50. Unknown keys are errors everywhere except plugin-config objects

**Rationale**: error UX is the product (PRD G3); a typo in `retrry:` must be a
diagnostic, not a silently ignored key. The exceptions (`settings:`, backend and
event-source configs) are exactly the objects whose schemas live in plugins.
*PRD G3, 5.9.*

### D51. Agents carry `max_tool_iterations`, default 8

**PRD-extending** — see this appendix's preamble.

**Rationale**: PRD 5.4 bounds graph cycles statically; the intra-agent tool loop
is the one remaining unbounded loop in a compiled graph, and a declarative bound
keeps termination reasoning complete. What it bounds is **turns of the loop**,
not calls that reached a tool: since
[D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node)
a call the tool's contract refuses goes back to the model, the correction is
another model call, and so a mis-typed argument spends one of these iterations
exactly as a call that ran does — which is *why* no second counter was needed to
keep a bouncing loop terminating (§5.4). **Status**: ratified — PRD §9.14 accepts
the bound and its default of 8; the 5.5 node taxonomy now names it. *PRD 5.4,
5.5, §9.14, §9.22.*

### D52. `human` node shape

Schemas, `timeout`, and `on_timeout` live inside the `human:` block;
`timeout` and `on_timeout` are jointly optional and jointly required — either
one alone is an error; node-level `timeout`/`retry` are
illegal (and
[D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level) carries
that exemption up the whole §9.3 chain); `on_timeout` accepts a flow-local node
id or `end`, exactly as
`on_error.fallback` does (D21). **Rationale**: PRD 5.5 gives the human node both
schemas plus timeout and route; separating the human wait from an activity
timeout prevents two keys named `timeout` meaning different things on one node.
Two control-transfer positions with different target sets would be a trap with no
rationale behind it. The pairing is symmetric because each half is inert without
the other: a budget with nowhere to go, or a route nothing can reach — the
[D61](#d61-else-takes-the-literal-true) no-op again. *PRD 5.5, 5.11.*

### D53. Flow outputs are name-based from state

Each `outputs:` field reads the channel of the same name; there is no `returns:`
binding, and
[D111](#d111-name-based-wiring-is-type-checked-in-both-directions)
fixes the type relation the two declarations stand in. **Rationale**: PRD 5.7's
default wiring is name-based, and `writes:` already covers renames — a second
output-binding construct would be the deferred data-edge feature under another
name. *PRD 5.7.*

### D54. `description` is required only where it is machine-consumed

Required on `tool.*` and on flows used as tools (LLM-facing); optional but
**always legal** everywhere else — every definition, node, trigger, and
placement entry accepts a `description:` string, and the per-construct key
tables list it as an ordinary optional key rather than an exception to the
unknown-key rule of [D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects).
**Rationale**: PRD 5.5 makes the description part of the tool
contract; forcing it on every definition would be documentation policy, not
grammar. Making it universally *accepted* costs nothing and keeps the one key an
author reaches for reflexively from being an error somewhere. *PRD 5.5.*

### D55. Definition order is irrelevant; IR order is canonical

Files and definitions may appear in any order; the resolver emits definitions
sorted by address and preserves author order only where it is semantic (edge
declaration order, `route:` lists, `imports:`). **Rationale**: PRD 5.12 requires
deterministic, byte-identical output for the same input. *PRD 5.12.*

### D56. Inline `exec`/`http` node results are envelopes, not decoded payloads

On an inline `exec:` node, `exit_code`/`stdout`/`stderr` are synthesized from the
child process; on an inline `http:` node, `status`/`body` are synthesized from the
response. They are never decoded from the payload, their types are fixed, and any
*other* declared field is decoded as JSON from stdout or from the response body
([D91](#d91-the-single-string-property-decode-exception-is-a-tool-surface-rule)
fixes that "decoded" is unconditional here). Envelope names carry no special
meaning in a `tool.*` result schema. **Rationale**: §6.1's decoding rule and the
kind defaults (`{exit_code, stdout}`, `{status, body}`) are otherwise in direct
contradiction — `npm test` would have to print `{"exit_code":0,…}` for the
*default* schema to work. The split follows the surfaces' purposes: a `tool.*`
declares a domain result, while an inline node is a one-off wrapper whose
interesting result is the process/response envelope. Fixing the envelope types
keeps the rule decidable per node and lets codegen emit the binding without
inference. This decision is about *decoding* only: which outcomes are node errors
is one rule on both surfaces, and declaring an envelope field never changes it
([D84](#d84-execs-and-https-failure-predicate-is-one-rule-on-both-surfaces-and-both-halves-are-configurable)).
*PRD 5.5.*

### D57. What counts as a CEL exit condition

An SCC is bounded by (1) an in-SCC edge carrying `max_iterations`, or (2) a node
in the SCC with an edge leaving it *and* a pass on which none of its in-SCC edges
is taken —
[D98](#d98-a-cel-exit-condition-is-about-the-in-scc-edges-going-false) fixes the
syntactic form of clause 2, which this entry originally stated too narrowly.
**Rationale**: PRD 5.4 settles "`max_iterations` **and/or** a CEL exit condition",
so refusing CEL-only cycles would contradict a settled position, while "provably
falsifiable" is not a decidable predicate. Clause 2 is the decidable core of what
an exit condition means under this document's multicast routing (§7.3): if any
in-SCC edge of that node is unconditional, the loop re-enters regardless of the
exit guard, so the guard is not an exit condition at all. The trade-off is stated
in §7.4: only clause 1 is a static termination proof. *PRD 5.4.*

### D58. `append` channels take one element per write

A write to an `append` channel supplies one element typed as the channel's
`items`; unreduced and `last_wins` writes supply the whole value; `merge` writes
supply a partial object. **Rationale**: PRD 5.6's join is "one dispatched
instance contributes its result, index-tagged and reordered by source-item
index", which is element-wise by construction, and PRD 5.7 requires wiring to be
schema-checked — undefined write arity would leave that check with nothing to
compare. Accepting both an element and a whole array would be ambiguous the
moment `items` is itself an array. *PRD 5.6, 5.7.*

### D59. Checkpointing is a target property, and `detach` is checked per target

`--target local` is not durably checkpointed; every other target is; v0 has no
grammar for configuring a checkpointer. `detach: true` is therefore a
target-dependent validation error, resolved like a backend alias (§11.3), with
`local` assumed when no target is named. **Rationale**: PRD 5.6 states the
restriction ("`detach` + checkpointing enabled is a validation error") but PRD
5.7/5.10 put checkpointer configuration in the M3 distributed target, so no v0
spec construct can express it. Inventing a `checkpointer:` key now would ship
grammar for an unbuilt feature; keying off the target makes the rule
implementable today with the machinery target-dependent checks already need.
*PRD 5.6, 5.10.*

### D60. A `flow:` node carries both its own policy and a `policy:` override for its children

`policy:` is level 1 of §9.3 for the nodes *inside* the instance;
`retry`/`timeout`/`on_error` on the same node are level 2 for the instance
*itself*, treated as one activity. **Rationale**: PRD 5.5 places the override at
the instantiation site so a caller can harden a reused module, which says nothing
about bounding the instance as a whole — and a subgraph is a node like any other,
so denying it the common node keys would be a special case with no PRD backing.
Naming the two levels separately is what keeps `policy: {timeout: 30s}` next to
`timeout: 10s` unambiguous. *PRD 5.5, 5.1.*

### D61. `else:` takes the literal `true`

`else: false` is a compile error;
[D107](#d107-an-else-edge-requires-a-when-guarded-sibling) refuses the other
spelling of the same no-op, an `else: true` with no guarded sibling to be else
to. **Rationale**: §7.3 gives meaning only to
`else: true`, and an edge with neither `when:` nor `else:` is already
unconditional — so `else: false` would be a key that changes nothing, which is
exactly the silent no-op the error-UX posture (PRD G3) rejects. *PRD 5.3, G3.*

### D62. An agent's declared `input:` has at least one field

`input: {}` on an agent is a compile error; omitting `input:` is the only way to
get the string-in default. **Rationale**: PRD 5.2 defines exactly two agent input
contracts, string-in and a declared object; an empty object is neither, and would
render an empty JSON object as the user turn. `{}` remains meaningful on
`tool.input`, where a no-argument tool is a real thing. *PRD 5.2.*

### D63. The per-item index is spelled `execution.item_index`

There is no bare `item_index` CEL root. **Rationale**: one spelling means one
type-checker rule and one reserved name (`item`, §2.5); a bare root would also
have to be reserved as a channel name to stay unambiguous, for no gain. What
that one spelling evaluates to where no `map` dispatch encloses it is
[D115](#d115-executionitem_index-is-absent-where-no-map-dispatch-encloses-the-expression)'s.
*PRD 5.6, 5.7.*

### D64. Implicit `manual` invocation is a CLI property, not a declared trigger

Every flow is runnable as `agent-compose run flow.<name>` without a `manual`
trigger being declared. The implicit entry contributes no `triggers:` entry and
no IR record; every check that quantifies over triggers quantifies over declared
ones; `manual` triggers default `session_key:` to `"payload.session"`; and a CLI
run's `--input`/`--session` values are checked at run start against the same
schemas a declared trigger's bindings would be checked against.
**Rationale**: PRD 5.11 carries two settled statements — "the entrypoints of a
project are exactly the flows that triggers point at" and "**`manual`** (v0) …
Implicit for every flow with an input schema". Read as one rule they collide:
every flow with inputs would be an entrypoint, which empties the first statement,
and §11.3's session-coherence check would have to reckon with an implicit,
session-key-less trigger on every flow — rejecting every project that touches a
session-scoped store. Splitting them by surface keeps both intact: declared
triggers define the *deployed* surface (generated routes, schedulers, IR entries,
and everything checked against it), while implicit manual invocation is a
*development* affordance of the generated CLI. Defaulting the one variable input
a CLI run carries — the session key — is what keeps the coherence check decidable
rather than undefined. *PRD 5.11, 5.8.*

### D65. A tool implementation binding sees only the tool's own `input`

Inside `tool.<t>`'s `http:` binding, `query:`/`body:` CEL has exactly one root,
`input`, bound to the tool's declared `input:` object; `state`, `execution`, and
`<node>.output` are out of scope (§4.1, §6.1). **Rationale**: PRD 5.5 makes one
definition serve two surfaces — an agent's tool list, where the *model* supplies
the arguments and no graph context exists at all, and a `function` node, where
the graph supplies them. A binding that could read `state` would be well-defined
on only one of those surfaces, so the shared definition would stop being shared;
scoping the binding to the declared parameters is what keeps the def/use split
honest, and it makes the tool's inputs a complete account of what it can see.
*PRD 5.5, 5.7.*

### D66. An inline node's `input:` never competes with its block for the same slot

On an inline `http:` node, node-level `input:` together with `body:` (body-bearing
method) or with `query:` (`GET`/`HEAD`) is a compile error; wherever an input
object becomes environment variables — an inline `exec:` node's bindings, a
`tool.*` `exec:` binding's declared fields — an `env:` key colliding with the
upper-snake-cased name of one of them is a compile error (§6.1, §8.2, §8.3).
**Rationale**: §6.1 gives the bound input object
a destination — the body, the query string, the environment — that an explicit
in-block key also claims. Silently preferring one leaves the other as a key that
changes nothing, the same silent no-op that
[D61](#d61-else-takes-the-literal-true) rejects for `else: false` and that
[D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)
rejects for unknown keys. Erroring on the overlap costs nothing: the author
either writes the request out in full or lets the bindings build it.
Non-competing combinations (`query:` alongside `input:` on a `POST`) stay legal
because there is no slot to fight over. *PRD 5.5, G3.*

### D67. Store writes inside a `map`: item-derived key **or** a keyed `kv` write

A write from a `map`-dispatched instance is legal when its `key:` is item-derived
*or* when it is a `kv` `set`/`delete` with any key; a `vector`/`blob` write with
a non-item-derived key is a compile error (§11.4).
[D83](#d83-item-derivation-is-traced-through-the-dispatch-binding) fixes what
item-derived means — the surface test this entry originally gave
("references `input.*`") is not it.
**Rationale**: this is PRD 5.8's settled sentence verbatim — "an item-derived key or a keyed `kv` write" — and an earlier draft of
§11.4 tightened it to item-derived keys only, which made the motivating case of
PRD 5.8 (a session-scoped `kv` memory updated from inside a fan-out,
`key: execution.session_key`) a compile error. The exemption is defensible on its
own terms: a `kv` write replaces the whole value at an author-named slot, so
concurrent instances sharing a key are a declared overwrite, the store-side
counterpart of `reduce: last_wins`. Its outcome is *not* deterministic, and that
is sound here where it would not be for a channel: store reads are recorded and
replay consumes history rather than the live store (PRD 5.8), so no store value
feeds the graph's own scheduling, whereas a channel value does (§7.6.4). A
`blob`/`vector` write is bulk content at a document id and has no such story.
*PRD 5.6, 5.8.*

### D68. Flow-node bindings are total; nothing falls through a module boundary

On a `flow:` node, every input field the subflow declares without a `default:`
MUST be bound by `input:`; an unbound one is a compile error. The state-channel
and flow-input fallthrough of §8.0 applies to in-flow targets only.
**Rationale**: PRD 5.7 is settled — "Subgraphs receive parent state only through
explicit bindings … Nothing crosses a module boundary implicitly" — and §8.5 and
§7.5 already said "explicit bindings only", so the single fallthrough sentence in
§8.0 was the outlier. Name-based wiring is what makes one flow's nodes compose
without ceremony; extending it across a module boundary would make a flow's
behavior depend on whether its *caller* happens to have a channel of the right
name, which is exactly the reusability failure PRD 5.7 names, and it would make
the same instantiation legal or illegal depending on an unrelated file. The cost
is one binding line per input, which is the Terraform-module discipline PRD 5.1
asks for. *PRD 5.1, 5.7.*

### D69. Execution is stepwise, and convergence is a per-step join over taken branches

A flow instance runs in steps; a node's outgoing edges are evaluated only after it
completes (P1); a node targeted by two or more edges taken in the same step runs
once (P2); a node reached in two different steps runs twice, and the statically
visible case of that — an unbalanced convergence — is a compile error (§7.6.1,
§7.6.2). **Rationale**: P1 and P2 are LangGraph's superstep semantics, the
compilation target PRD 5.5/5.12 pins, so the join costs no synthetic barrier and
the schedule is a pure function of the graph and the recorded node outputs —
replay-safe by construction. The alternative reading, once-per-arriving-edge, is
not what the substrate does and would fire a node twice for a plain symmetric
diamond. A true AND-join that waits for every branch was rejected for the reason
the finding this decision answers names: with guarded branches it must know which
branches are still live, which is not decidable ahead of time, so a false guard
would deadlock the join. Per-step joining has the opposite property — nothing
waits, so nothing deadlocks — and the only residue, a node re-firing when the
branches have different lengths, is refused statically wherever the distance is
computable (no cycle on the path) and defined explicitly where it is not;
[D112](#d112-balanced-convergence-compares-one-distance-from-each-edge-of-the-pair)
fixes which two lengths "the branches" names, one per edge of a co-takeable
pair.
Exclusivity is proved from `else:` and from enum-equality guards because those
are the two shapes §7.3/§7.3.1 already read; anything else is conservatively
concurrent. *PRD 5.3, 5.5, 5.6, 5.12.*

### D70. `end` retires a branch, and a flow instance ends at quiescence

`end` is a branch sink, not a terminator: the instance finishes when no node is
scheduled, and `outputs:` are materialized once, at that point. Concurrent
branches are never cancelled. **Rationale**: the alternative — the first branch
to reach `end` terminates the instance — makes the materialized output depend on
which branch finished first, which is completion order, which PRD 5.6 names as
the thing that silently breaks replay; under a step model "first" is not even
well-defined inside a step. Cancellation would also leave already-issued effects
(store writes, sink deliveries, HTTP calls) half-applied with no defined state,
against PRD 5.8's at-least-once/idempotency-key discipline. Quiescence is what
LangGraph does natively (a branch routed to `END` simply schedules nothing), so
it costs no machinery, and it gives the two control-transfer positions
(`on_error: {fallback: end}`, `human.on_timeout: end`) an honest meaning — "this
branch is done" — leaving `on_error: fail` as the one way to abort a run.
*PRD 5.5, 5.6, 5.8.*

### D71. No silent dead ends: every node exits, and every run starts

Every node MUST have at least one outgoing edge; at least one edge leaving
`start` MUST be unconditional or `else: true`; a node declaring `on_error: skip`
MUST have an outgoing edge of one of those two forms (§7.6.3) — where the
`else: true` spelling carries its own precondition, a `when:`-guarded sibling
([D107](#d107-an-else-edge-requires-a-when-guarded-sibling)), which the skip
rule's node has by its premise and a single-edge `start` does not.
**Rationale**: [D70](#d70-end-retires-a-branch-and-a-flow-instance-ends-at-quiescence)
makes `end` the only way a branch retires, so these three rules are what keep
that statement true rather than aspirational — without them a branch can vanish
at a node with no exits, an execution can die before its first step with every
`start` guard false, and a skipped node whose edges are all guarded dead-ends on
§7.3 rule 7. Each is the same shape of argument as
[D19](#d19-max_iterations-semantics-and-the-escape-edge-rule)'s escape-edge rule:
a construct that is guaranteed to fail at runtime in a statically visible way is
refused at compile time, and the fix costs one edge or one keyword. Guarded
`start` edges stay legal — routing on `input.*` at entry is useful and its scope
is well-defined (§4.1) — they just cannot be the *only* edges. *PRD 5.3, 5.4, G3.*

### D72. Concurrent writes are applied in a canonical order

Writers within a step are ordered by node id, a `map`'s instances by source-item
index, a `flow:` node counts as one writer, and a node's effective write map MUST
be injective ([D93](#d93-the-effective-write-map-is-what-must-be-injective)); the
reduce policy is applied in that order (§7.6.4). **Rationale**: PRD 5.6 index-tags
and reorders `append` writes "so replay is deterministic" and calls unordered
reduces a silent break of replay — but `merge` and `last_wins` had no order at
all, so two codegens (completion-order versus index-order) produced different
final values from identical runs. Defining one total order for *every* policy
makes PRD 5.6's append rule a consequence rather than a special case. Node id
rather than declaration order is deliberate: it makes the order a property of the
graph, not of file layout, so reordering `nodes:` provably cannot change a run —
[D55](#d55-definition-order-is-irrelevant-ir-order-is-canonical)'s canonical-IR
posture extended to execution. Injectivity of the *effective* map — not of
`writes:` alone ([D93](#d93-the-effective-write-map-is-what-must-be-injective)) —
is what makes "at most one write per
channel per writer" true, which is what makes the order total. Forbidding the
combination outright — no `merge`/`last_wins` from concurrent contexts — was the
other option, and was rejected because it would delete the only expressible
fan-in aggregate that is not a list. *PRD 5.6, 5.7, 5.12.*

### D73. `on_item_error` carries its retry policy inline

`on_item_error: fail | skip | { retry: <§9.1 retry block> }`; a bare
`on_item_error: retry` is an error; exhausted item retries resolve as `fail`; no
implicit chain supplies an item policy (§8.6 rule 10). **Rationale**: PRD 5.6
lists `retry` as a per-item strategy but gives it no parameters, so "retry" alone
named a behavior with no count and no backoff — two implementations would emit
different graphs from one spec, and an author had no way to configure it. Reusing
§9.1's block verbatim inside an enum-or-single-key object is the shape
`on_error:` already uses one section earlier, so this adds no vocabulary and one
reading rule. The chain is stated as *absent* rather than extended because
`defaults:` applies to nodes and a dispatched instance is not a node of the
enclosing flow ([D29](#d29-map-targets-are-component-references-not-flow-local-node-ids));
letting `defaults: { retry: … }` silently become a per-item policy would make the
fan-out's behavior depend on a file the map does not mention. "Retry then skip"
stays expressible one level up, as the map node's own `on_error:`. *PRD 5.5, 5.6.*

### D74. Reserved roots may not be shadowed by node ids or item bindings

The reserved root names of §2.5 may not be used as state channel names, as
flow-local node ids, or as a `map` `as:` binding (except `item`, which is that
binding's own default). **Rationale**: §2.5 previously reserved them for channels
only, which left a node named `input` making `input.output.verdict` ambiguous
between the flow input object and that node's output, and `as: state` making
`state` ambiguous between the state object and the item. A precedence rule —
"the shadow wins", "the root wins" — would have to be memorized and would read
differently in the two cases; refusing the collision is one rule with one error
message, the same posture [D5](#d5-one-identifier-class-lowercase-snake_case)
takes on identifiers generally. That one-rule posture is also why the list keeps
its two non-root members: `messages` is the implicit history channel and
`output` is the selector half of `<node>.output` (§2.5), neither of which a
parser could read two ways, but splitting the seven names into "refused" and
"discouraged" would cost a second rule and a second message to permit
`output.output.verdict`. Definition names are untouched because a
namespaced address is never a bare token in expression position.
*PRD 5.3, 5.5, 5.7, G3.*

### D75. Map dispatch bindings take both `input:` forms

A `map`'s per-item `input:` (and a route's) accepts a field map or a bare scalar
CEL string, with the scalar form binding a string-in agent's single unnamed input
(§8.6 rule 12). **Rationale**: §5.3 requires a string-in agent to be bound with
the scalar form, and the map surface accepted only a field map, so a string-in
agent had no legal binding as a dispatch target at all — an unroutable corner
where one implementer would allow whole-item pass-through and another would
reject the target outright. Admitting the form already defined for node positions
([D14](#d14-string-in-agents-bind-with-a-scalar-input-at-the-node)) closes it
without new vocabulary, and pairing each form with its target's input contract
keeps the mismatch (a field map into a string-in agent, a scalar into a declared
object) an error rather than a coercion. *PRD 5.2, 5.6.*

### D76. `map.over` reads a node that dominates the map node

`over: <node>.output.…` requires `<node>` to dominate the map node; a skipped
dominator or an empty array dispatches zero instances (§8.6 rules 6, 11).
**Rationale**: "any node that precedes the map node" had at least three readings —
path-existence, dominance, declaration order — and under the weakest of them the
producer can sit on a guarded sibling branch that did not run, leaving `over`
with no value and the runtime undefined. Dominance is the reading that makes the
value's existence a *guarantee* rather than a hope, it is computed on the graph
§7.4 already builds, and it stays well-defined with cycles present because a
back-edge adds no new path from `start` — where path-existence degenerates
completely. The zero-instance rule is the companion: a producer that ran but was
skipped, or produced an empty array, must not be a deadlock, so the map completes
immediately and its downstream edge fires. *PRD 5.6.*

### D77. `default:` is legal on every type-node form except a union

Scalar, enum, object, and array type nodes accept `default:` at input surfaces
and on state channels; a discriminated union never does; result surfaces never do
(§3.6). **Rationale**: §3.6 said "scalar and enum", §10.1 said "literal matching
the type", and the published schema accepted `default: []` on an array channel
while refusing one on a union — three positions where there should be one. An
array or object default on an input field is ordinary and useful (`default: []`
for an optional list), and nothing about those forms makes a default harder to
check than a scalar's. A union default is different in kind: it would have to
name a variant, which is manufacturing a discriminator tag, and manufacturing
routing values is precisely what the output-surface prohibition in the same
subsection exists to stop. *PRD 5.2, 5.3, 5.6.*

### D78. Channel initial values, and reading an unset channel

An `append` channel starts as `[]` and a `merge` channel as `{}`; every other
channel with no `default:` is unset, and reading an unset channel fails the
execution naming the channel and the reader (§10.1).
[D101](#d101-a-merge-channels-properties-are-unset-until-supplied) carries the
same rule one level down, to a property a `merge` channel has not been given
yet, and
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)
carries it sideways, to a value a node's own result may legally omit.
**Rationale**: §7.6.2 makes
"what does a convergence see when only one branch ran?" a question the document
has to answer, and the answer is "the channels as they stand" — which requires
knowing what a channel that was never written holds. The two reduce policies with
an identity element get it, so a fan-out that produced zero items reads as
"nothing yet" rather than as a failure; everything else is unset, because
inventing a zero value for a `string` or an `enum` channel would manufacture data
the same way a defaulted output would (§3.6). Failing loudly at the read, naming
both ends, is the error-UX answer (PRD G3) and it tells the author exactly which
`default:` to add. *PRD 5.6, 5.7, G3.*

### D79. The outermost instantiation-site `policy:` wins

When several `flow:`-node `policy:` overrides in a nesting chain set the same
policy field for the same node, the outermost value applies (§8.5, §9.3 level 1).
**Rationale**: [D20](#d20-the-policy-resolution-chain-has-exactly-four-levels)
places the override at the instantiation site *so that a caller can harden a
module it does not own*. Under the innermost-wins reading that module could undo
the hardening by instantiating a deeper one — the level would guarantee nothing,
and a module's effective policy would change when its internals were refactored
into sub-modules. Outermost-wins also keeps resolution independent of nesting
depth, which is what makes a `policy:` a bound the caller can reason about. The
cost is that an inner site cannot tighten below an outer one for the same field;
an author who wants a tighter bound on a whole instance still has the node-level
`timeout:` of §8.5, which bounds the instance as one activity. *PRD 5.5.*

### D80. The published schema's per-file bounds are grammar rules

Where the editor schema constrains a value the grammar left loose and the
constraint is decidable in one file, the grammar states it: import path charset
(§1.4), `expect_status` and `expect_exit` non-empty, in range, and
distinct-membered (§6.1,
[D100](#d100-both-accepted-outcome-lists-are-non-empty-and-distinct)), `route:`
members distinct and `route_on:` non-empty (§12.2), `routes:` non-empty (§8.6),
non-empty `command`/`id`/`embed.model` (§6.1, §12.2, §11.2).
**Rationale**: Appendix B's invariant is one-directional — a file that fails the
schema always fails `validate` — and every one of these was a place the schema
rejected a file the grammar text permitted, which inverts it and makes the two
artifacts disagree about the language. Stating the rule was preferred to dropping
it in each case because each rejects something inert or non-portable: an empty
`expect_status` accepts no response at all, a repeated `route:` member fails over
to the model that just failed, an empty `route_on:` never fails over, an empty
`routes:` dispatches a union to one target with narrowing given up
([D30](#d30-union-items-require-route_by-non-union-items-forbid-it-default-is-the-catch-all)),
and a path with spaces or backslashes is a portability trap in a value that ends
up on command lines and in diagnostics. This is
[D61](#d61-else-takes-the-literal-true)'s posture applied to the schema's own
edges. *PRD G3, 5.1, 5.9.*

### D81. `timeout:` is illegal on an `async` http trigger

A trigger with `respond: async` — declared or defaulted — MUST NOT carry
`timeout:` (§13.3). **Rationale**: the timeout bounds the *response*, and an
async trigger has already responded with an execution id, so the key changes
nothing that can be observed; PRD 5.11 attaches the budget to sync's async
upgrade specifically. `callback:` with `respond: sync` was already an error for
the mirror-image reason, and leaving the other half accepted would say that one
inert key is a mistake and its twin is fine. Same posture as
[D61](#d61-else-takes-the-literal-true) and
[D52](#d52-human-node-shape)'s timeout/route pairing: a key whose author expected
it to do something gets a diagnostic, not silence. *PRD 5.11, G3.*

### D82. Exhaustiveness and exclusivity read one closed set of guard shapes

§7.3.1 fixes a table of guard shapes and two sets per enum field —
`guaranteed(g, f)` (true whatever else is true) and `possible(g, f)` (not
provably false). The exhaustiveness check fires for every enum field a node's
guards *mention*, and is satisfied by an unconditional/`else:` edge or by one
field whose variants the `guaranteed` sets cover; §7.6.1's exclusivity test is
the same table read for disjoint `possible` sets.
**Rationale**: the previous wording ("some guarded edge *can* be true for it")
left the flagship PRD 5.3 check with three undefined halves — whether it fires on
a mixed-guard node, which syntax counts, and what "can be true" means — and the
two readings disagree about real specs: under the literal one,
`when: "size(state.feedback) > 0"` routes *every* verdict variant and the check
guarantees nothing, so a spec it accepts dead-ends on §7.3 rule 7 at runtime.
Coverage must therefore be a *guarantee*, which is what `guaranteed` is, with
unrecognized terms contributing `∅`. Exclusivity needs the opposite bound —
excluding a variant, not guaranteeing one — so the same shapes carry a second
column rather than a second vocabulary, and §7.6.1 stops pointing at a shape no
section defined. The tables are closed and syntactic so that "does this
composition compile?" has one answer; a guard outside them is not rejected, it
simply proves nothing, which keeps the check conservative in the safe direction
(it asks for an `else:`, it never invents a route). Firing on *mention* rather
than on *coverage* is deliberate: a node whose only enum guard is
`f == 'a' && …` is routing on `f` in every sense the author meant, and the fix
is one keyword. *PRD 5.3, G3, G4.*

### D83. Item-derivation is traced through the dispatch binding

An expression is item-derived, per dispatch site, when it references
`execution.item_index` or an `input.<field>` whose binding at that site is itself
item-derived — with whole-item dispatch making every field item-derived, and
`state.*`/enclosing-`input.*`/literal bindings making none (§11.4).
**Rationale**: the earlier operational test — "the `key:` expression references
`input.*`" — was unsound, and unsound in exactly the direction the rule exists to
prevent. A map may bind a dispatched flow's input field from the enclosing
flow's `state` or from a literal (§8.6 rule 12), and then `input.doc_id` is the
*same* value in every instance: the check would pass while N documents land on
one vector key, which is PRD 5.8's "unkeyed blob/global writes from concurrent
instances" wearing a key-shaped mask. Tracing through the binding is the reading
that makes the check mean what its own justification claims, it is decidable
(bindings are CEL over a known scope, and the dispatch graph is finite and
acyclic — recursion is already an error, §7.5), and it composes through nested
`flow:` nodes and nested maps without a second rule. Per-site evaluation is
required because derivation is not a property of the store node alone: the same
flow can be dispatched with an item-derived binding from one map and a constant
one from another, and both facts are true. The `kv` exemption
([D67](#d67-store-writes-inside-a-map-item-derived-key-or-a-keyed-kv-write)) is
untouched — it is PRD 5.8's own second form. *PRD 5.6, 5.8.*

### D84. `exec`'s and `http`'s failure predicate is one rule on both surfaces, and both halves are configurable

An exit status outside `expect_exit` (default `[0]`) and a response status
outside `expect_status` (default: any 2xx) are node errors under §9 — on a
`tool.*` binding and on an inline node alike. Declaring an envelope field is a
*decoding* choice and never changes the predicate; widening the accepted set is
what turns a failure into routable data (§6.1, §8.2, §8.3).
**Rationale**: §6.1 stated the predicate on the tool surface only, and
[D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads)
overrode inline result *binding* without saying whether the failure rule came
with it — while the inline kind defaults (`{exit_code, stdout}`,
`{status, body}`) invited the opposite reading, that an inline node observes its
failure as data. The two readings give the commonest exec use case opposite
runtime behavior from one spec (skip-and-write-nothing versus
complete-with-`exit_code: 1`), so the document had to pick. Uniformity wins:
one predicate for one construct keeps §9's chain meaningful on every surface,
and it keeps `expect_status` — which the schema and §6.1 already carried — from
becoming inert on inline nodes, which is the silent no-op
[D61](#d61-else-takes-the-literal-true) refuses. That left the real gap, which
is that `exec` had no way to *say* an exit code is expected while `http` did;
`expect_exit` closes it with the shape already in the document rather than a new
concept, and it is what makes the `exit_code` envelope field worth declaring.
This is a shape decision in the class of `expect_status`, not new design
surface: PRD 5.5 routes node errors through retry/`on_error` without fixing
which outcomes *are* errors, and one of the two halves had to be nameable for
the other to mean anything. *PRD 5.5, G3.*

### D85. A routed map's `input:`, `writes:`, and `detach:` are declared per route

All three are map-block keys on the homogeneous form only; a map declaring
`route_by:` declares them on its routes (§8.6 rule 7). `max_concurrency:` and
`on_item_error:` stay map-wide.
**Rationale**: [D31](#d31-detach-rules) confined `detach:` for a reason that
applies verbatim to the other two — the routes of a heterogeneous map are
independently typed dispatch targets — but the §8.6 key table left `input:` and
`writes:` unrestricted, so a routed map could carry a map-level `writes:` naming
output fields only some route targets declare, with three defensible readings
(check against every route, apply only where a route is silent, reject) and no
text choosing one. `input:` is worse: PRD 5.6's narrowing guarantee types each
route's per-item CEL against *its own variant's* payload, so a map-level `input:`
would have to type-check against every variant at once, which is precisely the
lowest-common-denominator item type PRD 5.6 rejects. The two keys that stay
map-wide are the two that are not typed against a target: `max_concurrency:` is
a bound on the node (D28) and `on_item_error:` is a strategy (D73). The cost is
repeating a shared remap across routes, which the narrowing rule usually makes
impossible to share anyway. *PRD 5.6.*

### D86. Component reachability is one relation, and it crosses every invocation edge

§7.7 defines reaching once — own nodes, `map` dispatch targets, an agent's
`stores:`, an agent's `tools:` (including `flow.*` entries), transitively — and
session coherence (§11.3), sync interrupt-freedom (§13.3, §8.7), placement
colocation (§14.1 rule 4) and recursion (§7.5) all use it. A check may read one
part of what the relation returns — colocation reads clauses 1 and 2 — but never
a traversal of its own.
**Rationale**: the three checks each spelled out their own traversal, and the
sets differed: §11.3 enumerated nodes, `flow:` nodes, and `stores:`; §13.3 said
only "reachable from its entry". The gap is not academic — a `respond: sync`
trigger whose flow contains an agent whose `tools:` lists a `flow.*` containing a
`human` node is accepted by the narrow reading and interrupts inside a
synchronous HTTP request, which is the exact outcome PRD 5.11's settled
"statically interrupt-free" position exists to exclude. Flow-as-tool attachment
is an invocation (PRD 5.1 makes the tool and flow surfaces interchangeable), and
so is a `map` dispatch target, so both belong in the relation for every check
that asks "can this run?". Stating the relation once also removes the class of
bug where one check is later extended and its siblings silently are not. The
direction of the change is conservative: it rejects more compositions, and each
newly rejected one is a spec that would have violated a guarantee at runtime.
*PRD 5.1, 5.8, 5.11.*

### D87. `local` is a reserved target, and `deploy/local.yml` carries no `storage_backends`

`local` is built in: no deploy file is required, `hub:`, `placements:` and
`event_sources:` are read from `deploy/local.yml` when it exists,
`storage_backends:` there is a compile error, neither a store's `backend:` alias
nor an `event` trigger's `source:` needs a definition under it, and
`--target <other>` requires its deploy file to exist (§14, §11.3, §13.5).
**Rationale**: §1.6 listed `deploy/local.yml` as an ordinary target file while
§11.3/§14 said `local` substitutes local storage *unconditionally*, and the two
statements together left `storage_backends:` in that file with three readings —
honored (contradicting PRD 5.8's zero-infra guarantee, which is settled),
silently ignored (the inert key [D61](#d61-else-takes-the-literal-true) and
[D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)
refuse), or rejected. The PRD's sentence is senior, so honoring is out and the
document's own posture picks rejection over silence. The rest of the file stays
legal because `hub:`, `placements:` and `event_sources:` are not storage and are
carried into the IR under `local` exactly as under any other target, so nothing
there is inert. The asymmetry is the split §15 draws three ways:
`storage_backends:` configures something `local` overrides *today*; `hub:` and
`placements:` are live grammar whose every static rule `local` is held to, and a
`local` mesh — this process as the hub, a worker beside it — is a real
deployment rather than a contradiction; `event_sources:` is reserved grammar no
v0 target executes, so it too is carried into the IR under every target rather
than being dead under one. Satisfying both binding checks vacuously under `local` is what keeps
the zero-infra guarantee real — a project whose staging target names chroma and
Redis Streams still validates and runs with nothing installed, which is the
whole point of the guarantee. Requiring a named target's file to exist is the
same posture one level up: `--target stagng` is a typo, and resolving it to
built-in backends would deploy against the wrong infrastructure without a
diagnostic. *PRD 5.8, 5.10, 5.11, G3.*

### D88. The scalar `input:` form is legal only where an unnamed value has a destination

Scalar `input:` binds a string-in agent (node position and `map` dispatch) and an
inline `exec:` node's stdin; on `http:`, `function:`, `flow:`, and `human:` nodes
it is a compile error, and `store:` nodes take no `input:` at all (§8.0).
**Rationale**: [D14](#d14-string-in-agents-bind-with-a-scalar-input-at-the-node)
and [D75](#d75-map-dispatch-bindings-take-both-input-forms) introduced the form
for the one contract that has no field names, and §8.2 gave it a second honest
destination (stdin, from PRD 5.5's own input convention). The published schema
then offered it wherever it offered bindings, including inline `http:` nodes,
where §8.3 defines request construction for the field-map case only — so
`input: "state.draft"` on an `http:` node passed the editor schema with three
possible meanings (reject, send the bare string as the body, wrap it in an
object) and no text. Every other kind names its fields: a tool's `input` is a
field map (possibly `{}`), a subflow's `inputs` is a field map, a `human:`
node's is too. Listing the two legal positions instead of the six illegal ones
keeps the rule one line and makes the schema check it per file. *PRD 5.2, 5.5.*

### D89. `optional:` entries must name declared properties

An `optional:` entry naming something the object does not declare is a compile
error listing the unknown and the declared names (§3.4).
**Rationale**: §3.4 said only "property names that are not required", which left
`optional: [emial]` with no stated behavior at all — and the silent reading is
the harmful one, because the typo's visible effect is that `email` stays
required, which is the opposite of what the author wrote. This is
[D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)'s
posture on the one surface where the "key" is an array element rather than a
mapping key: a name that names nothing is a typo, and error UX is the product
(PRD G3). The schema cannot see it — JSON Schema constrains an array's items
without reference to a sibling object's keys — so it is a validator rule, listed
as such in Appendix B. *PRD 5.2, G3.*

### D90. `max_iterations` is legal only on a guarded edge

An edge carrying `max_iterations` MUST also carry `when:`; on an unconditional
edge or an `else: true` edge it is a compile error (§7.2, §7.3.1).
**Rationale**: four rules in this document rest on an edge being *guaranteed to
fire* — exhaustiveness clause 1 (§7.3.1), the `start` edge and the `on_error:
skip` escape ([D71](#d71-no-silent-dead-ends-every-node-exits-and-every-run-starts)),
and the cycle escape ([D19](#d19-max_iterations-semantics-and-the-escape-edge-rule))
— and all four name the same two edge forms. But §7.3 rule 5 makes an exhausted
edge untakeable *regardless of its guard*, so a budget on one of those forms
would quietly withdraw the guarantee on the pass after it ran out, which is the
runtime dead end the four rules exist to remove, reintroduced by the key meant to
bound a loop. Restricting the budget to guarded edges makes "guaranteed" mean
guaranteed everywhere the document says it, and it costs nothing: PRD 5.4's own
example and every cycle in `examples/` put `max_iterations` on the guarded
back-edge, which is where a bound belongs — the unconditional or `else:` sibling
is the way *out*. `when: "true"` remains writable and remains a guarded edge, so
it can carry a budget and still cannot serve as any rule's guarantee.

**One counter per bounded edge.** The budget is per *edge*, so an SCC with two
`max_iterations` edges carries two counters and two independent budgets (§7.4).
PRD 5.4's codegen sentence — "an iteration counter into graph state per bounded
cycle" — is the same statement for the shape it describes, a cycle bounded by
one edge, which is what its own example writes and what every cycle in
`examples/` writes; a per-*cycle* counter cannot be the general form, because it
would have to say which of two budgets it counts against and `max_iterations`
counts traversals of the edge it sits on. This document fixes the general form,
exactly as
[D104](#d104-the-idempotency-key-is-the-flattened-instance-path) does for what
"node" denotes in PRD 5.6/5.8's idempotency key: prd.md's shorter phrasing
stands as written and the precise form lives here. *PRD 5.3, 5.4, G3.*

### D91. The single-string-property decode exception is a `tool.*`-surface rule

§6.1's "except when `output` declares exactly one property and that property is
string-typed, in which case the raw text binds to it" —
[D109](#d109-the-single-string-property-exception-counts-the-whole-property-set)
fixes what that counts — applies to `tool.*` `exec:`/`http:` bindings only.
On an inline `exec:`/`http:` node, every non-envelope declared field is decoded
as JSON unconditionally (§8.2, §8.3).
**Rationale**: §8.2 said the exception was "computed over the decoded fields
alone" while §8.3's worked example declared
`{ status: {type: integer}, id: {type: string} }` and got `id` "from the decoded
body" — and under §8.2's own sentence that node's decoded set is exactly one
string-typed property, so the exception would bind the whole raw response text to
`id` instead. Two conforming codegens produced different runtime values for the
document's own example, so one of the two sentences had to go. The exception is
the one that goes, because on an inline node it has no job: raw text already has
a name there — `stdout`, `body` — which is exactly what
[D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads)
established the envelope for, and `writes: { stdout: <channel> }` renames it. A
`tool.*` has no envelope and no other way to name raw text, which is why the
exception exists at all and why it stays there. Keeping it on both surfaces was
the alternative and is worse in both directions: it would make
`{ report_normalized: {type: string} }` on an inline node mean "the whole
response body", which no author writing a field name means, and it would make a
node's decoding depend on how many *other* fields it declares — a lone
`{ id: {type: string} }` would take the whole raw body, and adding a second
field beside it would silently start parsing. One rule per surface, each
justified by what that surface can name. *PRD 5.5.*

### D92. The env-ref classification is total over string surfaces

§4.3 sorts every string-valued surface into env-ref-value-only, interpolable, or
no-refs, with no-refs as the default for anything the section does not list. The
`exec:` block is interpolable end to end (`command`, `args` entries, `cwd`,
`env` values); `embed.model`, model `settings:` values, a trigger's
`path:`/`cron:`/`timezone:`, and a `blob put`'s `content_type:` take no refs.
**Rationale**: the previous wording gave a legal list and an illegal list and
left the gap between them undefined — `exec` `command:`/`args:`, `embed.model`,
trigger `path:`, and `content_type:` were in neither, so `args: ["--host=${DB_HOST}"]`
had three defensible behaviors (interpolate, reject, pass the six characters
through) and the compile-error rule, which is scoped to the illegal list, decided
none of them. A partial classification cannot be implemented twice the same way,
so the fix is to make it total by construction rather than to extend a list and
leave the next surface unclassified. The split itself follows what each surface
is: an `exec:` block is a process invocation, the archetypal env-parameterized
value, and nothing in it is shell-interpreted, so a substituted value is one
argv element or one variable — never a re-parsed command line. The no-refs side
collects the strings the validator shape-checks (a cron expression, an IANA
zone, a route path) and the strings that are part of what the composition *is*
(a model id, an embedding model id, a content type, `settings:` values): a value
that first exists at process start is a value neither `validate` nor a diff can
see, and PRD 5.9 wants every LLM configuration greppable in one file. This is
orthogonal to
[D24](#d24-function-nodes-reference-tool-defs-inline-exechttp-nodes-stay-for-one-offs)'s
no-CEL rule for `args:` — CEL reads graph data at run time, an env ref reads the
environment at process start — and it keeps [D41](#d41-env-ref-forms-and-the-secret-field-list)'s
posture intact: where a ref cannot be substituted it is an error, never text that
silently survives. *PRD 5.8, 5.9, G3.*

### D93. The *effective* write map is what must be injective

A node's effective write map pairs each output field with the channel it writes —
the `writes:` value if remapped, otherwise the same-named channel if one is
declared — and MUST be injective (§8.0, §7.6.4 rule 4).
**Rationale**: [D72](#d72-concurrent-writes-are-applied-in-a-canonical-order)
rests on "within one writer there is at most one write per channel", and
injectivity of `writes:` alone does not deliver it. A node with output `{a, b}`,
`writes: { a: b }`, and a declared channel `b` satisfies every stated rule — the
keys are output fields, the value is a declared channel, a one-entry map is
trivially injective, and "a remapped field is not also written to its same-named
channel" only stops `a` → channel `a` — while `b` still writes channel `b` by
name. That is two writes to one channel from one writer, with no order between
them: on an `append` channel the element order is unspecified, and on an
unreduced channel the single-writer premise of §10.2 is violated without any
check firing. Since the canonical order (§7.6.4) is defined *per writer*, the
hole is directly a replay-determinism hole in the construct D72 exists to close.
Stating the rule over the effective map closes it with the same one-line
diagnostic and no new vocabulary; the alternative — inventing an intra-writer
tie-break, say output-field declaration order — would define an order for a
construct that has no reason to exist, since the author who wants both fields on
one channel can say so with an explicit `reduce:` policy and two nodes, or fix
the collision with a second remap. *PRD 5.6, 5.7, 5.12.*

### D94. A detached dispatch is resolved at dispatch

`detach: true` takes the instance out of the map's join and out of
`on_item_error`: the join counts it the moment the dispatch is issued, its
outcome is never observed, and a failed delivery is neither an item error nor a
map-node error (§8.6 rules 6, 7, 10).
**Rationale**: PRD 5.6 settles both halves — "Default join semantics wait on
*all* routes, including sinks (a failed enqueue is a surfaced failure)" and
"Per-route `detach: true` opts into fire-and-forget" — but §8.6 rule 6
quantified the join over "every instance" with no carve-out while rule 7 said
only that a detached dispatch may not write reduced state. Under `--target
local`, where `detach: true` is legal and M1 must implement it, one implementer
waits at the barrier (making the key observably meaningless, which contradicts
the second settled half) and another does not; and whether a locally-failed
detached POST is an item error subject to `on_item_error: { retry: … }` had no
answer at all. "Fire-and-forget" has exactly one coherent reading — the dispatch
is the last thing the graph knows about that item — so both consequences follow
from it rather than being separate choices. The trade is bounded by the same
mechanism PRD 5.6 pairs with detach: delivery carries an execution-derived
`idempotency_key` and is at-least-once, so an unobserved outcome is a delivery
the sink can dedupe rather than a message the graph silently dropped. The map
node's own `on_error:` still covers failures that are the *node's* — a dispatch
that could not be issued — which keeps `on_error:` meaningful without
reintroducing the wait. The key itself is
[D104](#d104-the-idempotency-key-is-the-flattened-instance-path)'s: without a
form that separates one dispatch site from another, the dedupe this trade rests
on would drop real deliveries. *PRD 5.6.*

### D95. Node reachability counts edges and the two control-transfer positions

Every node MUST be reachable from its flow's `start` over edges,
`on_error: { fallback: … }`, and `human.on_timeout:`; an unreachable node is a
compile error (§7.8).
**Rationale**: PRD §7 M0 lists "unreachable nodes" among the static checks this
grammar feeds, and it was the one check in that list with no defining section —
only an aside in §13 saying the analysis is per flow. Two implementations
followed: over `edges:` alone, which rejects a node targeted solely by
`on_error: { fallback: cleanup }` or by `human.on_timeout:`, and over edges plus
those two positions, which accepts it. The second is right, and not marginally:
§2.4 already calls those the **control-transfer positions**, §9.2 and §8.7 give
them scheduling semantics identical to an edge's, and a dedicated error-handling
or timeout node with no inbound edge is a natural shape — one keystroke away from
`examples/triage-fanout`'s `escalate`, which passes an edge-only check only
because it happens to also carry an inbound edge. Guards are ignored because the
question is addressability, not whether a path fires; §7.3.1 is where coverage is
decided, and folding the two questions together would make an unroutable-in-
practice node either an error twice or an error nowhere. §7.7's component
relation deliberately does not govern here: it is about which *components* a flow
can cause to run, across flows, and it says fallback and `on_timeout` targets are
"already covered by clause 1" for that purpose only. Nor does this relation feed
§7.6.2's `dist`, which counts edges because it measures steps, while this one
counts control transfers because it asks about addressability — a control
transfer fires *instead of* a node's edges, so admitting it here cannot
manufacture a concurrent branch there (§7.6.2). Three relations, three
questions, one section each. *PRD 5.3, 5.5, G3.*

### D96. An inline `exec`/`http` node's `output` is a result schema

The result surfaces are `agent.output`, `tool.output`, `flow.outputs`,
`human.output`, an inline `exec:`/`http:` node's `output`, and the two store
schemas. Arrays inside any of them MUST declare `max_items` and `default:` is
illegal in all of them (§3.5, §3.6, §3.9).
**Rationale**: the published schema already routed `execNode`/`httpNode`
`output` through the result-surface field map — `max_items` required, `default:`
refused — while §3.5, §3.6, D10, and Appendix B each enumerated the same shorter
list and omitted the inline pair. That inverts Appendix B's one-directional invariant: an
inline `exec:` node declaring `names: { type: array, items: {type: string} }`
passed the grammar text and failed the schema, so the two artifacts described
two different languages, which is the one thing this document and that file may
not do. Adding the surfaces was preferred to loosening the schema because the
justification for the rule reaches them: D10's second reason is that a bounded
array keeps every **edge payload** finite and serializable, which is what lets
placement promote an edge to a network boundary (PRD 5.7, 5.10), and an inline
node's declared fields travel edges exactly as an agent's do. The first reason —
model-produced cardinality (PRD 5.6) — does not apply here, and does not need
to: these fields are decoded from a process's stdout or a response body
([D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads),
[D91](#d91-the-single-string-property-decode-exception-is-a-tool-surface-rule)),
which are as unbounded as any model. The `default:` half follows from the same
reading of §3.6: a defaulted result manufactures a routing value the run never
produced, and nothing about a process makes that safer than a model. Stating the
list once, in §3.5, and pointing §3.6, §3.9, D10, and Appendix B at it is what
stops the next surface from being added to three of the four places. *PRD 5.2,
5.5, 5.6, 5.7, 5.10.*

### D97. A `skip` changes guard values, not the routing algorithm

After `on_error: skip`, routing runs through §7.3 unchanged; the single
substitution is that a `when:` guard referencing the skipped node's output
evaluates `false`, while a guard over `input`/`state`/`execution` evaluates
normally and an `else:` edge keeps its §7.3 rule 4 meaning — taken iff no
guarded sibling was taken (§9.2, §7.6.3 rule 3).
**Rationale**: three statements in this document disagreed the moment a skipped
node carried a state-only guard, which §4.1 explicitly permits and §7.3.1's own
worked example uses. §9.2's cell said unconditional *and* `else` edges "still
fire"; §7.3 rule 4 says an `else:` edge fires only when no guarded sibling was
taken; and §7.6.3 rule 3 paraphrased §9.2 as "its guarded edges evaluate false",
which is false of a guard the missing output has nothing to do with. Take
`{when: "size(state.xs) > 0"}` and `{else: true}` leaving a skipped node with a
non-empty `state.xs`: the first sentence fires both edges, the second fires only
the first, the third fires only the `else:`. Three conforming codegens, three
different control flows, one spec.

The fix is to stop restating routing in the error section. A skip is a fact
about one node's *output*, so it can only change what an expression reading that
output evaluates to; everything else — multicast, declaration order, `else:`,
exhausted budgets — is §7.3's, stated once. Choosing `false` for the
output-referencing guard rather than a failed read (§10.1's rule for an unset
channel) is what keeps `skip` distinct from `fail`: a strategy whose whole
purpose is to continue past a failure cannot fail the execution at the next
guard. And the choice leaves §7.6.3 rule 3 doing exactly the job
[D71](#d71-no-silent-dead-ends-every-node-exits-and-every-run-starts) gave it —
guaranteeing one taken edge — with the `else:` half now honest about *how*:
either a state-only guard routed the branch, or the `else:` did. *PRD 5.3, 5.5,
G3.*

### D98. A CEL exit condition is about the in-SCC edges going false

§7.4 clause 2 asks two questions of a node `n` in the SCC — does an edge leave
(2(i)), and is there a pass on which no in-SCC edge of `n` is taken (2(ii)) — and
puts no requirement on the *exit* edge's own shape beyond the one case where an
in-SCC `else: true` needs a guarded sibling to suppress it.
**Rationale**: the earlier clause 2 required the SCC-leaving edge to "carry a
`when:` guard", which rejected the exact loop this document endorses everywhere
else. Drop `max_iterations` from `examples/review-loop`'s cycle and it is
`{when: "verdict == 'revise'"}` back plus `{else: true}` out — the shape
§7.4's escape rule requires, §7.3.1 clause 1 accepts, and
[D19](#d19-max_iterations-semantics-and-the-escape-edge-rule) calls "the usual
spelling" — yet clause 2 called it an unbounded SCC while accepting the
semantically identical `{when: "verdict != 'revise'"}` rewrite. Two spellings of
one loop, opposite verdicts, with PRD 5.4's own words ("a CEL exit condition on
at least one edge in the cycle") satisfied by both: an `else:` edge fires exactly
when no guarded sibling fired, which is a condition, expressed in CEL, on an edge
in the cycle.

Fixing it by adding `else:` to the list of accepted exit shapes would have
patched the symptom. The real error was locating the exit condition on the wrong
edge: under multicast routing (§7.3 rule 6) a loop continues iff some in-SCC edge
fires, so termination is a property of the in-SCC edges going false, and the
leaving edge's only job is to give that pass somewhere to go. Restating clause 2
that way accepts both spellings, still rejects an unconditional in-SCC edge (it
fires every pass), and newly rejects a shape the old wording quietly allowed
through its "(or an `else:`)" parenthesis — an in-SCC `else: true` whose only
exit is unconditional, where the `else:` has no guarded sibling to suppress it
and so fires forever. The rule is decided from edge shapes alone, so it stays
one linear scan of `n`'s out-edges, exactly as before. *PRD 5.4.*

### D99. Co-takeability is a relation on a *pair* of sibling edges

An edge is never co-takeable on its own; it is co-takeable *with a named
sibling*. A fork is a node with two out-edges co-takeable with each other, and
each such pair is one **co-takeable pair**. §7.6.1's concurrency relation and
§7.6.2's `dist` are both stated per pair: two nodes are concurrent when one
co-takeable pair reaches one each, and balanced convergence compares the two
distances of one pair (§7.6.1, §7.6.2) — one distance from each edge of it,
which is
[D112](#d112-balanced-convergence-compares-one-distance-from-each-edge-of-the-pair)'s
half of the same reading.
**Rationale**: §7.6.1 defined co-takeability pairwise and then §7.6.2 used it as
a unary predicate — "paths that leave `f` by a co-takeable edge" — leaving an
out-edge that is exclusive with *every* sibling undecided: in or out of `dist`?
The two readings disagree on real specs. Take `e₁ when: "n.output.f == 'x'"`,
`e₂ when: "size(state.q) > 0"` (co-takeable, so `f` is a fork), and
`e₃ else: true` (exclusive with both by §7.6.1 rule 1), with `e₃` reaching a
convergence `d` in one step and `e₁`'s path in two. Reading it as "belongs to
some co-takeable pair" excludes `e₃`, leaves the pair `(e₁, e₂)` to be compared
on its own, and compiles; reading it as "any out-edge of a fork" pools `e₃`'s
`{1}` with `e₁`'s `{2}` and reports an unbalanced convergence. Two validators,
two languages.

The pairwise reading is the correct one, and not merely by fiat: `e₃` is an
`else:` edge, so it fires **only** on the passes where neither guarded sibling
did (§7.3 rule 4) — it cannot deliver to `d` alongside `e₁`, so there is no
second arrival, so there is nothing for the check to refuse. Rejecting it would
refuse a runtime-safe composition, which is the one direction a conservative
static check must not go: elsewhere this document errs toward asking for an
`else:`, never toward forbidding one. Stating `dist` per pair rather than per
fork is what makes that fall out of the definition instead of needing a
carve-out, and it costs nothing — a fork has finitely many pairs, and the
per-pair distances are the same walk the per-fork version already did. *PRD 5.3,
5.6, G3.*

### D100. Both accepted-outcome lists are non-empty and distinct

`expect_exit` and `expect_status` are each a **non-empty** list of **distinct**
members, on the `tool.*` binding and on the inline node alike (§6.1, §8.2,
§8.3), and the published schema enforces both halves of both.
**Rationale**: the two keys are one construct on two surfaces —
[D84](#d84-execs-and-https-failure-predicate-is-one-rule-on-both-surfaces-and-both-halves-are-configurable)
says so outright — and they disagreed about duplicates. The schema carried
`uniqueItems` on `expect_exit` and not on `expect_status`, while §6.1 and
[D80](#d80-the-published-schemas-per-file-bounds-are-grammar-rules)'s own
enumeration stated distinctness for neither. `expect_exit: [0, 0]` was therefore
rejected by the schema and permitted by the grammar text — the inverted
one-directional invariant D80 exists to close — and `expect_status: [200, 200]`
was accepted by both, so twin keys behaved differently for no stated reason.

Two ways to reconcile them; the tightening is the right one, for D80's stated
posture. Membership in an accepted-outcome list is a set test, so a repeated
member is inert: it changes no run's behavior, and an author who wrote it meant
something the key cannot do. That is exactly the silent no-op
[D61](#d61-else-takes-the-literal-true) refuses, and exactly the argument D80
already accepted for a repeated `route:` member, which is the same shape of
mistake in the same document. Dropping `uniqueItems` instead would have made
this document's only stated reason for that `route:` rule not apply to its
nearest neighbour. *PRD 5.5, 5.9, G3.*

### D101. A `merge` channel's properties are unset until supplied

Reading a property a `merge` channel does not currently hold fails the execution
naming the channel, the property, and the reader; type-checking is unaffected;
a channel `default:` (which must supply every required property) or a CEL
`has()` test is how a read becomes unconditional (§10.1).
**Rationale**: [D78](#d78-channel-initial-values-and-reading-an-unset-channel)
answered "what does a channel that was never written hold?" per channel, and a
`merge` channel is the one form where that is not enough. Its declared type is a
closed object with required properties, but §10.2 and
[D58](#d58-append-channels-take-one-element-per-write) let a write supply a
*subset* of them, so the channel legitimately holds `{}` and then partial
objects — values that do not validate against its own type. A guard
`when: "state.totals.skipped > 0"` then type-checks (the property is declared
`integer`) while the key may simply be absent at runtime, and the document said
nothing: one implementer would fail the run as for an unset channel, another
would surface a CEL absent-field error, a third would default to `0`. Three
runtime behaviours from one spec, in the construct §7.6.4 relies on for
deterministic fan-in.

Field-level unset semantics is the answer that costs no new concept: it is
D78's rule with the same justification (a manufactured zero for a declared
`integer` is invented data, and PRD G3 wants the failure to name what to fix)
and the same escape hatch (`default:`), applied one level down. The alternative
— requiring every property of a `merge` channel to be present or defaulted
before any read — would have to be enforced statically, and it cannot be: which
writes have landed is exactly the runtime question. Admitting `has()` as the
guarded-read spelling keeps the strict rule usable, and it is already in §4.1's
supported CEL surface, so this adds no vocabulary either. *PRD 5.6, 5.7, G3.*

### D102. A `human` node resolves no `timeout` and no `retry`, at any level

The two fields are withheld from a `human` node at every level of §9.3's chain —
a flow node's `policy:` (level 1), the node itself (level 2, where §8.7 already
makes the keys illegal), and `defaults:` (level 3) — leaving the wait bounded
only by the `human:` block's own `timeout:`/`on_timeout:` pair. `on_error`
resolves normally through all four levels (§8.7, §9.3).
**Rationale**: [D52](#d52-human-node-shape) makes the node-level keys illegal
because a wait is not an activity timeout and re-prompting a human is not a
retry, but §9.3 said `defaults:` "applies to every node in every flow" and §8.5
said a `policy:` applies to "every node inside" — so the two fields a `human`
node may not declare could still reach it from two other levels, and the
document named no winner. The readings are not close: with
`defaults: { timeout: 90s }` and a `human:` block declaring `timeout: 24h`, one
implementation kills the wait after ninety seconds and the other waits a day,
from one spec — and the first one breaks the shipped
[`examples/triage-fanout`](../examples/triage-fanout), whose human step could
then never be reached. D52's reason is a statement about the *construct*, not
about which file the value was written in, so it has to hold wherever the value
comes from; the alternative would also make a `human` node's behavior depend on
whether some unrelated flow in the composition wanted a default timeout — the
kind of action at a distance [D2](#d2-singleton-sections-are-declared-in-exactly-one-file)
and PRD 5.7 reject elsewhere. Exempting `on_error` too was rejected for the
opposite reason: a delivery failure *is* an ordinary node error, so the strategy
covering it is ordinary policy, and §8.7 already keeps the key legal at the node.
The cost is that a composition cannot bound its human waits from one place —
which is what `human.timeout:` is for, one line at the node that owns the wait.
**A budget above a wait does not run while the wait is open** (§9.3). The four
levels this decision speaks about are the ones that resolve *onto the `human`
node*, and a pause reached inside a subflow has a fifth clock beside them: the
`flow:` node or `map` that dispatched the instance, which is not a `human` node
and whose `timeout:` §8.5 makes bound the entire instance. Left to run, it would
cap every nested wait at whatever `defaults:` said — this decision's own reading,
defeated one construct out — so the enclosing deadline is held still for as long
as a wait inside it is open. What that costs is nothing a composition can
observe except the thing it is for: the enclosing node's budget still bounds
every millisecond of *work* the instance does, which is what §9.2 says it bounds.
*PRD 5.5, 5.11, G3.*

### D103. `fallback` is a node-level `on_error` form only

`on_error: { fallback: … }` is legal as a node's own key (§9.3 level 2) and is a
compile error in the `defaults:` section (level 3) and in a `flow:` node's
`policy:` (level 1), where `on_error:` takes `fail` or `skip`. The published
schema enforces it at both levels (§9.2, §9.3, Appendix B).
**Rationale**: [D21](#d21-on_error-strategies-and-fallback-targets) fixes the
target as a **flow-local node id**, precisely so error routing stays visible to
the per-flow analyses (§7.8's reachability, §7.6.2's `dist`) — but levels 1 and 3
are not attached to a flow. `defaults:` reaches every node of every flow in the
composition and a `policy:` propagates into nested instantiations, so a
`fallback: cleanup` written once would have to resolve, per flow, in flows that
never declare a node by that name. The published schema accepted the form
(`policyBlock` reused `onError` verbatim) and the text said nothing, leaving a
validator implementer three defensible behaviors — reject; resolve per flow and
error wherever the id is missing; apply it where it resolves and silently fall
back to `fail` elsewhere — the last of which is a silent no-op of the kind
[D61](#d61-else-takes-the-literal-true) refuses, and the middle of which makes
one composition-wide default reject flows that have nothing to do with it.
Refusal is the reading that keeps D21's own justification true: a fallback is
control flow *inside* one graph, so it belongs on a node of that graph. Nothing
is lost — `retry` is a separate field, so "retry, then give up" is still one
`defaults:` block, and the fallback that mattered is one line on the node that
needs it. The alternative worth naming, a per-flow `defaults:` layer where a
flow-local target would be in scope, is exactly the flow-definition-level default
[D20](#d20-the-policy-resolution-chain-has-exactly-four-levels) already
considered and rejected to keep the chain as PRD 5.5 settles it. *PRD 5.5, G3.*

### D104. The idempotency key is the flattened instance path

The key an effect carries is the execution id followed by the frames of every
node crossed from the root flow instance to the effect site, outermost first —
each frame a node id, that node's traversal ordinal within its flow instance,
and, for a `map` node, the source-item index of the instance it dispatches — plus
one frame per flow-as-tool call crossed on the way, `<tool name>/<call ordinal>`
(§9.4). Its two carriers are a detached `map` dispatch (§8.6 rule 7) and a store
write (§11.4).
**Rationale**: PRD 5.6 and 5.8 both write the derivation as
`execution_id + node + item_index` and 5.8 raises it to "a named cross-cutting
rule", so the *principle* is settled; what "node" denotes is not, and this
grammar makes four constructs under which a node id names several distinct
effects in one execution. A flow instantiated twice — `a: {flow: flow.ingest}`
and `b: {flow: flow.ingest}` — puts the same store node `save` at two sites with
no `item_index` at all; nested maps repeat the inner index across outer items, so
inner item 0 under outer item 0 and inner item 0 under outer item 1 collide; a
node inside a bounded cycle (§7.4) executes twice in one instance; and an agent
node whose `tools:` names a `flow.*` (§5.4) may have the model instantiate that
flow any number of times in one execution, which is the fourth and the one no
node id can address at all — hence the extra frame, and hence PRD resolved q19's
positional key reuse across an agent-node retry, stated in §9.4. Under
at-least-once delivery a collision is not a cosmetic defect: the sink or store
dedupes the second write away, so the design meant to stop a message being lost
loses one. Reading "node" as the *occurrence* — the path that identifies which
instantiation, which item, and which traversal — is the only reading under which
the PRD's own dedupe rule preserves data, so it is a refinement of the settled
principle rather than a departure from it (prd.md's shorthand phrasing stands as
written; the precise form lives here). The traversal ordinal is the component
that does not fall out of "path", and it is required by the same argument: two
traversals of a store node in a bounded cycle are two writes an author expects to
land. It also costs nothing on the other side of the trade, because it counts
*executions within an instance*, which every kind of retry restarts or leaves
alone — so retried delivery still repeats one key, which is what at-least-once
means. Fixing a rendering (frames joined with `/`, integers in decimal, node ids
being identifiers and so separator-free) is what makes two implementations agree
on the string a sink sees, in the spirit of
[D34](#d34-the-store-op-catalog-is-normative-including-derived-output-schemas)'s
fixed output names. Every component is reproduced by a replay, because the
schedule is a pure function of the graph and the recorded outputs (§7.6.4), so
the key is stable across a resume. *PRD 5.6, 5.8, 5.12.*

### D105. A `map` dispatch isolates conversation history per instance

Every instance a `map` dispatches runs on a fresh conversation history that is
discarded when the instance completes; it neither reads nor appends to the
enclosing flow instance's `messages` channel, and no key opts out — `context:`
stays a `flow:`-node key (§10.4, §8.6 rule 13, §8.5).
**Rationale**: §10.4 scoped history across `flow:`-node boundaries only, so what
a `map`-dispatched *agent* sees was undefined — and the two readings differ in
what the model is sent. Treating an instance as an agent node of the enclosing
flow has five concurrent workers reading and appending to one `messages`,
interleaving five conversations into a history none of them can make sense of and
whose contents depend on completion order — the replay hazard PRD 5.6 names,
reintroduced in the one channel §7.6.4's canonical write order does not govern.
Isolation is the reading the rest of the document already implies:
[D29](#d29-map-targets-are-component-references-not-flow-local-node-ids) makes an
instance an isolated per-item invocation rather than a node of this flow, PRD 5.6
runs instances "in isolated item-scoped contexts", and PRD 5.7 settles history as
isolated across module boundaries with the opt-in placed at the *instantiation
site* — a `map` block, which declares no `context:`, is exactly such a site. Not
extending `context: inherit` to the map block is deliberate and is the same
argument one level down: inheriting into N concurrent instances would either fork
the channel N ways (so nothing is shared after all) or serialize the fan-out
(so `max_concurrency` means nothing). A dispatched flow's own `flow:` nodes keep
their `context:` key, sharing the instance's fresh history inward, which is all
the continuation any single item can coherently want. *PRD 5.6, 5.7.*

### D106. A provider kind's key row is closed

Beyond `kind:` and `description:`, a `provider.*` definition takes exactly the
keys §12.1's row for its own `kind` names; a key from another kind's row is a
compile error, and the published schema enforces it (§12.1, Appendix B).
**Rationale**: §12.1 already published a per-kind table with a Required and an
Optional column, and [D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)
already makes a key belonging to no row an error — but the table's status was
left unstated, so `{ kind: anthropic, api_key: "${K}", region: us-east-1 }` and
`{ kind: vertex, project: p, location: l, api_key: "${K}" }` sat in the gap:
refused by a reading of the table as closed, accepted by a reading of it as
illustrative, and accepted by the published schema, which branched on `kind:`
for *required* keys only. That inverts Appendix B's one-directional invariant in
the direction that matters least visibly — the schema was the *looser* artifact,
so an editor gave a clean bill of health to a definition `validate` rejects.

Closing the row is the reading that keeps D50's own justification true: an
ignored key is a setting whose author believes it is in effect, and a credential
or a region silently dropped is the most consequential form of that mistake —
the run reaches the wrong account, or reaches nothing and fails at first call
with a message pointing at the provider rather than at the spec. The alternative,
declaring the shared keys universal, was rejected because it is not true of the
two SDK-reached kinds: `bedrock` and `vertex` have no bare endpoint for
`base_url:` to name and no spec-composed request for `headers:` to ride on, so
admitting them there would ship keys with nothing to do — and `openai_compatible`
already exists for the deployment that really is talking to an HTTP endpoint.
The check costs one `if`/`then` per kind and reads the same literal the required
keys already branch on, which is what makes it decidable in one file, exactly as
§11.4's per-op parameter rows are ([D34](#d34-the-store-op-catalog-is-normative-including-derived-output-schemas)).
*PRD 5.9, G3.*

### D107. An `else:` edge requires a `when:`-guarded sibling

An edge carrying `else: true` MUST have at least one sibling outgoing edge from
the same node carrying `when:`; with none it is a compile error naming the node
and the edge (§7.2, §7.3). The check reads edge shapes only and is the
validator's, not the schema's.
**Rationale**: §7.3 rule 4 gives an `else:` edge exactly one behavior — taken
iff no guarded sibling was taken — so with no guarded sibling it is taken on
every pass, which is rule 2's unconditional edge under a keyword. That is the
inert key [D61](#d61-else-takes-the-literal-true) already refuses in its
`else: false` spelling, and refusing one while accepting the other would say
that a keyword meaning nothing is a mistake when it is written `false` and fine
when it is written `true`. The failure it prevents is not cosmetic:
`[{from: a, to: b}, {from: a, to: c, else: true}]` reads to its author as "go to
`c` only when the edge to `b` did not fire", and what it does is multicast to
both — two concurrent branches (§7.3 rule 6), a reduced-channel requirement the
author did not expect (§10.2), and possibly an unbalanced convergence downstream
(§7.6.2), none of it diagnosed. An unconditional edge is how "always go here" is
written, so nothing is lost by refusing the second spelling of it.

The rule is deliberately weaker than the two places a guarded sibling has to do
more than exist. §7.4 clause 2(ii) needs an in-SCC `else:` edge's guarded
sibling to *leave the SCC*
([D98](#d98-a-cel-exit-condition-is-about-the-in-scc-edges-going-false)), and
§7.3.1 clause 2 needs the guards to *cover* an enum's variants; satisfying this
rule satisfies neither of those, and it is stated separately because it is about
whether the keyword means anything at all, which is a question every `else:`
edge answers whether or not it sits in a cycle or beside an enum.

JSON Schema cannot express it — it relates two items of an `edges:` array
through a shared `from` value, and the schema constrains items without reference
to each other's values — so it joins `optional:` entries
([D89](#d89-optional-entries-must-name-declared-properties)) on Appendix B's
validator-owned list rather than being dropped for want of a per-file check.
*PRD 5.3, G3.*

### D108. The published schema's patterns stay in the interoperable regex subset

Every `pattern` in `schemas/agent-compose.schema.json` uses only the constructs
common to ECMA-262 and RE2: no lookahead, no lookbehind, no backreferences. The
no-`${ENV}` check therefore spells "unescaped" as the alternation
`(^|[^$])\$\{…\}` rather than as a lookbehind (§4.3, Appendix B).
**Rationale**: [D12](#d12-pattern-is-re2-format-is-a-closed-list) holds a spec
author's own `pattern:` to RE2 because a regex has to mean the same thing in the
Rust validator, in generated JS validation, and in a provider's
structured-output engine. The published schema is read by strictly more engines
than that — every editor and CI validator a project points at it — so the same
standard has to hold for the artifact, and the failure mode is worse than a
divergence: an RE2-backed validator cannot *compile* a schema containing
`(?<!…)`, so it rejects every file with a regex-compile error at `$defs`,
including the files that are correct. A rule the grammar states and the schema
enforces everywhere except on one class of toolchain is not the one-directional
invariant Appendix B claims.

The alternation is exactly equivalent, not an approximation: `${NAME}` is
unescaped when it starts the string or follows one character that is not the
`$` of `$${`, which is what the lookbehind asserted. The reason to record the
choice is that the lookbehind reads better and a future editor would otherwise
restore it as a simplification. *PRD 5.2, 5.12, G3.*

### D109. The single-string-property exception counts the whole property set

§6.1's raw-decode exception applies when a `tool.*` binding's `output` declares
**exactly one property and that property is string-typed**; the count is over
the whole declared property set, never over its string-typed members alone
(§6.1, both bindings).
**Rationale**: the clause read "declares exactly one string-typed property",
which is two sentences at once — "declares one property, and it is a string" and
"exactly one of its properties is a string". They disagree about
`output: { text: {type: string}, count: {type: integer} }`: the first decodes
stdout as JSON and reads both fields out of it, the second binds raw stdout to
`text` and leaves `count` with no source at all. One tool definition, two
runtime results — the same divergence
[D91](#d91-the-single-string-property-decode-exception-is-a-tool-surface-rule)
closed for the question of *which surface* the exception applies to, left open
one clause earlier for *when* it applies.

The whole-set reading is the one the exception's own justification supports. D91
keeps the exception on `tool.*` because a tool "has no other way to name raw
text" — an account of an output that *is* the text, which is one property, not
of a schema that happens to carry one string beside three integers. The other
reading has no answer for the fields it does not bind: the JSON that would have
populated them was never parsed, so they would have to be dropped, defaulted
(`default:` is illegal on a result surface, §3.6), or the call failed — three
further readings inside the reading. It would also make decoding turn on the
types of the fields the exception does *not* bind, so adding
`count: {type: integer}` beside a string field would silently switch stdout from
raw to parsed: the "decoding depends on the siblings" hazard D91 rejects a whole
surface over. Nothing static changes — both shapes are legal tool definitions
under either reading, so this fixes what a conforming implementation *does*, not
what it accepts, and there is nothing here for the published schema or a
negative fixture to reject. *PRD 5.5, G3.*

### D110. An absent value fails the read, and an absent output field writes nothing

Reading a value that is legally absent — a property listed in an object's
`optional:` (§3.4) and the `value` a `kv`/`blob` `get` omits on a miss (§11.4),
alongside [D78](#d78-channel-initial-values-and-reading-an-unset-channel)'s unset
channel, [D101](#d101-a-merge-channels-properties-are-unset-until-supplied)'s
unsupplied `merge` property, and
[D115](#d115-executionitem_index-is-absent-where-no-map-dispatch-encloses-the-expression)'s
`execution.item_index` outside a `map` dispatch — **fails the execution**,
naming what was absent
and the expression that read it. An output field a node's result does not carry
performs **no write**: the channel it would have written, remapped or
same-named, keeps whatever it held (§4.1, §8.0, §11.4).
**Rationale**: §11.4's catalog marks exactly one output field optional —
`{ value: V (optional), found: boolean }` on `kv get` and `blob get` — while
§8.0's write rule (then "each field of its output is written to the state
channel of the same name", with no clause for a field the result omits) and
§4.1's read scope both said nothing about the pass where the field is not
there. §8.8's own worked example is the case:
`writes: { value: prefs }` on a miss had one codegen skipping the write, another
writing `null` (a type error against `V`), and a third failing the node, while a
guard `load_prefs.output.value.theme == 'dark'` could error, read false, or
require a `has()`. That is the three-way divergence D101 was added to close for
a `merge` channel's properties, unanswered one level up at a node's output.

Both halves fall out of rules already here rather than adding any. Not writing
is the only choice that neither invents a value nor fails a node whose op
succeeded: `null` does not validate against `V`, and §3.6 refuses a defaulted
result for that reason exactly; failing would make `found: boolean` a field no
author could act on. The diagnostic is not lost, it moves — the channel is
unset if nothing else wrote it, so the failure arrives at the next read of it,
naming that channel, which is D78's rule and D78's message. Failing the *read*
of the absent value is the same posture one step over, and it is what CEL
already does when a select finds no such key, so the Rust interpreter and the
embedded JS evaluator agree without a special case (PRD 5.5's two-interpreter
discipline). It stays distinct from `skip`, which routes an output-referencing
guard `false` (§9.2,
[D97](#d97-a-skip-changes-guard-values-not-the-routing-algorithm)), on D97's own
line: `skip` continues past a node that never ran, while a `get` that ran and
answered `found: false` has already named the field an author should test.

Nothing here is statically checkable, and nothing needs to be: `found` and
`value` are derived rather than authored
([D34](#d34-the-store-op-catalog-is-normative-including-derived-output-schemas)),
so there is no shape for the published schema or a negative fixture to reject —
only a runtime rule two implementations now read the same way. *PRD 5.7, 5.8,
G3.*

### D111. Name-based wiring is type-checked in both directions

A **name-based read** requires its source to **satisfy** the declaration it
lands in: every value the source can hold is a legal value of that declared type
node. Its two sites are a node input field resolved from the same-named channel
or enclosing flow input (§8.0 steps 2–3) and a flow `outputs:` field read from
its channel (§7.5); the write side is §10.2's table
([D58](#d58-append-channels-take-one-element-per-write)), and an explicit
`input:` binding is typed by its CEL result instead (§4.1). A channel's
`reduce:` and its `default:` do not enter the read comparison.
**Rationale**: the write half of name-based wiring has an explicit table and the
read half had nothing. §7.5 and §10.3 required the channel a flow `outputs:`
field reads to *exist* and said nothing about its type, and §8.0's steps 2 and 3
resolve an input field from a channel with no expression to carry a type at all;
[D101](#d101-a-merge-channels-properties-are-unset-until-supplied) covers only
presence. So `outputs: { draft: {type: integer} }` over a channel
`draft: {type: string}` sat between two defensible validators — one folding it
into schema compatibility and rejecting it, one accepting it and failing at
materialization — with this document's layer split (schema compatibility is
specified by the PRD and implemented in the validator) readable as licence for
either. A flow's `outputs:` is where a module states its contract, and PRD 5.1
makes that contract interchangeable with a tool's, so "the type of what it
returns" is the one thing about it that cannot be left unfixed; leaving the
input half unstated beside it would have been the same hole one construct over.

The direction is the one the data forces: the channel supplies the value and the
declaration is the promise, so the source must be no wider than the destination,
never the reverse. Naming the surfaces and the direction is all this decision
does — the compatibility relation itself stays where the layer split puts it,
next to the write side's, so the two halves of one wiring cannot drift into two
subtyping rules. `reduce:` is excluded because §10.2 is about a *write*'s arity
while a read always takes the whole value: an `append` channel of
`items: {type: string}` still holds an array. `default:` is excluded because
§10.1 already makes it validate against the channel's own type, so it adds no
value the comparison has not seen; it is also the one key legal on a channel and
illegal on a result field (§3.6), which is why an author meets the two
declarations spelled differently and has to be told they are still compared.

The rule costs one thing worth naming: a channel with no `max_items` cannot feed
an `outputs:` field, because
[D10](#d10-max_items-is-required-on-result-schemas-and-fanned-out-arrays)
requires the bound on the field and only the channel's own declaration can keep
that promise. Adding it to the channel is one key, and it is the same bound the
field already carries — as `examples/triage-fanout`'s `patches` shows on both
sides. Presence is untouched and stays D78's and D101's: a channel that is unset
at quiescence, or a `merge` channel missing a property, fails at the read
(§10.1). Both sites are cross-file by nature — the flow or node in one file,
`state:` in another — so the check is the validator's alone, and Appendix B
lists it as such. *PRD 5.1, 5.2, 5.7.*

### D112. Balanced convergence compares one distance from each edge of the pair

For a fork `f`, a co-takeable pair `(e₁, e₂)` of it, and a node `d`, the
convergence at `d` is unbalanced when some `a ∈ dist(f, e₁, d)` differs from
some `b ∈ dist(f, e₂, d)` — one distance from each edge, never the union of the
two sides, and never a comparison inside one side (§7.6.2).
**Rationale**: the union reading fires where a single edge of the pair reaches
`d` at two depths and the other reaches it at none, which is not a second
arrival at all. Take a fork with co-takeable edges `e₁ → A` and `e₂ → B` (two
unconditional edges, as `classify` in
[`examples/triage-fanout`](../examples/triage-fanout/flows/triage.yml)), a
guarded shortcut inside `A`'s branch — `A → C when: g`, `A → D else: true`,
`D → C` — and a `B` branch that retires at `end`. Then
`dist(f, e₁, C) = {2, 3}`, `dist(f, e₂, C) = ∅`, the union holds two values, and
the union reading demands a compile error naming a pair one of whose edges never
reaches `C`. At run time `A`'s two out-edges are exclusive by §7.6.1 rule 1, so
`C` takes exactly one delivery per pass and never runs twice: a runtime-safe,
ordinary shape refused, which is the one direction
[D99](#d99-co-takeability-is-a-relation-on-a-pair-of-sibling-edges) says a
conservative static check must not go, with a diagnostic that misattributes the
two distances to a pair that cannot deliver them.

Nothing is lost on the soundness side, which is why the tightening is safe
rather than a relaxation of the guarantee. Two paths of different lengths from
`f` through `e₁` to `d` share a prefix and part at some node `g`, taking
distinct out-edges of it; their suffix lengths differ, since the prefix is
shared and the totals do not. So those two out-edges are either co-takeable —
making `g` a fork whose pair this same check compares, over exactly those two
suffix lengths, and rejects — or exclusive, in which case only one of the paths
is ever taken and there is no second delivery to refuse. The same-side case is
therefore already decided one fork down, and the union adds no rejection the
check needs; it only adds ones it must not make. Stating the comparison across
the pair is also what makes the error message true: the two distances it prints
are the ones the two named edges deliver. *PRD 5.3, 5.6, G3.*

### D113. `metadata_schema` is a `vector`-only key

`metadata_schema:` is legal on a `vector` store and a compile error on a `kv`
and a `blob` one; the published schema enforces all three (§11.1, Appendix B).
**Rationale**: §11.1 declared it "vector/blob optional" and nothing in the
grammar consumes it on a `blob`. No `blob` op takes `metadata` or `filter`
(§11.4's rows are `put` = `key`/`value`/`content_type`, `get` = `value`/`found`,
`list` = `keys`), and the tools §11.5 synthesizes for one — `<name>_get`,
`<name>_list`, and, unless `agent_access: read` withholds it, `<name>_put` —
have nowhere to put it either. An author following the table's own gloss
("filterable metadata") therefore writes a key that every write and every read
ignores, with no diagnostic. That is precisely the silent no-op this document
refuses for `else: false` ([D61](#d61-else-takes-the-literal-true)), for
`timeout:` on an async trigger ([D81](#d81-timeout-is-illegal-on-an-async-http-trigger)),
and for `storage_backends:` under `local`
([D87](#d87-local-is-a-reserved-target-and-deploylocalyml-carries-no-storage_backends)),
applied here to one of its own constructs.

The other repair — giving `blob` ops metadata — is the one that does not fit.
It would add parameters to three rows, a filter predicate the blob providers of
§14.3 (`local_fs`, `s3`, `gcs`) would each have to implement, and a synthesized
tool surface, all to answer a question PRD 5.8 never asks: its own example
declares `metadata_schema` on a vector store, where metadata exists to narrow a
similarity search, and its compile-check sentence pairs the two schemas with
*schema-checked ops*, which is exactly what a `blob` has none of. That is new
design surface, the class this appendix's preamble reserves for
[D37](#d37-agent_access-narrows-the-synthesized-store-tool-surface) and
[D51](#d51-agents-carry-max_tool_iterations-default-8), so it would need PRD
ratification; removing an inert key needs none, and leaves the additive path
open. The cost is one key on one kind, and what it bought was never available:
a `blob` whose attributes must be queryable pairs the blob with a `kv` store
under the same key, which is two definitions and no new grammar. *PRD 5.8, G3.*

### D114. A `vector` store with no `metadata_schema` derives matches with no `metadata` field

Where a `vector` store declares no `metadata_schema`, a `search`'s derived item
type is `{ id, score, text }` — the `metadata` field is absent, not an empty
object — and `filter:`/`metadata:` parameters have no legal key. A declared
`metadata_schema: {}` is the other spelling: `M` is the empty closed object and
the field is present (§11.4).
**Rationale**: §11.4 wrote the item type as
`{ id, score, text, metadata: M }` with `M` "the store's `metadata_schema`
object", and §11.1 makes that key optional
([D113](#d113-metadata_schema-is-a-vector-only-key) leaves it optional on the
one kind that keeps it), so a store declaring none left `M` undefined. Two
implementations follow and they disagree about everything downstream: one
derives `metadata: {}` and one omits the field, which changes whether
`search.output.matches[0].metadata` type-checks (§4.1), which channel type a
name-based write of `matches` is accepted by (§10.2,
[D58](#d58-append-channels-take-one-element-per-write)), and what Zod type
codegen emits (§3.8). That is the three-way divergence
[D101](#d101-a-merge-channels-properties-are-unset-until-supplied) and
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)
were added to close for other absent-value corners, one construct over.

Absence is the reading that says what is true: a store with no declared metadata
has none to return, and objects are closed
([D8](#d8-objects-are-closed)), so a `metadata: {}` would be a field that can
hold exactly one value forever — an inert surface the author never asked for,
which every consumer's channel and every generated type would still have to
carry. Reading it is then an unknown-field compile error naming the field, which
is the diagnostic an author who expected metadata should get, rather than a
silently empty object they read `has()` against. Keeping `metadata_schema: {}`
distinct costs nothing and is forced by §3.9, which makes `{}` legal at every
store schema surface: it is a real, if unusual, contract — "there is a metadata
object and it has no fields" — and the reason to state the pair together is that
nothing else in the grammar distinguishes an omitted field map from an empty
one. *PRD 5.8, 5.2, G3.*

### D115. `execution.item_index` is absent where no `map` dispatch encloses the expression

`execution.item_index` holds the source-item index of the **innermost** enclosing
`map` dispatch — in that map's own per-item expressions and everywhere inside the
instance it dispatches. Anywhere else it is absent, and reading it fails the
execution, which is
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)'s
rule and D110's diagnostic (§4.1, §11.4).
**Rationale**: §4.1 marked the member "present only inside a `map`-dispatched
instance" and then enumerated the legitimately-absent values as exactly three —
an unset channel, an unsupplied `merge` property, a value a result may omit —
with D110 repeating the same three. The index was a fourth case that neither
list held, so what a read of it does outside a map had no answer, and the case
is not exotic: [D83](#d83-item-derivation-is-traced-through-the-dispatch-binding)
contemplates one flow "dispatched by several maps and instantiated outside every
map as well", and §11.4's own fix list endorses a `key:` that reads
`execution.item_index` inside such a flow. One implementer rejects that
composition at compile time, one fails the read at run time, one emits `0` or
`null`: different accept sets and different runtime values from one spec.

Failing the read is the same choice for the same reason as everywhere else in
this cluster — a manufactured index is invented data, and `0` is worse than
invented, because it is a *real* item index that would make a direct
instantiation silently address whatever the first item addresses.

The static reading is the one worth ruling out explicitly, because it is the
tempting one. It cannot be made sound: implicit `manual` invocation makes **every
flow** runnable as a root instance
([D64](#d64-implicit-manual-invocation-is-a-cli-property-not-a-declared-trigger)),
so "every path to this flow passes through a map" is false of every flow in
every composition, and a check enforcing it would reject the very spelling
§11.4 endorses. A weaker check — refuse the read only in a flow no map dispatches
at all — is decidable, but it leaves untouched exactly the shape D83 declares
legal site by site, where the flow is a dispatch target at one site and a plain
`flow:` instantiation at another; that site still needs a runtime answer. Since
the runtime rule is required regardless, adding a static one on top would only
reject compositions D83 already accepts, for no case it actually closes.

The other half this entry fixes is *where* the index is available, which the old
wording got slightly wrong in the direction D83 depends on. A map's per-item
`input:` CEL is evaluated in the **enclosing** flow's scope, not inside the
dispatched instance — yet the index is well defined there, one value per item,
and D83's item-derivation test reads it there. "Inside a `map`-dispatched
instance" alone would put that surface outside the index's scope and unground
D83's first clause. Naming both surfaces, and naming the innermost map as the
one that supplies the value (§9.4 already says so for the idempotency key),
leaves one rule with one scope.

Nothing here is statically checkable and nothing needs to be: `execution` is a
derived root with a fixed member set, so there is no shape for the published
schema or a negative fixture to reject — only a runtime rule two
implementations now read the same way, exactly as with D110. *PRD 5.6, 5.7, G3.*

### D116. `embed.provider` is required, and no backend serves embeddings

A `vector` store's `embed:` block MUST name a `provider.*`; the storage backend
never computes vectors. The referenced provider must publish embedding
capability, checked at the store definition by §12.2's mechanism (§11.2, §2.3).
**Rationale**: §11.2 said an omitted `provider:` was "default resolved from the
target's backend", and no section defines that resolution. §14.3's storage
vocabulary (`sqlite_vec`, `chroma`, `pgvector`, `qdrant`) publishes no embedding
capability and its capability checks are about *vector storage*;
[D36](#d36-embedmodel-is-a-bare-provider-native-id-not-a-model-ref) said only
that the ref was optional; and under `--target local` the substitution consults
no alias and no per-kind default at all
([D87](#d87-local-is-a-reserved-target-and-deploylocalyml-carries-no-storage_backends)),
so under the zero-infra target the phrase named nothing whatsoever. One
implementer routes embeddings to chroma's server-side embedder, another errors
because `pgvector` and `sqlite_vec` cannot embed, and under `local` neither
`validate` nor codegen has a rule to follow — with M1's store-tool synthesis and
the mock-provider harness both needing one answer.

The phrase was also against a settled position, which is what decides the
direction of the fix. PRD 5.9 holds that **providers are logical-layer, not
per-target** — keys and URLs vary by env ref, the connection does not — while
`backend:` is the one store key that *does* fork per target (§11.3). Deriving
the embedding connection from the backend would make one composition embed with
different models' vector spaces under `staging` and under `local`, which is the
same store holding incomparable vectors. Splitting the two questions — `backend:`
says where the vectors live, `embed.provider` says what computes them — is what
keeps a store's embeddings identical across targets.

That leaves what an omission should mean, and the answer is that it cannot mean
anything the composition supplies. A "sole declared provider" default is
ambiguous the moment a project declares two, which
[`examples/triage-fanout`](../examples/triage-fanout/providers.yml) already does
— and wrong even when it is unambiguous, since a lone `anthropic` provider is a
perfectly legal composition and has no embeddings API at all. Deriving from the
model id would require the grammar to know which vendor owns
`text-embedding-3-small`, which is precisely the plugin knowledge PRD 5.9 keeps
out of the spec layer. So requiring the key is not a tightening chosen over a
workable default; it is the absence of one.
[D13](#d13-prompt-is-required-and-literal) is the precedent for the shape of the
move: the PRD's own illustrative snippet omits `prompt:` too, and an agent with
no instructions is not a specifiable component. Neither is a vector store whose
embeddings name no connection.

The cost is one line per vector store and the gains are concrete. The rule is
decidable in one file, so the published schema enforces it and a negative
fixture pins it (Appendix B) rather than it being a validator-only rule about a
value that does not exist. And PRD 5.9's least-privilege distribution becomes
computable for embeddings: an isolated deployment receives the env vars its
resolved providers reference, which requires the embedding connection to be a
named ref rather than a target-derived guess. A backend that genuinely embeds
server-side stays additive — that is a storage-plugin capability and would
arrive as one, published in §14.3's vocabulary and PRD-gated like any other new
design surface. *PRD 5.8, 5.9, G3.*

### D117. `payload.body` is an empty object on a bodyless request

On an `http` trigger, `payload.body` is the decoded JSON object of the request
body; a request that carries no body — every `GET`, and a body-bearing method
sent with an empty one — presents `{}`. A body that is present but is not a
decodable JSON object is rejected at request time (400) and starts no execution.
A `method: GET` trigger MUST NOT read *through* `payload.body` in its `input:`,
`session_key:`, or `callback:` CEL; reading it whole stays legal (§13.3).
**Rationale**: §13.3 fixed the member as "decoded JSON object" while listing
`GET` among the legal methods one row earlier, and a `GET` has no body — so
whether `payload.body` was then `{}`, absent (failing reads per D110), or a
request-time rejection was unstated, and a binding `goal: "payload.body.goal"`
compile-checks under all three. One generated app supplies `{}` and fails the
member read, another rejects the request: same spec, different HTTP behavior.

`{}` is the reading that keeps the declared shape true. The member stays present
and readable, so `has(payload.body.goal)` answers `false` instead of erroring,
and a member the object does not carry falls under
[D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)
with no new rule — where declaring the root itself absent would have needed a
fifth absent-value case for a member this document declares present. The 400 is
the other half of "decoded JSON object": a body the app could not decode is not
a payload, and refusing it before the execution exists is what keeps a
half-decoded object from ever reaching a binding.

Refusing the `GET`-through-`body` read follows this document's standing posture
on a guaranteed runtime failure that is statically visible — the escape edge of
[D19](#d19-max_iterations-semantics-and-the-escape-edge-rule), the no-dead-end
rules of [D71](#d71-no-silent-dead-ends-every-node-exits-and-every-run-starts),
the inert keyword of
[D107](#d107-an-else-edge-requires-a-when-guarded-sibling). `payload.body` is
`{}` on *every* request such a trigger can receive, so the read fails every
time; accepting it ships a route that can only ever 500, when `payload.query` is
one word away. Reading the object whole is left alone because it is not that
case: it binds `{}`, and whether a flow input field accepts an empty object is
an ordinary type check.

The check is the validator's, not the schema's, and deliberately so. Deciding it
means reading a CEL expression, and the schema's only instrument is a regex over
the string — which would also reject a CEL *string literal* that happens to
contain the text, making the schema stricter than `validate` and inverting
Appendix B's one-directional invariant, the failure
[D80](#d80-the-published-schemas-per-file-bounds-are-grammar-rules),
[D96](#d96-an-inline-exechttp-nodes-output-is-a-result-schema),
[D100](#d100-both-accepted-outcome-lists-are-non-empty-and-distinct) and
[D106](#d106-a-provider-kinds-key-row-is-closed) each exist to repair. Appendix B
already declines CEL surfaces on exactly that reasoning, so this joins the
validator-owned list rather than being sniffed for. *PRD 5.11, G3.*

### D118. A detached dispatch reaches no `human` node

A `map` dispatch declaring `detach: true` MUST NOT reach a `human` node, in the
sense §7.7 fixes — its target's own nodes, the flows that target instantiates,
its maps' targets, and the `flow.*` tools of any agent it reaches. The check is
target-independent, and it is the fourth reader of §7.7's relation (§8.6 rule 7,
§8.7). **Rationale**: [D94](#d94-a-detached-dispatch-is-resolved-at-dispatch)
makes a detached dispatch resolved at dispatch — "the last thing the graph knows
about that item" — and §8.7's runtime makes a pause a wait held **by an
execution**, published on that execution's status report and answered through its
resume route. The two do not compose. The join never observes the instance, so
the execution can finish while it is still in flight and the wait is dropped with
every other one that run was holding; and while the run is alive, whether the
pause is reachable at all depends on which of the two finishes first — a
composition with one joined route beside the detached one would settle the pause
at the joined route's pace, and the same composition without it would leave the
question published on an execution whose graph has already moved on. That is one
construct with two behaviours chosen by scheduling, which is the shape this
document refuses elsewhere by refusing the construct
([D71](#d71-no-silent-dead-ends-every-node-exits-and-every-run-starts),
[D107](#d107-an-else-edge-requires-a-when-guarded-sibling)).

Refusing it statically is also what keeps
[D102](#d102-a-human-node-resolves-no-timeout-and-no-retry-at-any-level)'s
promise honest one construct out. The enclosing node's budget is held still while
a pause below it is open (§9.3), which is a statement about a node that is
*waiting*; a detached dispatch is defined not to wait, so there is no budget to
hold and nothing for the promise to be about. The narrower alternatives were both
worse: letting the run end the wait makes an answer's reachability a race, and
keeping the execution alive until every detached instance quiesces would make a
fire-and-forget dispatch delay the enclosing run — precisely what rule 7's third
clause forbids. The construct an author reaching for it wants is a *joined*
dispatch, which is one key away. *PRD 5.5, 5.6, 5.11, G3.*

### D119. A refused tool call returns to the model, and a failed one ends the node

An agent's tool loop divides what can go wrong with a tool call in two, and the
line is **what the model could do about it**.

A call the tool's declared contract **refuses** is returned to the model as an
error tool result, and the loop turns again. Two things are refusals, and they
are refusals at every surface a tool can be attached from:

- **arguments the tool's own schema does not admit** — a `tool.*`'s `input:`
  (§6), a `flow.*`'s `inputs:` (§5.4), or a synthesized store tool's row of
  §11.4 (§11.5). One rule over all three, because §5.4 makes "attached as a tool"
  mean one thing and a surface answering this differently would make it mean two;
- **a call naming a tool the agent was not offered**, which is the same event
  with the contract missing entirely.

Everything else is the tool's **execution** failing — an `exec:` that exits
outside its accepted list, an `http:` whose response a non-2xx rule refuses, a
child flow instance that failed or quiesced without an output, a store backend
that could not answer, a result the tool's own `output:` refuses — and fails the
agent node, with §9's chain deciding the run exactly as before.

**Rationale**. A tool call is the one place a *model* proposes work, and the two
kinds of failure are not alike from where it sits. A schema refusal is a
statement about the call: the model chose the arguments, it can choose others,
and the composition's contract is intact either way — so ending the run over it
throws away a graph that was working because a model made a correctable mistake,
which is not what §9's `retry:`/`on_error:` chain is for. An execution failure is
a statement about the world: the command exited 2, the endpoint answered 500, the
child flow failed. Handing that back would ask the model to route around a broken
system, which is precisely the decision PRD 5.3 takes away from models — "the
runtime decides every transition" — and the ladders in §9 are the declared way an
author says what should happen instead.

The narrower alternatives were both considered and are worse. Failing the node on
every refusal — the rule this decision replaces — makes a composition's
reliability depend on a model never mis-typing an argument, and pushes authors
toward loosening schemas until nothing is checked, which costs the constrain ==
parse property PRD §9.16 is built on. Bouncing *everything* would leave the model
retrying an endpoint that is down, invisibly, until the loop's bound ran out,
with the real failure buried in a tool result no trace field carries as a node
error.

**Termination is unchanged, and that is what makes the bounce affordable.** A
correction is another model call, so a bounce costs one of the agent's
`max_tool_iterations` ([D51](#d51-agents-carry-max_tool_iterations-default-8),
§5) — the bound that already made the loop statically terminating, with no second
counter to reason about. Several refusals in one answer are corrected together
and cost one iteration between them, which is the honest reading of a bound that
counts *model calls*: the model gets one turn to fix everything it got wrong.
A model that never corrects spends the bound and fails the node, and that failure
**names the last refusal it was holding** — without it, a loop that ran out
because the model kept calling wrongly and a loop that ran out because the model
kept calling correctly and never answered produce the same sentence. *Holding* is
the operative word and is read strictly: a loop whose last call **worked** holds
none, so the failure names none, and the sentence never reports a mistake the
model already corrected.

Three consequences are stated where a reader meets them rather than derived:

- **every call of an answer comes back to the model, refused or not.** A refusal
  does not stop the loop over the calls of one answer — the calls after it still
  run, and each one's result or refusal is handed back. On the wire that is the
  call's own id being answered, because both surfaces refuse a request that
  leaves a `tool_use` id or a `tool_call_id` unanswered — with one exception,
  which is the surface's own rather than this runtime's choice: Chat Completions
  re-validates a replayed assistant turn's `tool_calls` against the request's
  `tools`, so a call naming a tool the agent was never offered may not be
  replayed there at all. That `tool_call` is dropped from the replayed turn and
  its refusal travels as a `user` turn instead, while the Messages API replays
  the `tool_use` and answers it with an `is_error` `tool_result`
  (`crates/mock-provider/WIRE-NOTES.md` (18)). The sentence the model is handed
  is the same either way. A failure still does stop the loop, because the node is
  ending;
- **a refused call spends no call ordinal** (§9.4). That ordinal counts how many
  times a flow-tool has been *invoked*, and a refused call reaches no flow: it is
  the next call that takes the frame the refused one was offered. A frame counted
  from the calls a model *made* would move every instance path behind a refusal —
  turning a mistake into a changed idempotency key for work that has nothing to
  do with it, and, under a node `retry:`, re-firing every child flow's effects on
  an attempt whose model simply got it right sooner;
- **the refusal names what a diagnostic would name** — the tool, the failing
  field, the constraint, and an excerpt of the offending value (PRD G3). It is
  one sentence written for two readers: the model, which has to act on it, and
  the person reading `ToolCallRecord.error`, which carries it verbatim as the
  `<message>` half of `docs/trace.md` §3's `<error name>: <message>` — so the two
  strings differ by the `ToolCallRefused: ` that format prefixes and by nothing
  else. Quoting the arguments back costs nothing `docs/trace.md` §11 was
  protecting, because the party being shown them is the party that composed
  them. The tool is named
  the way the **model** was offered it — a `tool.*`'s local name, not its address
  (§5.4, §6) — because a refusal is an instruction to call again and an
  identifier the model cannot call is not one. A mismatch on the *other* usage
  surface, where a `function:` node's binding is what missed the contract, names
  the address instead: there the reader is a person with a composition to fix.

The trace records a refused call under `outcome: "refused"` rather than reusing
`"failed"`, which cost a `trace_version` bump: version `3` defines `"failed"` as
a call that ended the node and whose answer the model never saw, and both halves
are false of a refusal. `docs/trace.md` §10.3.3 is that reasoning in full.
*PRD 5.1, 5.2, 5.3, 5.8, §9.14, §9.22, G3.*

### D120. A keyless `anthropic` or `openai` provider names its endpoint

On the two kinds with a default endpoint, `api_key:` is required when `base_url:`
is absent and optional when it is present; a provider declaring neither is a
compile error (`missing-credential`, §12.1). An absent key means the compiled
runtime sends **no** authentication header — no `x-api-key`, no `authorization` —
rather than an empty one. **Rationale**: §12.1's row made `api_key:` flatly
required on both kinds, which refuses the shape a corporate deployment actually
writes: model traffic goes through a gateway that injects the vendor credential
server-side, and the employee running the graph holds no key at all. The only
workaround was `kind: openai_compatible` with a re-pointed `model.*` — a
different plugin, a different settings schema, and `thinking:` no longer
type-checked — for a connection that is still talking to Claude. PRD 5.9's
"swapping a project from hosted to local inference is a one-line provider edit"
is the promise that row was breaking.

Making the key **simply optional** was the obvious repair and is the rejected
one. It admits `provider.x: { kind: anthropic }`, which validates, builds, ships,
and 401s on its first live call against `https://api.anthropic.com` — moving a
forgotten credential out of `validate` and into production. That is the failure
this document's whole static layer exists to prevent, and G3 makes the
diagnostic a product feature; trading it away to save one `if`/`then` is the
wrong side of that trade. The conditional keeps the forgotten-key report exactly
where it was and buys the gateway shape with a message that names both repairs —
declare the key, or name the endpoint that supplies one.

`base_url:` is the right discriminator because it is the only key in the
definition that can say the traffic is not going to the vendor. It is not a
proxy for intent: the runtime *resolves* an omitted `base_url:` to the vendor's
own host, so "no `base_url:`" is literally "reaching Anthropic's or OpenAI's
endpoint", where the key is not optional in any deployment. And a gateway that
authenticates callers with a token of its own is already served: `headers:` is
interpolable (§4.3 class 2), so `authorization: "Bearer ${PROXY_TOKEN}"` is a
declared header that reaches the wire and reads as what it is, rather than an
`api_key:` pretending to be a vendor credential.

Sending **no header** rather than an empty one is the other half, and it is not
cosmetic. `x-api-key: ""` and `authorization: Bearer ` are requests that *claim*
to authenticate and fail, which a gateway is entitled to reject before it ever
injects its own — and where the gateway forwards headers verbatim, an empty
credential arrives at the vendor as a 401 that names authentication rather than
as the absence the deployment intended. The rule is stated over the connection,
not over the kind: whichever wire a compiled graph reaches, a provider that
declares no `api_key:` sends no authentication header for it.

The rule is decidable in one file — `kind:`, `api_key:` and `base_url:` are three
literals in one mapping — so the published schema branches on it exactly as it
branches on the required keys already
([D106](#d106-a-provider-kinds-key-row-is-closed), Appendix B), and it is decided
in the same pass as those keys, one report with the kind's own span beside it.
The other four kinds keep their rows unchanged:
`azure_openai` reaches a per-resource deployment that has no default endpoint to
fall back to, so all three of its keys stay required;
`openai_compatible` already made `api_key:` optional beside a required
`base_url:`, which is the same posture arrived at from the other direction; and
`bedrock` and `vertex` authenticate through their cloud's own credential chain
with no header for this rule to be about. *PRD 5.9, G3.*

### D121. Durability adds no grammar: journaling is unconditional and the target binds the backend

Every invocation of every flow is **journaled**, with no key to turn it off and
none to turn it on, and `--target local` binds a SQLite journal file beside the
project. There is no `journal:` block, no deploy-file section, and no addition to
the published schema. [`docs/durability.md`](durability.md) is normative for the
record, its keys, and the replay that reads it back. **Rationale**: PRD resolved
q27 makes the journal a *deploy-target slot*, exactly as `storage_backends` are —
"the composition says nothing, the target binds it" — and every target this
compiler release can build is process-local, so every one of them binds the same
backend. A configuration surface is a choice expressed in grammar; with one
backend there is no choice, and a key whose only legal value is its default is a
key an author has to read and cannot use. resolved q27 also fixes the property
that key would otherwise carry: durability is "durable by default, zero
configuration, one file to delete".

The deploy-level surface arrives with the **first non-local backend** — the
Postgres journal a distributed target binds — which is the release where a
choice exists to express, and it will land in §14 beside `storage_backends:`
where it belongs. Reserving the key now would be reserving a shape nobody has
had to write against a backend nobody has implemented, which is the one kind of
forward-compatibility this document does not practise (§15's reserved
constructs are all *fully specified*).

Durability is **not** §14's checkpointing, and the two must not be read as one
rule. "`local` is not durably checkpointed; every other target is" — the property
`detach: true` keys off ([D59](#d59-checkpointing-is-a-target-property-and-detach-is-checked-per-target), §8.6 rule 7) — is
about a LangGraph **checkpointer**, which PRD resolved q26 rules out as this
project's durability mechanism in favour of journal + replay. `--target local` is
still the un-checkpointed target, `detach: true` is still legal only there, and
it is now also a durable one. *PRD 5.11, resolved q26–q29.*

### D122. Server tools are provider-side config, checked in two tiers

A provider MAY declare `server_tools:`, an array of config objects in that
provider's own wire vocabulary, and the runtime appends them to the `tools` of
every request that provider serves. Each entry requires a string `type:`;
everything else is the provider's and travels verbatim, as grammar 4.3 class 2
values. The compiler keeps a **curated table** of the server tools each kind is
known to serve: an entry naming one is checked strictly against it, and anything
the table cannot speak for is a **warning** that says so and is carried to the
wire unchanged. **Rationale**: PRD resolved q30. The governing constraint is *no
manual support treadmill* — a server tool the vendor ships tomorrow must be
usable the day it ships, without waiting for a compiler release — and the two
tiers are how that coexists with G3 diagnostics: the table buys a real error
message for what it knows, and buys nothing at the cost of a warning for what it
does not. A table that *gated* would be the treadmill; no table at all would
make a misspelled `max_uses` a 400 on the first live call, with no span.

**"Anything the table cannot speak for" is two things, not one.** A `type` it
does not name (`unknown-server-tool`), and a **key** it does not name inside a
`type` it does (`unknown-server-tool-field`). The second is the same rule read
at field granularity, and it has to be, for the same reason: a row is keyed on
`type` alone and is a snapshot of one tool at one release, vendors add
parameters to tools they already ship, and a strict tier nobody can opt an entry
out of would refuse them until a new binary shipped. What stays an **error** is
everything the table genuinely knows: a field it models given the wrong kind of
value, a value outside a stated range or closed set, a required field left out,
two fields the vendor refuses together. A field the vendor documents whose
interior the compiler's vocabulary cannot state — `file_search`'s recursive
`filters` — is *in* the row as an unconstrained key, since a documented
parameter warned about is a diagnostic that teaches nothing.

**The runtime dispatches nothing.** A server tool executes on the provider's
side, inside the model call, and its results arrive woven into the assistant's
turn — which is why the array is a wire object rather than a construct of this
grammar, and why a `server_tool_use` block is *not* a tool call the agent's loop
answers. It is also why replay is untouched: the use happens inside the recorded
model call (`docs/durability.md` §3.1).

**Launch scope is `anthropic` and `openai`, plus `openai_compatible`
unverified.** For Anthropic's Messages wire the table holds web search, web
fetch and code execution; for OpenAI it holds the built-in suite of the
**Responses** API — web search, file search, code interpreter, image generation
— which Chat Completions does not carry. So an `openai` provider that declares
`server_tools:` speaks the Responses API for **all** of its calls, and one that
declares none keeps Chat Completions: one provider, one wire, because a
connection that switched per request would make "what did this model see" depend
on which agent asked. `openai_compatible` takes the key with every entry
second-tier — a gateway may honour any vocabulary, and refusing would recreate
the treadmill — and rides its Chat Completions `tools` array. `azure_openai`,
`bedrock` and `vertex` refuse it outright rather than dropping it silently
(D50).

**Moving the wire moves what that wire has.** Two of §12.2's `settings:` keys
are Chat Completions' and have no Responses spelling — `stop:` and `seed:` — so
declaring a suite makes them a compile error on the models that connection
serves rather than a 400 on the first call. Two others change spelling and the
runtime translates them (`max_tokens` → `max_output_tokens`, `reasoning_effort`
→ `reasoning: { effort }`). One thing changes that the compiler does not decide:
the Responses API's service-side default for `store` is `true` where Chat
Completions' is `false`, so the provider retains prompts and completions for a
connection that has moved. The emitted request does not pin the key, because
`store: false` makes the service refuse a replayed `reasoning` item and a tool
loop replays every turn; `docs/topics/models.md` says so where an author meets
the seam.

**A suite belongs to a connection.** Every agent whose model resolves to that
provider holds it, and scoping a suite to one agent is done by defining a second
provider — providers are cheap. Failover capability is therefore per-chain-member
by construction, so `validate` **warns** when a route's members declare differing
suites: which tools were on offer depends on which member answered, and that is
a legal thing to want (a fallback vendor that has no web search is still a
fallback) as well as a real thing to know.

**One name, one tool — §11.5's rule, reached from the connection.** The suite is
appended to the same `tools` array the agent's own tools land in, and the
provider surfaces refuse a request offering two tools under one name, so
`tool-name-collision` is an **error** on two more pairs: two entries of one suite
that reach the wire as one tool, and a server tool whose name an attached
`tool.*`/`flow.*` or a synthesized store tool already takes on an agent whose
model reaches that provider. The first is what the Messages wire's canonical
`name:` pinning makes decidable — both dated `code_execution_*` revisions *are*
`code_execution`, so the strict tier checking each entry in isolation would let
the pair through to a 400 with no span, which is the failure that pinning exists
to prevent. The second is stated over the Messages wire alone: it is the wire
where a server tool and a client tool sit under one key, a Responses built-in is
addressed by its `type` while a function tool carries a `name`, and no table
could say what a gateway keys its vocabulary on. *PRD 5.9, resolved q30.*

### D123. A built-in is one `tools:` entry carrying its own bounds

The four runtime built-ins (§5.5) are attached from an agent's `tools:` list, one
name per entry, spelled as a **single-key mapping** whose key is the built-in's
address and whose value is the bounds that address requires — `root:` on all
four, and `timeout:` on `builtin.bash` as well:

```yaml
tools:
  - tool.repo_grep
  - builtin.read_file: { root: "${WORKSPACE}" }
  - builtin.bash:      { root: "${WORKSPACE}", timeout: 30s }
```

**Rationale**. PRD resolved q31 fixes three things this spelling has to carry at
once, and they pull against the shapes that would otherwise be obvious. The
opt-in is "one tool name at a time … never ambient, and never a single switch
that grants the set, because *which capabilities does this agent hold* must be
readable off the node that holds them"; the bounds are mandatory; and the set is
closed and named. A key of its own — `builtins: [bash, read_file]` — would answer
the first and lose the second, because a list of names has nowhere to put a root,
and every alternative that puts the roots somewhere else (a sibling `builtin_
root:`, a block above the list) separates the capability from its bound by
exactly the distance a reader has to close to answer the only question that
matters about it. Putting the bounds *in the entry* is what makes the answer
local: the line that grants `bash` is the line that says where it runs and for
how long.

**The entry is where it is because the wire is one array.** A built-in is offered
to the model beside the agent's `tool.*`s and its stores' synthesized tools —
§11.5's one-name-one-tool rule reaches it unchanged, and a `tool.bash` on the same
agent is a collision — so a second list would have made "what is this agent
offered" a question with two places to look and one of them able to contradict
the other. It also settles the ordering with no new rule: entries reach the wire
in the order `tools:` declares, then the stores'.

**Why a single-key mapping rather than a discriminator object.**
`{ builtin: bash, root: … }` was the other candidate and reads worse in exactly
the place this decision is about: the name of the capability stops being the
entry's subject and becomes one field of it, three characters from a `root:` that
looks like a sibling rather than a bound. The mapping form also gives the
published schema a precise shape with no `if`/`then` — four named properties,
`additionalProperties: false`, `minProperties`/`maxProperties` of 1 — so "one
name at a time" is enforced by an editor before `validate` ever runs, and each
name's own bounds are checked against its own row (Appendix B).

**`root:` is required on `builtin.bash` too**, though q31 introduces it as the
file tools'. The headline is "bounded by a mandatory root and a timeout", and a
shell whose working directory defaulted to wherever the runtime happened to be
started would be the ambient capability the whole decision refuses — read off no
entry, different on a developer's machine and a deployment's.

**Required means non-empty**, on all four, for exactly that reason and in two
places. `root: ""` is a compile error, because it is a key present and a bound
absent — `""` resolves to the process's own working directory, so an entry
spelling it would grant precisely the ambient capability the paragraph above
refuses, while *looking* bounded to a reader. And because the value is
interpolable, the same hole is reachable through an environment variable that is
set and empty: `${WORKSPACE}` satisfying the presence check of PRD 5.9 with
nothing in it. The parser cannot see that one, so the runtime refuses an empty
*resolved* root as an execution failure, under §9 like every other bound the
call could not honour. One rule, checked wherever it can be broken. `timeout:` is
required rather than defaulted for the same reason and with the same words: "a
model holding bash is arbitrary code execution on the host running the graph,
which is why every bound here is explicit". A default is a bound nobody wrote and
nobody read.

`timeout:` is **illegal on the file tools** rather than accepted and ignored,
which is [D50](#d50-unknown-keys-are-errors-everywhere-except-plugin-config-objects)'s
rule reaching the smallest surface it has: there is no command there to bound, and
a key that did nothing would teach a reader that the file tools were bounded in
time when they are not. `root:`, by contrast, is one key with one meaning on all
four — the directory this tool may not leave — which is what lets an author read
four entries sharing a `${WORKSPACE}` as one grant.

**What the bounds are not.** Neither is a parameter: the model names a `path:` and
a `command:`, never a root and never a deadline, because a model that could widen
its own bound would not be bounded. And a built-in takes no `retry:`/`timeout:`/
`on_error:` of its own, exactly as a `tool.*` definition does not
([D25](#d25-tool-defs-require-description-input-and-output-and-exactly-one-binding),
§6.2): policy is a property of the use site, and the use site here is the agent
node, whose §9.2 deadline bounds the whole loop that a `timeout:` bounds one
command of.

**Failure, refusal, trace and journal are all borrowed rather than invented.** A
call the argument schema refuses bounces back to the model
([D119](#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node));
a root escape, a nonzero exit, a timeout kill and a missing shell are execution
failures that end the node under §9. The trace records a built-in call as the
`ToolCallRecord` it is, with the built-in's address as the target and, per
`docs/trace.md` §11, no result; the journal records the answer in full, so a
resumed execution consumes a recorded `bash` instead of running it a second time
(`docs/durability.md` §3.2). That last one is the property that makes a built-in
worth having over a hand-rolled `exec:` tool at all — it is the same property,
reached with none of the boilerplate. *PRD 5.5, 5.12, resolved q31, G3.*

### D124. A built-in's deadline kills the command's process group, not just the shell

`builtin.bash` runs its command in a **process group of its own**, and the
`timeout:` kills the group. So does an abort — a §9.2 node deadline, or a
cancelled run — and so does a stop signal delivered to the process running the
graph while a command is in flight.

**Rationale**. §5.5 promises that a command which outruns its `timeout:` "is
killed", and the bound is one of the two things q31 says a built-in *has*: "a
model holding bash is arbitrary code execution on the host running the graph,
which is why every bound here is explicit". A kill aimed at the shell's own pid
does not keep that promise, because the shell is almost never where the work is.
`bash -c 'npm run build'` forks; so does a pipeline, a subshell, a command list.
Kill the shell and every one of those children keeps running — and keeps writing
inside the `root:` the attachment bounded it to — while the graph has already
reported the call as failed and moved on. With `retry: 2` that is two generations
of one command writing one root with the composition believing exactly one is
live; with `on_error: skip` it is a downstream node reading files a "killed"
command is still producing. The bound would be a message rather than a fact.

**What this costs and why it is worth it.** A detached command is out of the
**terminal's** reach as well as the shell's: its group is no longer the
foreground one, so the `SIGINT` a person types no longer reaches it the way it
reaches an `exec:` tool's child. That would have traded one orphan for another,
so the runtime closes it directly — while a command is running, `SIGINT` and
`SIGTERM` sweep the live groups and are then re-raised, leaving the exit
behaviour, the exit status and `serve`'s own shutdown exactly as they were. The
handlers exist only for as long as a command does.

**What is still out of reach**, and is said rather than implied: a process that
*left* the group on purpose — `setsid`, a shell that turned job control on
(`set -m`), a daemon that double-forks. Those escape a hand-rolled `exec:` tool
identically, and containing them is the distribution work's, beside the container
and syscall isolation §5.5 defers there. The runtime stops **reading** what such a
process holds rather than waiting on it, so the call is still bounded even when
the process is not.

**The same rule inside this process.** `builtin.list` is the one built-in whose
work is the runtime's own — a walk over a directory the model named, matching a
glob the model wrote — and both of those size it. An abort stops that walk where
it is, for the reason it kills a process group: an activity the graph has stopped
*waiting* for is not an activity that may go on working, and a compiled graph is
embedded code, so a listing left running is a core taken from every other
execution in the same process. The walk also hands the event loop back as it
goes, because a deadline is a timer and a timer cannot fire inside work that
never yields. *PRD resolved q31, §5.5, §9.2.*

### D125. Inbound `auth:` is one scheme per trigger, with env-ref secrets

An `http` trigger's `auth:` block declares **exactly one** of `bearer:` and
`hmac:`; a block declaring neither and a block declaring both are each a compile
error naming the repair, and both blocks' secrets take the env-ref value form
alone (§13.3, §4.3).

**Rationale**. PRD resolved q32 settles the two kinds and settles that auth is
**per trigger**; what this entry fixes is the shape. *Exactly one* rather than a
set, because one request carries one credential: a route that accepted either a
bearer token or a signature would be exactly as open as its weaker half, and
"which one did this caller use" is not a question the deployment gets to answer
after the fact. It is the shape a `tool.*` implementation binding already takes
([D25](#d25-tool-defs-require-description-input-and-output-and-exactly-one-binding)),
and it is refused the same way — `missing-key` naming both spellings,
`conflicting-keys` on the second — so an author meets one rule twice rather than
two rules once each. *Env refs only* is [D41](#d41-env-ref-forms-and-the-secret-field-list)
applied to two new field names: a `token:` or `secret:` written as a literal is
a credential committed to a repository, which is the failure §4.3 exists to
prevent, and the value form is what makes `validate` able to say so without ever
holding the secret.

**Why not vendor presets** — `auth: { github: … }`, `auth: { stripe: … }` —
which is the shape an author coming from a webhook vendor's documentation would
reach for first. That is the **support treadmill** resolved q30 refused for
server tools, arriving through a second door: a preset is a promise to track a
vendor's signing scheme across releases, and a vendor that changes one leaves
every deployment pinned to a compiler release rather than to a configuration.
The configurable `hmac:` covers the GitHub-shaped family — digest, encoding,
header, prefix — which is what most vendors actually speak, and the schemes it
cannot express (Stripe's timestamped `t.body` with a tolerance window) are named
out of scope in §13.3 rather than half-modelled. Presets can grow later as a
curated table on q30's own terms, and adding one then breaks nothing written
against this shape. *PRD resolved q32, §13.3, §4.3, G3.*

### D126. `callback_auth:` makes `callback_allow:` mandatory

Declaring `callback_auth:` on an `http` trigger makes `callback_allow:` a
required key of that trigger, reported as its own diagnostic class,
`missing-callback-allowlist` (§13.3). Neither key is legal on a trigger with no
`callback:`.

**Rationale**. PRD resolved q33 ratifies the rule; this entry records that it is
an **error rather than a warning**, and why the asymmetry with a plain callback
is the right one. The callback URL is read from the request payload
([D110](#d110-an-absent-value-fails-the-read-and-an-absent-output-field-writes-nothing)),
so it is attacker-controlled by construction. A deployment that attaches a
credential to its deliveries and does not say where they may go will hand that
credential to whichever host a payload named — and a warning is precisely the
wrong instrument for it, because the composition that ships is the one that
validated. A trigger declaring **no** outbound auth is untouched: it signs
nothing, claims nothing, may POST anywhere, and §13.3 says so in as many words,
because the test posture is worth being able to write and worth recognising in
review.

The code is its own rather than a `missing-key` for the reason
`missing-credential` is
([D120](#d120-a-keyless-anthropic-or-openai-provider-names-its-endpoint)): what
is absent is decided by a sibling value, and the repair is a choice of two —
declare the allowlist, or drop the auth. The **no-`callback:`** half is
[D81](#d81-timeout-is-illegal-on-an-async-http-trigger)'s posture rather than a
new one: a key describing a delivery the trigger never makes changes nothing
observable, and a key whose author expected it to do something gets a diagnostic
rather than silence. *PRD resolved q33, §13.3, G3.*

### D127. A callback allowlist entry is a wildcard URL, and the delivery wire is fixed

A `callback_allow:` entry is an absolute `http`/`https` URL — the scheme spelled
lowercase, as the match will read it — in which `*` matches any run of
characters, matched against the whole callback URL; it names a host — the run
from the scheme to the first `/`, `?` or `#` — and an entry where that run is
empty is a compile error; the list is non-empty. Every delivery carries the
`X-AgentCompose-*` headers §13.3 tabulates, over the `Content-Type`,
`Content-Length` and `Host` any POST of a JSON body carries, and a
`callback_auth.bearer.header:` naming one of those is a compile error; outbound
`hmac:` signing is HMAC-SHA256 in hex with no keys of its own (§13.3).

**Rationale**. *One wildcard kind*, unlike §5.5's `glob:`: `**` earns its
existence where a path tree has a directory boundary to be significant about,
and a URL has no such boundary — `*` against the whole string is what an author
writing `https://hooks.example.com/*` already means, and a second wildcard would
only invite the question of what it did differently. The cost of having no
boundary is that a `*` **crosses `/` and `?`**, so a wildcard reaching the host
constrains no host at all: `https://*.hooks.example.com/*` is satisfied by
`https://attacker.test/collect?x=.hooks.example.com/y`, because the first `*` is
free to consume a host, a path and a query on its way to the literal that
follows, and `https://hooks.example.com*` by
`https://hooks.example.com.evil.test/collect`, because a name is a prefix of
longer ones. §13.3 states that where an author writes one, and *`validate` does
not refuse it*, because refusing would be taking the language decision that
question belongs to. A wildcard bounded to a single label inside the authority —
stopping at `.`, `:`, `@` and `/` — would make that first entry mean what its
author intends, and it is one wildcard with two meanings, which the PRD owns and
CLAUDE.md's PRD discipline puts there before an implementation. Refusing looks
like the conservative half and is not: the entry an author writes for a
multi-tenant receiver has no compiling enumeration, so on a trigger whose
`callback_auth:` makes the list mandatory
([D126](#d126-callback_auth-makes-callback_allow-mandatory)) the only repair
left is dropping the outbound auth — trading a broad allowlist for no allowlist
and no signature, which is the posture D126 exists to prevent. And the direction
of a later change is safe: bounding the wildcard narrows what an already
compiling entry matches, so a resolved question refuses deliveries that were
admitted rather than admitting deliveries that were refused.

*A host, though*, because that much is not about breadth: the run from the
scheme to the first `/`, `?` or `#` is what names a receiver, and
`https:///deliveries` names none — an entry no callback URL was written to
match, which is the same statically visible dead surface the empty list is. The
rule is about the *entry*, not about matching: `*` crosses every delimiter
wherever it is legal, because the alternative — a wildcard that stopped at one —
is the second kind this entry leaves to the PRD.

*Scheme-anchored*, because a match that could not name the scheme would
let one entry admit URLs that merely begin with the same characters. *And
spelled lowercase*, because the entry is compared to the URL as text: `HTTPS://`
is the scheme a callback is delivered over written in a case the match will
never see, so the entry admits nothing — the empty list's dead surface in a
single entry, and the one refusal here whose message has to name the *spelling*
rather than the two schemes, since telling that author their scheme is not one
of two schemes, one of which is theirs, is a message with no repair in it.
*`http` stays legal*: localhost development is the common first case, and
refusing it would push every author to a workaround worse than the rule.
*Non-empty*,
because an allowlist satisfied by nothing refuses every delivery — a statically
visible webhook that can never fire, and the same guaranteed-dead-end posture
§7.6.3 takes. Matching itself is a
**runtime** rule: the URL does not exist until the payload arrives, so `validate`
owns entry shape and nothing more.

*The delivery wire is normative in the grammar* rather than left to the emitter
because it is the half a **receiver** implements, and a receiver is code nobody
in this repository writes. PRD resolved q34 and q35 fix what a delivery carries
and that retries make ordering by arrival wrong; naming the exact headers here
is what lets a receiver be written against the language rather than against an
observed release. Outbound signing takes no `algorithm:`/`encoding:` for the same
reason: one recipe verifying every agent-compose deployment is worth more than a
knob, and the inbound block is where a vendor's choices have to be matched
because there the vendor made them.

*And naming them normatively reserves them*: a `callback_auth.bearer.header:`
inside the `X-AgentCompose-` namespace is refused, because the delivery is
already writing there. Two values under one header name is not a configuration a
receiver can read — it gets whichever its HTTP stack kept, or the pair joined —
so the same sentence that lets a receiver be written against this table has to
stop a trigger from contradicting it. The prefix rather than the five spellings,
so a sixth header added to the wire needs no second rule; **outbound only**, so
that an inbound `auth:` naming `X-AgentCompose-Signature` — a trigger receiving
another deployment's callbacks — stays exactly as writable as one naming
GitHub's.

*And the reservation is about the collision, not the namespace*, so it covers
the three headers a delivery writes without this table's help:
`Content-Type: application/json`, the `Content-Length` that frames the report,
and the `Host` the allowlist admitted. A token asked for under one of those is
the same two-values-one-name failure read from the transport's side, and a worse
one to debug, because the delivery is refused before any receiver code runs —
415, a body framed by a credential's length, or a request that reached a
different host entirely. Those three are matched **whole** rather than as a
prefix, since `X-Content-Type` and `Content-Type-Signature` are the receiver's
own names and a rule that swallowed them would refuse a configuration that
collides with nothing. *PRD resolved q33, q34, q35, §13.3.*

### D128. A placement is a named claim, and its members are components

`placements:` is keyed by an **identifier** (§2.1) naming a claim, and each entry
lists the components that claim runs. The previous shape — keyed by component
address, carrying `runtime: isolated|colocated` and a reserved `network:` — is
retired ([D47](#d47-placement-entries-take-runtime-plus-a-reserved-network--retired)).

**Rationale**. PRD resolved q37 and q38 settle the topology and the binding
surface together, and the pair leaves the old shape with nothing to say. The hub
owns the graph and workers dial out to it, so a placement can never be an
address: a Mac behind NAT joins the way a CI runner joins, and what it offers is
a *claim* — `agent-compose worker --join <hub> --claims mac,gpu`. That makes the
name the binding surface, and a name is exactly what an address-keyed mapping
has nowhere to put. The direction is the one Kubernetes labels take and for the
same reason: which machine satisfies a claim is decided by whoever shows up, so
several workers claiming one name form a pool, and the spec never learns their
addresses.

`runtime:` went the same way. `isolated` versus `colocated` was a *mode* a
component declared about itself, and under hub-and-spoke the mode is implied by
whether the component is claimed at all: a placed component runs on a worker
process, an unplaced one runs on the hub, and there is no third answer for a
keyword to select. `network:` recorded a sandbox intent the v1 containment story
does not have — PRD resolved q31 puts containment beyond the process boundary
out of scope, and resolved q44 names it out again — so it would have been a key
that read like a security control and was not.

**Why re-shaping was available at all.** Reserved grammar is fully specified,
parsed, type-checked and carried into the IR *and executes as a no-op* (§15).
The purpose of that posture is exactly this: the shape may be re-cut before its
first execution, because nothing has run and nothing can have depended on what
it did. The alternative — keeping the address-keyed grammar beside the claims
model — would have shipped two placement languages, one of them dead, to spare a
migration no deployment had yet made. What the posture does **not** license is
re-shaping a construct whose absence of runtime is invisible from outside, which
is the asymmetry §15 draws around the authentication keys.

A component in **no** placement executes on the hub. That is stated in §14.1 as
the default rather than enforced as a rule, because there is nothing to enforce:
`placements:` names the exceptions, and a project that names none is a
single-process deployment. *PRD 5.10, resolved q37, q38, q40, q44.*

### D129. Placement members are agents and tools, disjoint, and colocated with what attaches them

A placement's `members:` is a non-empty list of `agent.*` and `tool.*` addresses
that MUST resolve, each named once. A `flow.*` member is refused with a message
naming the deferral. One component may be a member of at most one placement. A
`tool.*` whose placement differs from that of an agent attaching it — including
an agent with no placement at all — is a compile error, and so is a placement
reached only through a `flow.*` that agent attaches.

**Rationale**, one clause at a time.

*Agents and tools only.* An agent is one model call and a tool is one
implementation: each is a unit of work a hub can hand to a worker, and each is
where a machine's capability actually lives — the signing keys, the GPU, the
licensed binary. A flow is a subgraph, and what schedules a subgraph is the hub's
scheduler, journal and wait board. Placing one would mean shipping the scheduler
to the worker — a second scheduler and, behind it, a second journal, which is
the peer-partition shape PRD resolved q37 rejects — or quietly placing every node
the flow reaches, which is a different feature wearing one address. PRD resolved
q44 names flow placement out of v1, so the refusal is a **deferral** and its
message says so: the composition is well-formed, and what the author asked for is
a feature this release does not have. Telling that author "expected an `agent.*`
or `tool.*` reference" would read as a spelling correction for a decision the PRD
took deliberately, which is why the `flow.` prefix is intercepted before the
address is read.

*Non-empty.* A placement with no members is a claim nothing is ever dispatched
under — a statically visible dead surface, the same posture §13.3 takes to an
empty `callback_allow:`.

*Each named once.* A repeated entry is a different failure from a shared one and
gets a different rule, because disjointness has nothing to say about it: a
component written twice in one list holds one answer, written twice, and a
message naming "both `mac` and `mac`" would offer a choice between one thing and
itself. It is the repeated-entry rule §5.4 already applies to an agent's
`tools:` and `stores:`, spelled the same way and carrying the same code, so an
author meets one rule about repeated list entries rather than two.

*Disjoint.* Workers claiming `mac` and workers claiming `gpu` are different
machines by construction; that is what a claim is for. A component named by both
gives the hub two answers to "which worker runs this", and whichever it picked
would be invisible in the spec. The choice belongs to the author, in the file
where both lines are, so the diagnostic names both placements and labels the
first.

*Colocated with what attaches it.* This is the clause that is not obvious, and
it follows from PRD resolved q40: the hub ships the **whole** artifact to every
worker, with per-placement slicing deferred. So a placement decides which
*process* runs a node, never which code exists where — and an attached tool is
called from inside its agent's own tool loop, in the process running the agent.
The tool's own placement therefore gets no say, and a composition that declares
one is holding two answers again. The asymmetry is deliberate: a tool with **no**
placement attached to a placed agent is fine, because the artifact really is
everywhere and the tool has claimed nothing; a **placed** tool attached to an
agent with no placement is refused, because that agent runs on the hub and would
drag the tool there — a placement written, accepted, and silently ignored, which
is the failure mode worth a compile error. A placed tool reached from a
`function:` node (§8.4) is untouched: nothing there disagrees with it, and that
case is what placing a tool is for.

*Colocated through an attached flow, too.* The rule is about the agent's
**process**, not about the `tool.` prefix, and §5.4 puts a second construct in
that process: a `flow.*` in a `tools:` list starts an instance inside the tool
loop, exactly where a tool call happens. Reading the flow's own address — "a
flow cannot be placed, so there is no claim to disagree with" — is true of the
flow and false of everything it reaches, and stopping there would leave the last
row of the table open one indirection out: a `mac` tool reached only through an
attached flow would run on the hub, silently, with the deploy file saying
otherwise. So the check walks what the instance reaches (§7.7 clauses 1 and 2)
and holds each placed component to the same four rows. The walk stops where
clauses 3 and 4 stop, at an agent's own attached `tool.*`, because that pair is
already governed by the direct rule and following it would report one
contradiction twice. The other direction stays untouched and matters as much: a
flow instantiated by a `flow:` node (§8.5) is scheduled by the hub, so every
placement inside it is honoured and there is nothing to refuse — the repair the
diagnostic offers third. *PRD 5.10, resolved q38, q40, q44.*

### D130. The `hub` block, and the conditional join token

`hub:` takes `join_token:` — an `${ENV}` value-form reference, never a literal —
and `public_url:` — an absolute `http`/`https` URL naming a host, with no
wildcard. `join_token:` is REQUIRED when `placements:` is non-empty and optional
otherwise; a `hub:` block with neither key is legal, and so is one declared where
no placement is.

**Rationale**. *A block rather than two top-level keys*, because both describe
one thing — the process that owns the graph (PRD resolved q37) — and a deploy
file's top level is a short list of sections whose members should each be a
subject rather than a field.

*An env reference only.* The join token is a credential, and §4.3's rule is that
the spec never contains one. Nothing about this key is special enough to be the
exception, and the one kind is bearer (PRD resolved q38), so there is no scheme
to select.

*Conditionally required.* Declaring a placement is declaring that some node runs
somewhere else, and the only way a worker becomes that somewhere else is an
authenticated join: a mesh described without a token is a mesh nobody can join,
which `validate` can see and a deployment would discover at the first dispatch.
The failure gets a code of its own — `missing-join-token` — rather than
`missing-key`, for the reason
[D126](#d126-callback_auth-makes-callback_allow-mandatory) and
[D120](#d120-a-keyless-anthropic-or-openai-provider-names-its-endpoint) have
theirs: what is absent is decided by a *sibling*, and the repair is a choice of
two, so the message names both — declare the token, or remove the placements.
The rule reads `join_token:` as **written** rather than as parsed, so an author
who wrote a literal is told what is wrong with the literal and is not also told
the key is missing.

*Legal on its own.* `public_url:` is useful with no placements at all: PRD
resolved q44's fourth invariant makes ingress name-addressed, so every URL a
deployment hands out derives from a configurable base, and a single-process
`serve` behind a load balancer needs that base as much as a mesh does.

*No wildcard in `public_url:`.* §13.3's `callback_allow:` entries are **patterns**
matched against a URL somebody else supplied, and `*` there is the match. This is
the opposite direction: the base is ours, written out, and concatenated into URLs
this deployment publishes. A `*` in it is a character in a hostname that resolves
nowhere. The rest of the shape rules are `callback_allow:`'s, deliberately, down
to the wording — an author meeting both surfaces should meet one set of URL
rules ([D127](#d127-a-callback-allowlist-entry-is-a-wildcard-url-and-the-delivery-wire-is-fixed)).

**The trust model this key carries is v1's whole answer, and §14.2 states it
rather than implying it**: holding the join token is being trusted with the mesh
— the whole artifact, the right to claim any placement, and the journal's effect
stream. The per-placement environment manifest checked at join is self-reported
presence, not proof, because proving a secret would mean sending it and PRD
resolved q41 exists to forbid that. Per-placement credentials are an additive
later hardening. Saying so in the grammar is the same discipline §13.3 applies to
a constant-time comparison: a security property a reader could assume wrongly is
worse than one they have to look up. *PRD 5.10, resolved q37, q38, q41, q44.*

---

## Appendix B — Editor integration

[`schemas/agent-compose.schema.json`](../schemas/agent-compose.schema.json) is a
draft 2020-12 schema covering **one file at a time**. It validates the structural
shape of any spec or deploy file: section placement, definition-key patterns,
required fields, enum values, and closed objects, with descriptions on every
property for editor hovers.

```jsonc
// .vscode/settings.json
{
  "yaml.schemas": {
    "./schemas/agent-compose.schema.json": ["main.yml", "**/*.yml"]
  }
}
```

It is necessarily **looser** than `agent-compose validate`, which is the
authority. The schema cannot see across files, so it does not check:

- reference resolution or reference typing (`model.smart` existing, and being a
  model);
- singleton-section cardinality across files, or duplicate addresses;
- whether `version:` is present in a file that turns out to be the entrypoint
  (it is required whenever `imports:` or a deploy section is present, which is
  the best per-file approximation);
- any rule listed in PRD §7 M0's static-check list: exhaustiveness, SCC
  termination, fan-out bounding, reducer-write rules, trigger input
  compatibility, sync-trigger interrupt-freedom, store schema/keying rules,
  session coherence, provider settings/capability checks, env-ref presence,
  unreachable nodes, undefined channels, and schema compatibility — including
  both sites of a name-based read, a node input field resolved from a channel
  (§8.0) and a flow `outputs:` field read from one (§7.5, D111), whose two
  declarations routinely sit in different files;
- the graph analyses of §7.6, §7.7, and §7.8, which need the whole flow graph
  rather than a key-and-value pair: balanced convergence (§7.6.2, D112), the
  no-dead-end rules of §7.6.3 apart from the `start` edge below, component
  reachability (§7.7) and the four checks over it, node reachability from
  `start` (§7.8, D95), `map.over` dominance (§8.6 rule 11), the totality of
  `flow:`-node bindings (§8.0, D68), and the injectivity of a node's *effective*
  write map (§8.0, D93) — JSON Schema constrains property *names*, never the set
  of values, and the name-based half of that map is decided by the `state:`
  section in another file;
- rules relating two siblings whose correspondence JSON Schema cannot express:
  `optional:` entries naming declared properties (§3.4, D89) — an array's items
  cannot be constrained against a sibling object's keys — the `when:`-guarded
  sibling an `else: true` edge requires (§7.3, D107), which relates two items of
  one `edges:` array through a shared `from` value, and item-derivation of
  a store key (§11.4, D83), which is a path through bindings in other files;
- rules that turn on what a CEL expression *reads* rather than on its shape: a
  `method: GET` trigger reading through `payload.body` (§13.3, D117). Both
  values sit in one trigger object, but deciding it needs the expression
  grammar — a regex over the string would also reject a CEL *string literal*
  containing the same text, making the schema stricter than `validate` and
  inverting the invariant this appendix closes with;
- rules that key off the file's *name* or the active target rather than its
  content: no `storage_backends:` in `deploy/local.yml` and the existence of
  `deploy/<name>.yml` (§14, D87), and `detach: true` under a checkpointed target
  (§8.6 rule 7, D59);
- context-sensitive schema rules whose surface is not syntactically identifiable
  in one file. `max_items` is the example of the split: on **every** result
  surface (§3.5) — `agent.output`, `tool.output`, `flow.outputs`,
  `human.output`, an inline `exec:`/`http:` node's `output`, and the two store
  schemas — the surface is a named key, so the schema *does* require it there,
  and refuses `default:` there, on the same list (§3.5 clause 1, §3.6, D96); as
  the target of a `map.over` path (§3.5 clause 2) it depends on resolving a path
  through other files, so only the validator can require it there.

What the schema *does* enforce beyond plain shape, because the deciding value is
a literal in the same object: store-op parameter sets per `op` (§11.4) and the
three parameters that are literals rather than CEL — `top_k` and `limit` typed
as integers in their ranges, `content_type` as a string carrying no env ref
(§8.8, §11.4) — store definition key sets per `kind`, both the schema each kind
requires and the ones it refuses, `metadata_schema` being `vector`'s alone
(§11.1, D113) and the `provider:` a `vector` store's `embed:` block must name
(§11.2, D116), provider key sets per `kind` — both halves, the required keys and
the closed row the optional ones live in (§12.1, D106), including the one
required key that is *conditional*, an `anthropic` or `openai` provider's
`api_key:` where no `base_url:` names a gateway (§12.1, D120), which is a second
`if`/`then` on the same object rather than a rule about another file — trigger
keys per `type` (§13) including the `respond`/`timeout` and `respond`/`callback`
pairings, the scheme counts on both auth blocks — exactly one on `auth:`, at
least one on `callback_auth:` — and the two conditionals the callback keys carry:
`callback_auth:` or `callback_allow:` requiring a `callback:` for either to
describe, and `callback_auth:` requiring `callback_allow:`, which is that same
conditional-required-key shape a second time (§13.3, D126, D127), the deploy
layer's third instance of it — `hub.join_token:` required as soon as
`placements:` is non-empty, which is one `if`/`then` over two sections of one
file (§14.2, D130) — and, beside it, a placement's non-empty `members:` list, its
entries' distinctness, and the `agent.*`/`tool.*` pattern they take (§14.1,
D129), the map form
rules and the `on_item_error` shape (§8.6) — including the confinement of
`input:`/`writes:`/`detach:` to the homogeneous form (rule 7, D85) and the
absence of any `context:` key, which is a `flow:` node's alone because a
dispatch's history isolation is unconditional (rule 13, D105) —
the field-map-only `input:` on the node kinds that name their
fields (§8.0, D88), the non-empty `expect_exit`/`expect_status` lists (§6.1), the
direct-XOR-route split on model definitions (§12.2), the built-in entries of an
agent's `tools:` — one name per entry over the closed four, `root:` required on
every one of them and `timeout:` required on `builtin.bash` and refused on the
file tools (§5.5, D123), which is an `if`/`then` keyed on the entry's own
*type* rather than on a sibling literal: a string is an address and a mapping is
a built-in, so an editor underlines the missing `root:` rather than reporting
that the entry is neither kind of thing — the `human` timeout/route
pairing (§8.7) and the absence of node-level `timeout:`/`retry:` on a `human`
node (§8.7, D52 — the other two levels of that exemption are resolution
semantics, with nothing to reject), the `fail`/`skip`-only `on_error:` in
`defaults:` and in a flow node's `policy:` (§9.3, D103),
the inline-`http` `input:`-versus-`body:`/`query:` rule (§8.3),
**identical** duplicate edges and the `max_iterations`/`when:` pairing on one
edge (§7.2, D90) — `uniqueItems` on `edges:` catches byte-identical edge
objects, while §7.2's rule keys on `from`/`to`/`when` alone, so two edges
differing only in a `max_iterations` are a duplicate only the validator sees —
the presence of one unconditional-or-`else` edge leaving
`start` (§7.6.3 — an `edges:` array is one value, so this one *is* per-file),
the reserved-root exclusions on node ids, edge endpoints, control targets, and a
map's `as:` (§2.5), and the absence of `${ENV}` tokens on the surfaces where §4.3
makes them illegal and a single string is the whole surface (`prompt:`, model
`id:`, `embed.model:`, a trigger's `path:`/`cron:`/`timezone:`, the `header:` and
`prefix:` of an `auth:`/`callback_auth:` scheme and every `callback_allow:`
entry (§13.3), and a `blob put`'s `content_type:` — §4.3 class 3, D92). The
validator owns the rest of class 3: CEL surfaces need the expression grammar,
and descriptions and schema literals would need the same `not` repeated on
dozens of properties, which the one-directional invariant does not require — a
file the schema lets through is still rejected by `validate`.

**Diagnostics.** Where a construct has variants, the schema branches on the
literal that selects the variant — a node's kind key, a trigger's `type:`, a
model's `route:` — with `if`/`then` rather than a bare `oneOf` over the whole
variant list. A `oneOf` reports one error against the whole object ("is not valid
under any of the schemas listed in the `oneOf` keyword"), which in an editor
underlines the entire node and names nothing; branching reports the real error
against the offending key. The conformance suite pins this: each negative fixture
declares both the instance location and the schema keyword that must reject it
(`crates/compose-core/tests/schema_conformance.rs`).

**Regex dialect.** Every `pattern` in the schema stays inside the interoperable
subset — the ECMA-262 constructs RE2 also implements, so no lookaround and no
backreferences (Decision
[D108](#d108-the-published-schemas-patterns-stay-in-the-interoperable-regex-subset)).
This is [D12](#d12-pattern-is-re2-format-is-a-closed-list)'s standard for a spec
author's `pattern:`, applied to the artifact that checks it: a schema an
RE2-backed validator cannot compile rejects *every* file with a regex error
rather than validating any of them. The one place it costs a spelling is the
no-`${ENV}` check, which reads `(^|[^$])\$\{…\}` — start-of-string, or one
character that is not the `$${` escape's `$` — where a lookbehind would say the
same thing more briefly.

A file that passes the schema and fails `validate` is normal and expected; a file
that fails the schema always fails `validate`. Keeping that direction is why the
schema stops short of guessing: `queue_url` on an event source is a plain
interpolable string, because §4.3's secret-field list is closed and does not name
it (§14.4), and a trigger `path:` is only required to start with `/` and carry no
whitespace, because the router's own parameter syntax (`/reviews/:id`) is opaque
to the grammar (§13.3).

---

## Appendix C — Construct reference card

Shape sketch, not a document: `<x>` is a placeholder, `?` marks an optional key,
`...` elides. It does not parse as YAML and is not meant to be copied — for
copyable specs see [`examples/`](../examples).

```yaml
# ---- spec file --------------------------------------------------------------
version: "0.1"                      # entrypoint + deploy files
imports: [ "<relative path>", ... ] # entrypoint only
defaults: { retry: {...}, timeout: <dur>, on_error: fail|skip }  # no fallback (D103)
state:    { <channel>: <type node + reduce/default> }
triggers: { <name>: <trigger> }

agent.<name>:
  model: model.<m>                  # required
  prompt: <text>                    # required
  output: <field map>               # required
  input: <field map>                # optional (default string-in)
  tools:                            # references, and built-ins with their bounds
    - tool.<t> | flow.<f>
    - builtin.read_file: { root: <dir> }        # also write_file, list
    - builtin.bash:      { root: <dir>, timeout: <dur> }
  stores: [store.<s>]
  max_tool_iterations: <int>        # default 8

tool.<name>:
  description: <text>               # required, LLM-facing
  input: <field map>                # required
  output: <field map>               # required
  exec|http|function: {...}         # exactly one

flow.<name>:
  description: <text>               # required when used as a tool
  inputs: <field map>
  outputs: <field map>              # required; each field reads the same-named
                                    # channel, which must satisfy it — as must
                                    # any name-based input read (D111)
  nodes: { <id>: <node> }
  edges: [ { from, to, when?, else?, max_iterations? } ]
          # else: true needs a when:-guarded sibling (D107);
          # max_iterations needs a when: on its own edge (D90)

store.<name>:
  kind: kv|vector|blob              # required
  scope: execution|session|global   # required
  value_schema: <field map>         # kv only
  metadata_schema: <field map>      # vector only — no blob op reads it (D113)
  embed: { model, provider, dimensions? }    # vector — `provider:` is a
                                    # provider.* ref and is required; a storage
                                    # backend never computes vectors (D116)
  backend: <alias>
  agent_access: read|read_write

provider.<name>: { kind: ..., api_key: "${ENV}", base_url: "${ENV}", ... }
                 # keys beyond kind/description are per kind — 12.1's row is
                 # closed, and another kind's key is an error (D106)
                 # anthropic/openai: api_key required unless base_url names a
                 # gateway, and then no auth header is sent at all (D120)
model.<name>:    { provider: provider.<p>, id: <string>, settings: {...} }
model.<name>:    { route: [model.<a>, model.<b>], route_on: [...] }

# ---- node shapes ------------------------------------------------------------
# <bindings> = field map, or a bare CEL scalar where D88 allows one (marked)
{ agent: agent.<a>,  input: <bindings>, writes: {...}, retry/timeout/on_error }
                     # scalar input: only for a string-in agent
{ function: tool.<t>, input: <field map>, writes: {...} }
{ flow: flow.<f>,    input: <field map>, writes: {...}, context: isolated|inherit,
                     policy: {...},                    # for the nodes inside —
                                                       # on_error: fail|skip only
                     retry/timeout/on_error }          # for the instance itself
{ exec: { command, args?, cwd?, env?, expect_exit?, output? },
                     input: <bindings>, writes: {...} }   # scalar input: stdin
{ http: { method, url, headers?, query?, body?, expect_status?, output? },
                     input: <field map>, writes: {...} }
{ human: { input, output, timeout?, on_timeout? }, input: <field map>, writes: {...},
                     on_error }             # no retry/timeout, at any level (D102)
{ store: store.<s>, op: <op>,     # params are exactly the op's row (11.4):
                     # get/delete: key | set: key,value | list: prefix?,limit
                     # search: query,top_k,filter? | upsert: key,value,metadata?
                     # put: key,value,content_type?
                     # top_k/limit/content_type are literals, not CEL (8.8)
                     # a get that misses returns found:false and no value:
                     # no write, and reading .output.value fails (D110)
                     writes: {...} }
{ map: { over, as?, node | (route_by + routes + default?),
         max_concurrency,
         on_item_error?,           # fail | skip | { retry: {max, backoff, ...} }
         input?,                   # field map, or a scalar for a string-in agent
         writes?,
         detach? } }               # input/writes/detach: homogeneous form only —
                                   # a routed map declares all three per route.
                                   # no context: — a dispatch always isolates
                                   # conversation history (D105)
# route: { node, max_concurrency?, input?, writes?, detach? }

# ---- deploy file ------------------------------------------------------------
version: "0.1"
hub:              { join_token?: ${VAR}, public_url?: "https://<host>" }
placements:       { <name>: { members: [agent.*|tool.*, ...], description? } }
storage_backends: { defaults: { kv|vector|blob: {...} }, aliases: { <alias>: {...} } }
event_sources:    { <name>: { kind: ..., ... } }
```

