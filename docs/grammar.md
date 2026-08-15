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
| **Deploy file** | `--target <name>` → `deploy/<name>.yml` | `version`, `placements`, `storage_backends`, `event_sources` |

A spec file that declares `placements`, `storage_backends`, or `event_sources`
is a compile error, and a deploy file that declares definitions, `imports`,
`state`, `triggers`, or `defaults` is a compile error. This is the mechanical
enforcement of the PRD 5.8 per-target invariant: only the deploy layer forks per
environment.

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
- A version outside the supported set is refused with a pointer to
  `agent-compose migrate`; old syntax is never silently reinterpreted (PRD §9.5).

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
- A path MAY contain `..` but MUST resolve inside the project root.
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
| `store.<s>.embed.provider` | `provider.*` | which connection serves embeddings (§11.2); `vector` stores only |
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
  still-required `email`.
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
  `execution.item_index` (integer, present only inside a `map`-dispatched
  instance).
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
| **2. Interpolable** | `http` node and `http:` tool-binding `url` and `headers` values; the whole `exec:` surface — `command`, every entry of `args`, `cwd`, and `env` values, on both the tool binding (§6.1) and the inline node (§8.2); provider `headers` values and the non-secret provider keys of §12.1 (`region`, `location`, `project`, `organization`, `profile`, `api_version`); non-secret `storage_backends` and `event_sources` config values (§14.2, §14.3) | embedded `${NAME}` tokens are substituted at process start |
| **3. No refs** | **everything else** | an unescaped `${NAME}` token is a **compile error** naming the field |

Class 3 therefore covers, among others: prompts; every `description:`; every
part of a schema (`enum` members, `pattern`, `format`, `default:` literals); CEL
expressions on every surface; model `id` and every value inside `settings:`;
`embed.model` (§11.2); every identifier and reference position (node ids,
channel names, typed addresses, a store's `backend:` alias, a tool's
`function.name`, an event trigger's `source:`); `version:`; `imports:` entries;
a trigger's `path:`, `cron:`, and `timezone:` (§13.3, §13.4); a `blob put`'s
`content_type:` (§11.4); and every enum-valued key.

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

Env refs **survive unresolved into the IR**. `validate` checks syntax only;
`build`/`serve`/`run` check presence and fail fast naming the missing variable
(PRD 5.9).

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
| `tools` | array of `tool.*` / `flow.*` | no | `[]` | PRD 5.5, 5.1 |
| `stores` | array of `store.*` | no | `[]` | PRD 5.8 |
| `description` | string | no | — | documentation only; not LLM-facing (agents are not tools) |
| `max_tool_iterations` | integer 1..50 | no | `8` | bounds the intra-agent tool loop (D51) |

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

---

## 6. Tool definitions

One definition, two usage surfaces: attached to an agent (LLM-discovered,
nondeterministic) and invoked as a `function` node (graph-invoked, deterministic).
The definition is shared; validation is surface-specific (PRD 5.5).

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
validated against `output`, except when `output` declares exactly one
string-typed property, in which case trimmed raw stdout binds to it.

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
exactly one string-typed property, in which case the raw response text binds to
it — the same `tool.*`-surface exception the `exec` binding carries above, and
inapplicable on inline nodes for the same reason (D91). A status outside
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
Codegen emits one counter per bounded edge into the graph state; iteration
boundaries are checkpoint/resume points.

### 7.5 Instantiation, inputs, and outputs

- **Inputs**: `flow.<f>.inputs` is the module's parameter surface. Inside the
  flow, `input.<field>` is in scope everywhere (§4.1).
- **Outputs**: at quiescence (§7.6.3), each field of `outputs:` is read from the
  state channel of the same name; that channel MUST be declared in `state:`
  (§10) or it is a compile error. There is no `returns:` binding — use a node
  `writes:` remap to feed a differently-named channel
  (Decision [D53](#d53-flow-outputs-are-name-based-from-state)).
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
by the taken edges leaving `start`. When every node of step *k* has completed,
its writes are applied to state (§7.6.4) and its outgoing edges are evaluated
(§7.3); the union of the targets of all edges taken in step *k* is step *k+1*.
The instance finishes when that union is empty (§7.6.3).

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
every edge counts as one step. If for some node `d` the set
`dist(f, e₁, d) ∪ dist(f, e₂, d)` holds two different values, the convergence at
`d` is **unbalanced** and is a compile error naming `f`, `d`, the two edges, and
the differing distances (Decisions
[D69](#d69-execution-is-stepwise-and-convergence-is-a-per-step-join-over-taken-branches),
[D99](#d99-co-takeability-is-a-relation-on-a-pair-of-sibling-edges)).

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
— whose distances to `merge` are `{1}` and `{2}`: `merge` would run in step 1 and
again in step 2. That is the unbalanced-convergence compile error; the fixes are
to route the short branch through the same depth, or to make the pair exclusive
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

Three static checks ask whether a flow can *reach* something: session coherence
(§11.3), sync-trigger interrupt-freedom (§13.3, §8.7), and recursion (§7.5). They
share **one** relation, defined here once so that they cannot drift apart
(Decision [D86](#d86-component-reachability-is-one-relation-and-it-crosses-every-invocation-edge)).

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
| recursion (§7.5) | flow definitions | a flow reaches itself |

The relation is uniform across the three on purpose. An interrupt inside a
flow-as-tool is still an interrupt in the middle of a synchronous request, and
PRD 5.11's settled position is that a `respond: sync` flow is *statically*
interrupt-free; a session-scoped store reached through a map-dispatched flow
still needs a session identity; and recursion through a tool attachment is still
recursion. Clause 4 — traversal into `tools:` — is the one every earlier
per-check wording left unstated.

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
a store op has no input schema, and its parameters are the per-op CEL values of
§11.4, written out in the node and resolved directly against the roots of §4.1.
Nothing falls through by name there — an omitted `key:` is a missing required
parameter, never a lookup of a channel named `key`.

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

**Writing.** After a node completes, each field of its output is written to the
state channel of the same name **if such a channel is declared**; fields with no
matching channel stay node-scoped and remain readable as `<node>.output.<field>`
by that node's outgoing edge guards and by `map.over` (PRD 5.7 tier 1). `writes:`
remaps the destination:

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
  model chooses arguments at runtime.

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
| `max_concurrency` | integer 1..256 | **yes** | — | node-wide bound (D28) |
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

Human-in-the-loop pause (PRD 5.5). Grammar is active in v0; runtime support may
land in M2 — the same reserved-grammar move as `placements`.

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
delivery failures). A flow a `respond: sync` http trigger targets MUST NOT
**reach** a `human` node, in the sense §7.7 fixes — which includes a `human` node
inside a `map`-dispatched flow and inside a flow attached to an agent's `tools:`
(PRD 5.11).

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
own row in §11.4, each a CEL value in the scope of §4.1, and none of them
resolves by name from a channel or a flow input (§8.0). The op's output schema is
derived from the store definition (§11.4) and is written by name like any other
node output.

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

```
exec_01/a/0/save/0          # store node `save`, in flow.ingest instantiated by node `a`
exec_01/b/0/save/0          # …and by node `b`: a different write, a different key
exec_01/outer/0/3/inner/0/0/save/0   # `save` under item 0 of `inner`, itself item 3 of `outer`
exec_01/dispatch/0/7        # the detached dispatch of item 7 by map node `dispatch`
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
is the schema-compatibility rule for state wiring (§8.0, §10.3), and it is
decided from declared types alone
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
  embed: { model: text-embedding-3-small, dimensions: 1536 }
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
| `metadata_schema` | field map (result surface, §3.5) | `vector`/`blob` optional; `kv` illegal | filterable metadata |
| `embed` | block | `vector` required; others illegal | §11.2 |
| `backend` | identifier (bare alias) | no | abstract slot; never provider config |
| `description` | string | no | LLM-facing for agent-attached stores |
| `agent_access` | `read` \| `read_write` | no (default `read_write`) | narrows the synthesized tool surface |

### 11.2 `embed` (vector only)

| Key | Type | Required | Notes |
|---|---|---|---|
| `model` | string (non-empty) | yes | provider-native embedding model id (a bare string, not a `model.*` ref — D36); no env refs, exactly as a model `id` (§4.3) |
| `provider` | `provider.*` ref | no | which connection serves it; default resolved from the target's backend |
| `dimensions` | integer ≥ 1 | no | asserted against the backend's index |

### 11.3 Backends and scope

- `backend:` names an **abstract alias** defined per target in
  `deploy/<target>.yml` → `storage_backends.aliases` (§14.2). Resolution order:
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
store's `value_schema` object; `M` its `metadata_schema` object.

| kind | `op` | Parameters | Output |
|---|---|---|---|
| `kv` | `get` | `key` (CEL string) | `{ value: V (optional), found: boolean }` |
| `kv` | `set` | `key`, `value` (map field→CEL matching `V`) | `{ key: string }` |
| `kv` | `delete` | `key` | `{ deleted: boolean }` |
| `kv` | `list` | `prefix` (CEL string, optional), `limit` (integer 1..1000, required) | `{ keys: array<string> }` |
| `vector` | `search` | `query` (CEL string), `top_k` (integer 1..100, required), `filter` (map metadata-field→CEL, optional) | `{ matches: array<{ id: string, score: number, text: string, metadata: M }> }` |
| `vector` | `upsert` | `key`, `value` (CEL string — the text), `metadata` (map field→CEL, optional) | `{ id: string }` |
| `vector` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `put` | `key`, `value` (CEL string), `content_type` (string, optional) | `{ key: string }` |
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
  *looks* item-derived at the store node. `key: "input.text"`,
  `key: "execution.item_index"`, or dropping the map's `input:` so the whole item
  is passed all satisfy it.

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
| kind-specific keys | per plugin | per kind (below) | validated against the plugin's published schema |

v0 provider kinds and the keys each one takes, `kind:` and `description:` aside:

| `kind` | Required | Optional |
|---|---|---|
| `anthropic` | `api_key` | `base_url`, `headers` |
| `openai` | `api_key` | `base_url`, `headers`, `organization` |
| `openai_compatible` | `base_url` | `api_key`, `headers` |
| `azure_openai` | `base_url`, `api_key`, `api_version` | `headers` |
| `bedrock` | `region` | `access_key_id`, `secret_access_key`, `session_token`, `profile` |
| `vertex` | `project`, `location` | `credentials_json` |

`region`, `location`, `project`, `organization`, `profile`, and `api_version` are
plain strings and MAY be interpolated; the credential keys listed in §4.3 MUST be
env-ref values.

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

`payload` shape: `payload.body` (decoded JSON object), `payload.query` (map of
string), `payload.headers` (map of string, lowercase names), `payload.path`
(string), `payload.method` (string).

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
| `source` | identifier | yes | logical name; bound to infrastructure by `event_sources:` (§14.3) |
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

- `deploy/local.yml` is OPTIONAL. When present it MAY declare `placements:` and
  `event_sources:` — both are reserved grammar (§15), parsed, type-checked, and
  carried into the IR under every target including `local`, so neither is inert
  there.
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

placements:
  agent.researcher: { runtime: isolated, network: egress }
  flow.review_loop: { runtime: colocated }

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

### 14.1 `placements` (RESERVED — parsed and validated, no-op in v0)

Keys are component addresses (`agent.*`, `tool.*`, `flow.*`) that MUST resolve in
the composition.

| Key | Type | Required | Notes |
|---|---|---|---|
| `runtime` | `isolated` \| `colocated` | yes | own instance/container vs in-process |
| `network` | `none` \| `egress` \| `all` | no (default `all`) | sandbox network policy (reserved) |
| `description` | string | no | documentation only (D54) |

`--target local` implies everything colocated in one process. `--target
distributed` (post-v0) stitches boundaries with remote subgraphs (PRD 5.10).

### 14.2 `storage_backends`

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

### 14.3 `event_sources` (RESERVED — parsed and validated, no-op in v0)

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
| `placements` | parsed + validated, no-op | M3 |
| `event_sources` | parsed + validated, no-op | M3 |
| `triggers.<t>.type: schedule` | parsed + validated, no-op | M3 |
| `triggers.<t>.type: event` | parsed + validated, no-op | M3 |
| `human` nodes | grammar active; runtime may land later | M2 |
| `network:` on a placement | parsed, no-op | M3 |

---

## Appendix A — Decisions

Every entry is a place the PRD left the *grammar shape* open. Semantics stay
inside what the PRD settles; the cross-reference names the section each decision
must remain consistent with.

Two entries are different in kind and are labelled **PRD-extending**:
[D37](#d37-agent_access-narrows-the-synthesized-store-tool-surface) and
[D51](#d51-agents-carry-max_tool_iterations-default-8) each add a key answering a
question the PRD does not ask. They are consistent with the sections they cite
and neither contradicts a settled position, but they are *new design surface*,
not shape decisions — so under CLAUDE.md's PRD discipline each must land in the
PRD's Open Questions and be resolved there before the affected area (store tool
synthesis; the agent tool loop) is implemented. If either is declined, dropping
it from this document costs one key and one default; nothing else in the grammar
depends on them. No other entry in this appendix introduces a construct the PRD
does not already imply.

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
ambiguous. Result decoding (JSON, or raw text into a single string field) is
specified so `output` is always honored. *PRD 5.5.*

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
which is also what makes them the natural unit for `runtime: isolated`.
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

Each `kind`/`op` pair fixes its parameters and its output shape (§11.4).
**Rationale**: PRD 5.8 requires schema-checked ops and name-based wiring of their
results; without fixed output names, `writes:` and downstream guards would have
nothing stable to bind. *PRD 5.8.*

### D35. `scope:` is required on store definitions

**Rationale**: PRD 5.8 makes lifetime semantically load-bearing (session scope
requires a session-keyed trigger); defaulting it would hide a validation-relevant
choice. *PRD 5.8.*

### D36. `embed.model` is a bare provider-native id with an optional `provider:` ref

**Rationale**: the PRD writes `embed: { model: text-embedding-3-small }`;
embedding models are not chat models and do not belong in `model.*`, whose
capability checks are about structured output. *PRD 5.8, 5.9.*

### D37. `agent_access` narrows the synthesized store tool surface

**PRD-extending** — see this appendix's preamble.

Default `read_write`, narrowable to `read`. **Rationale**: PRD 5.8 synthesizes
`get`/`set` pairs, so `read_write` is the settled default; a declarative way to
withhold writes costs one enum and serves the same least-privilege posture as
placement isolation. **Status**: the PRD asks no least-privilege question about
store attachment, and this key changes which tools codegen synthesizes, so it is
new design surface rather than a shape choice — it needs PRD ratification before
M1 synthesizes those tools. Declining it removes one optional key and the `read`
column of §11.5. *PRD 5.8, 5.10.*

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

### D47. Placement entries take `runtime` plus a reserved `network:`

**Rationale**: PRD 5.10 describes isolation as covering sandbox, credentials, and
network policy; `network:` is the smallest reserved surface that records the
intent without pre-building the M3 feature. *PRD 5.10.*

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
keeps termination reasoning complete. **Status**: PRD 5.4 is about *graph*
cycles and says nothing about the tool loop, so both the bound and its default
are new design surface and need PRD ratification before M1 emits the loop.
Declining it removes one optional key; the loop then relies on whatever bound the
runtime imposes. *PRD 5.4, 5.5.*

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
binding. **Rationale**: PRD 5.7's default wiring is name-based, and `writes:`
already covers renames — a second output-binding construct would be the deferred
data-edge feature under another name. *PRD 5.7.*

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
have to be reserved as a channel name to stay unambiguous, for no gain. *PRD
5.6, 5.7.*

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
computable (no cycle on the path) and defined explicitly where it is not.
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
yet. **Rationale**: §7.6.2 makes
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
session coherence (§11.3), sync interrupt-freedom (§13.3, §8.7), and recursion
(§7.5) all use it.
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

`local` is built in: no deploy file is required, `placements:` and
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
legal because `placements:` and `event_sources:` are not storage and are carried
into the IR under `local` exactly as under any other target, so nothing there is
inert. The asymmetry with `event_sources:` is the
active/reserved split of §15: `storage_backends:` configures something `local`
overrides *today*, while `event_sources:` is reserved grammar no v0 target
executes, so it is carried into the IR under every target rather than being dead
under one. Satisfying both binding checks vacuously under `local` is what keeps
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
it can carry a budget and still cannot serve as any rule's guarantee. *PRD 5.3,
5.4, G3.*

### D91. The single-string-property decode exception is a `tool.*`-surface rule

§6.1's "except when `output` declares exactly one string-typed property, in which
case the raw text binds to it" applies to `tool.*` `exec:`/`http:` bindings only.
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
node's decoding depend on how many of its *other* fields happen to be strings —
adding `id: {type: string}` beside a lone `status:` would silently stop the body
from being parsed. One rule per surface, each justified by what that surface can
name. *PRD 5.5.*

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
distances of one pair (§7.6.1, §7.6.2).
**Rationale**: §7.6.1 defined co-takeability pairwise and then §7.6.2 used it as
a unary predicate — "paths that leave `f` by a co-takeable edge" — leaving an
out-edge that is exclusive with *every* sibling undecided: in or out of `dist`?
The two readings disagree on real specs. Take `e₁ when: "n.output.f == 'x'"`,
`e₂ when: "size(state.q) > 0"` (co-takeable, so `f` is a fork), and
`e₃ else: true` (exclusive with both by §7.6.1 rule 1), with `e₃` reaching a
convergence `d` in one step and `e₁`'s path in two. Reading it as "belongs to
some co-takeable pair" excludes `e₃`, gives `dist(d) = {2}`, and compiles;
reading it as "any out-edge of a fork" gives `{1, 2}` and an unbalanced-
convergence error. Two validators, two languages.

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
and, for a `map` node, the source-item index of the instance it dispatches
(§9.4). Its two carriers are a detached `map` dispatch (§8.6 rule 7) and a store
write (§11.4).
**Rationale**: PRD 5.6 and 5.8 both write the derivation as
`execution_id + node + item_index` and 5.8 raises it to "a named cross-cutting
rule", so the *principle* is settled; what "node" denotes is not, and this
grammar makes three constructs under which a node id names several distinct
effects in one execution. A flow instantiated twice — `a: {flow: flow.ingest}`
and `b: {flow: flow.ingest}` — puts the same store node `save` at two sites with
no `item_index` at all; nested maps repeat the inner index across outer items, so
inner item 0 under outer item 0 and inner item 0 under outer item 1 collide; and
a node inside a bounded cycle (§7.4) executes twice in one instance. Under
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
  unreachable nodes, undefined channels;
- the graph analyses of §7.6, §7.7, and §7.8, which need the whole flow graph
  rather than a key-and-value pair: balanced convergence (§7.6.2), the
  no-dead-end rules of §7.6.3 apart from the `start` edge below, component
  reachability (§7.7) and the three checks over it, node reachability from
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
a literal in the same object: store-op parameter sets per `op` (§11.4), provider
key sets per `kind` — both halves, the required keys and the closed row the
optional ones live in (§12.1, D106) — trigger
keys per `type` (§13) including the `respond`/`timeout` and `respond`/`callback`
pairings (§13.3), the map form rules and the `on_item_error` shape (§8.6) —
including the confinement of `input:`/`writes:`/`detach:` to the homogeneous form
(rule 7, D85) and the absence of any `context:` key, which is a `flow:` node's
alone because a dispatch's history isolation is unconditional (rule 13, D105) —
the field-map-only `input:` on the node kinds that name their
fields (§8.0, D88), the non-empty `expect_exit`/`expect_status` lists (§6.1), the
direct-XOR-route split on model definitions (§12.2), the `human` timeout/route
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
`id:`, `embed.model:`, a trigger's `path:`/`cron:`/`timezone:`, and a
`blob put`'s `content_type:` — §4.3 class 3, D92). The validator owns the rest of
class 3: CEL surfaces need the expression grammar, and descriptions and schema
literals would need the same `not` repeated on dozens of properties, which the
one-directional invariant does not require — a file the schema lets through is
still rejected by `validate`.

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
it (§14.3), and a trigger `path:` is only required to start with `/` and carry no
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
  tools: [tool.<t> | flow.<f>]
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
  outputs: <field map>              # required
  nodes: { <id>: <node> }
  edges: [ { from, to, when?, else?, max_iterations? } ]
          # else: true needs a when:-guarded sibling (D107);
          # max_iterations needs a when: on its own edge (D90)

store.<name>:
  kind: kv|vector|blob              # required
  scope: execution|session|global   # required
  value_schema|metadata_schema: <field map>
  embed: { model, provider?, dimensions? }   # vector
  backend: <alias>
  agent_access: read|read_write

provider.<name>: { kind: ..., api_key: "${ENV}", base_url: "${ENV}", ... }
                 # keys beyond kind/description are per kind — 12.1's row is
                 # closed, and another kind's key is an error (D106)
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
placements:       { <address>: { runtime: isolated|colocated, network? } }
storage_backends: { defaults: { kv|vector|blob: {...} }, aliases: { <alias>: {...} } }
event_sources:    { <name>: { kind: ..., ... } }
```

