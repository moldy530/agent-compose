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
deploy/local.yml    # deploy target (not imported)
deploy/staging.yml
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
  and nothing MAY target it. `start` is legal **only** as an edge `from`.
- `end` — the flow's exit. No edge MAY leave it. Reaching `end` terminates the
  flow instance and materializes its `outputs` (§7.5). `end` is legal as an edge
  `to` and in the two **control-transfer positions** — `on_error.fallback`
  (§9.2) and `human.on_timeout` (§8.7) — where it means "finish this flow
  instance now, materializing whatever the `outputs:` channels currently hold".
  The two control-transfer positions accept exactly the same targets; nothing
  else accepts a pseudo-node.

A node MUST NOT be named `start` or `end`.

### 2.5 Reserved names

The following identifiers are reserved and MUST NOT be used as **state channel
names** (they are CEL root identifiers or implicit channels — §4.1, §10.4):

`input`, `state`, `execution`, `item`, `messages`, `output`, `payload`

`start` and `end` are reserved as node ids only. Definition names have no
reserved words beyond the identifier grammar.

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
| `default` | scalar, enum | matching literal | input surfaces and state channels only (§3.6) |

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
- Objects are **closed**. There is no `additional_properties` knob; unknown keys
  in an instance are invalid (Decision [D8](#d8-objects-are-closed)).
- Nesting depth is limited to **8** levels (declaration surface counts as 1).

#### 3.5 Arrays

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

**`max_items` is REQUIRED** when the array (Decision [D10](#d10-max_items-is-required-on-result-schemas-and-fanned-out-arrays)):

1. appears anywhere inside a **result schema** — `agent.output`, `tool.output`,
   `flow.outputs`, `human.output`, `store.value_schema`,
   `store.metadata_schema`. Model-produced cardinality must be bounded, and
   structured-output validation rejects longer arrays so the model *cannot*
   return more (PRD 5.6); the same bound keeps every edge payload finite and
   serializable (PRD 5.7, 5.10); or
2. is the schema a `map.over` path resolves to — unbounded fan-out is a compile
   error (PRD 5.6).

It is OPTIONAL in **input schemas** (`agent.input`, `tool.input`, `flow.inputs`,
`human.input`) and on state channels, where the value is not model-produced and
its bound comes from whatever produced it.

#### 3.6 `default`, requiredness, and surface rules

- `default:` is legal on scalar and enum type nodes at **input** surfaces
  (`agent.input`, `flow.inputs`, `tool.input`, `human.input`) and on **state**
  channels, where it is the channel's initial value.
- `default:` is ILLEGAL in **output** schemas (`agent.output`, `tool.output`,
  `flow.outputs`, `human.output`) and in `store.value_schema` /
  `metadata_schema`. A defaulted model output would silently manufacture routing
  values (PRD 5.3).
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

| Surface | Kind | Required |
|---|---|---|
| `agent.<a>.output` | field map | REQUIRED (PRD 5.2) |
| `agent.<a>.input` | field map | optional; default string-in (§5.3) |
| `tool.<t>.input` | field map | REQUIRED (may be `{}`) |
| `tool.<t>.output` | field map | REQUIRED |
| `flow.<f>.inputs` | field map | optional (default: no inputs) |
| `flow.<f>.outputs` | field map | REQUIRED |
| `human.input` / `human.output` | field map | REQUIRED |
| inline `exec:` / `http:` node `output` | field map | optional (kind default, §8.2/§8.3) |
| `state` channels | field map + `reduce` | optional section |
| `store.<s>.value_schema` / `metadata_schema` | field map | per kind (§11) |

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
| `map.over` | `<node>.output` for any node that precedes the map node, `input`, `state` | list (path expression only, §4.2) |
| `map.input` / `map.routes.<tag>.input` values | `<as-name>` (the item), `input`, `state`, `execution` | field-typed |
| node `input:` bindings | `input`, `state`, `execution` | field-typed |
| store-op `key`, `value`, `query`, `prefix`, `filter`, `metadata` values | `input`, `state`, `execution` | per §11.4 |
| inline `http` node `query` / `body` values | `input`, `state`, `execution` | field-typed |
| trigger `input:` values | `payload` | field-typed |
| trigger `session_key`, `callback`, `dedupe_key` | `payload` | string |

Root identifier meanings:

- **`input`** — the enclosing flow instance's input object (`input.goal`).
- **`state`** — the state object; only declared channels (§10) are members.
- **`execution`** — run metadata: `execution.id` (string), `execution.session_key`
  (string, empty when no session-keyed trigger), `execution.item_index` (integer,
  present only inside a `map`-dispatched instance).
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
- `$${` is an escape producing a literal `${`.

**Secret-bearing fields take the env-ref value form only** — a literal is a
compile error (Decision [D41](#d41-env-ref-forms-and-the-secret-field-list)):

| Field | Where |
|---|---|
| `api_key`, `api_secret`, `token`, `password`, `access_key_id`, `secret_access_key`, `session_token`, `credentials_json` | `provider.*`, `storage_backends.*`, `event_sources.*` |
| `url`, `base_url`, `endpoint`, `dsn` | `provider.*`, `storage_backends.*`, `event_sources.*` |

Interpolated refs are legal in: `http` node/tool `url` and `headers` values,
`exec` `env` values and `cwd`, and non-secret deploy config values.
Env refs are ILLEGAL in: prompts, descriptions, schemas, CEL expressions, model
`id`, and any identifier or reference position.

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
| `output` | field map (result surface, §3.6) | **yes** | — | PRD 5.2; MUST have ≥ 1 property |
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
| `output` | field map (result surface, §3.6) | **yes** | result schema; makes edges serializable (PRD 5.7) |
| `exec` \| `http` \| `function` | block | **exactly one** | implementation binding |

### 6.1 Implementation bindings

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
| `command` | string | yes | executable name/path; never shell-interpreted |
| `args` | array of string | no | literal; no CEL (D24) |
| `cwd` | string (interpolable) | no | |
| `env` | map env-var-name (`[A-Za-z_][A-Za-z0-9_]*`) → string (interpolable) | no | added to the child environment |

Input/output convention (PRD 5.5): an object input is passed as environment
variables (`UPPER_SNAKE_CASE` of each field, JSON-encoded for non-scalars); a
string input is passed on stdin. The child's **stdout** is decoded as JSON and
validated against `output`, except when `output` declares exactly one
string-typed property, in which case trimmed raw stdout binds to it. A non-zero
exit status is a node error subject to §9.

A `tool.*` declares a *domain* result schema, so every one of its fields is
decoded as above — `exit_code`/`stdout` are not special here. Inline `exec:`
nodes are the surface that exposes the process envelope; see §8.2.

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
| `query` | map param-name (`[A-Za-z0-9_-]+`) → CEL | no | |
| `body` | map identifier→CEL | no | JSON body; illegal for `GET`/`HEAD` |
| `expect_status` | array of integer | no | default: any 2xx |

Without `body`/`query`, the bound input object is sent as the JSON body
(body-bearing methods) or as query parameters (`GET`/`HEAD`). The response body
is decoded as JSON and validated against `output`, except when `output` declares
exactly one string-typed property, in which case the raw response text binds to
it. A status outside `expect_status` is a node error subject to §9. As with
`exec`, this is the *tool* surface: the response envelope (`status`, raw `body`)
is exposed by inline `http:` nodes only (§8.3).

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
    - { from: review, to: end,   when: "review.output.verdict == 'approve'" }
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `nodes` | map node-id→node | **yes** | ≥ 1 node |
| `edges` | array of edge | **yes** | ≥ 1 edge |
| `outputs` | field map (result surface, §3.6) | **yes** | the module's result surface (PRD 5.1) |
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
| `input` | map field→CEL, or scalar CEL | input bindings; §8.0 |
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
```

| Key | Type | Required | Notes |
|---|---|---|---|
| `from` | node id \| `start` | yes | |
| `to` | node id \| `end` | yes | |
| `when` | CEL (bool) | no | guard over the source node's output (§4.1) |
| `else` | `true` | no | marks the default edge; mutually exclusive with `when`. `true` is the only legal value — `else: false` says nothing (an unguarded edge is already unconditional) and is a compile error |
| `max_iterations` | integer 1..1000 | no | cycle bound (PRD 5.4) |

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
   declared `reduce` policy (§10.2) — the same rule maps obey (PRD 5.6).
7. If no edge is taken, the execution fails with a "no viable route" error naming
   the node. The validator rejects statically-provable instances of this
   (exhaustiveness, §7.3.1; escape edges, §7.4).

**7.3.1 Exhaustiveness.** When a node's outgoing edges are guarded by equality
against an enum-typed field of its own output, every enum variant MUST be
routable: some guarded edge can be true for it, or an `else:` edge exists.
Otherwise it is a compile error naming the unroutable variants (PRD 5.3). This is
why `enum` is string-only and closed (§3.3).

### 7.4 Cycles and termination

Back-edges are permitted (PRD 5.4). The compiler computes SCCs and requires:

- **Bounding**: every SCC with ≥ 1 edge MUST be *bounded*. An SCC is bounded when
  at least one of the following holds — PRD 5.4's "`max_iterations` and/or a CEL
  exit condition on at least one edge in the cycle", made decidable
  (Decision [D57](#d57-what-counts-as-a-cel-exit-condition)):

  1. **Counting bound** — some edge whose `from` *and* `to` are both in the SCC
     carries `max_iterations`; or
  2. **CEL exit condition** — some node `n` in the SCC has an outgoing edge that
     leaves the SCC and carries a `when:` guard, **and** every outgoing edge of
     `n` that stays inside the SCC carries a `when:` guard (or an `else:`).
     Both halves are required: an unguarded in-SCC edge from `n` is
     unconditional (§7.3 rule 2), so it would re-enter the loop no matter what
     the exit guard says, and the "exit condition" would never exit.

  An SCC satisfying neither is a compile error naming the SCC's nodes.

  A counting bound is a **static termination proof**; a CEL exit condition is
  not — its guard is a runtime value, so a model that never emits the exit value
  keeps looping. PRD 5.4 accepts both, so the validator does too; only clause 1
  makes the loop provably finite, which is why the examples in this document use
  it.
- **Escape**: the source node of each `max_iterations`-carrying edge MUST have at
  least one outgoing edge that leaves the SCC, so exhausting the budget cannot
  dead-end the execution (Decision [D19](#d19-max_iterations-semantics-and-the-escape-edge-rule)).
  Clause 2 already requires such an edge by construction.

`max_iterations` counts **traversals of that edge within one flow instance**.
Instances of the same flow (including `map`-dispatched ones) count independently.
Codegen emits one counter per bounded edge into the graph state; iteration
boundaries are checkpoint/resume points.

### 7.5 Instantiation, inputs, and outputs

- **Inputs**: `flow.<f>.inputs` is the module's parameter surface. Inside the
  flow, `input.<field>` is in scope everywhere (§4.1).
- **Outputs**: on reaching `end`, each field of `outputs:` is read from the state
  channel of the same name; that channel MUST be declared in `state:` (§10) or
  it is a compile error. There is no `returns:` binding — use a node `writes:`
  remap to feed a differently-named channel
  (Decision [D53](#d53-flow-outputs-are-name-based-from-state)).
- **As a node**: `{ flow: flow.review_loop, input: {...} }` — §8.5. `input:` is
  REQUIRED whenever the subflow declares inputs: subgraphs receive parent state
  only through explicit bindings (PRD 5.7).
- **As a tool**: listing `flow.review_loop` in an agent's `tools:` makes its
  `inputs`/`outputs` the tool's parameter/result schemas. `description:` is then
  REQUIRED.
- **Recursion is forbidden**: a flow that reaches itself through `flow:` nodes or
  tool attachment is a compile error naming the cycle
  (Decision [D26](#d26-flow-defs-outputs-required-description-when-tool-no-recursion)).

---

## 8. Node types

### 8.0 Input bindings, name-based wiring, and `writes`

**Reading.** A node's input fields are resolved in this order
(Decision [D15](#d15-node-level-input-is-the-one-binding-mechanism)):

1. an explicit `input:` binding for that field, if present;
2. otherwise the state channel of the same name (§10);
3. otherwise the enclosing flow input of the same name;
4. otherwise a compile error naming the unbound field.

```yaml
review:
  agent: agent.reviewer
  input:                       # explicit bindings (CEL, §4.1)
    goal:  "input.goal"
    draft: "state.draft"
```

Explicit `input:` MUST bind a subset of the target's declared input fields;
unbound fields fall through to steps 2–4. For string-in agents the scalar form
`input: "input.goal"` is used (§5.3). On `flow:` nodes `input:` is REQUIRED when
the subflow declares inputs (PRD 5.7 `passVariables` discipline). On `map` nodes
the per-item binding lives inside the `map:` block instead (§8.6).

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
- Every write — name-based or remapped — is type-checked against the target
  channel by its reduce policy (§10.2): whole value for an unreduced or
  `last_wins` channel, one element for an `append` channel, a partial object for
  a `merge` channel.
- A remapped field is not also written to its same-named channel.
- Remapping is the fix for channel collisions between nodes and for renames
  across subgraph boundaries (PRD 5.7).
- Writing the same channel from concurrent contexts (parallel branches, `map`
  instances) requires a channel with a declared `reduce` policy (§10.2).

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
    output:
      exit_code: { type: integer }
      stdout:    { type: string }
  input: { pattern: "state.test_filter" }
  on_error: skip
```

`exec:` block keys: `command` (required), `args`, `cwd`, `env` as in §6.1, plus:

| Key | Type | Required | Default |
|---|---|---|---|
| `output` | field map | no | `{ exit_code: {type: integer}, stdout: {type: string} }` |

The node-level `input:` bindings produce the object passed to the child as
environment variables — binding keys are identifiers (§2.1) and are
upper-snake-cased on the way into the environment, so `pattern:` above arrives as
`PATTERN` — or on stdin for a scalar binding, per the §6.1 convention.

**Result binding.** An inline `exec:` node wraps a *process*, so its result is
the process envelope, not a decoded payload
(Decision [D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads)).
`exit_code` (`{type: integer}`), `stdout` (`{type: string}`) and `stderr`
(`{type: string}`) are **envelope fields**: declaring one in `output` binds it
from the child process directly, and it is never decoded from stdout. Declaring
an envelope name with any other type is a compile error. Every *other* declared
field is decoded from stdout as JSON per §6.1 (whose single-string-property
shortcut is computed over the decoded fields alone). When `output` declares only
envelope fields — as the default does — stdout is never parsed.

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
| `output` | field map | no | `{ status: {type: integer}, body: {type: string} }` |

**Result binding.** As with `exec:` (§8.2), an inline `http:` node's result is
the response envelope. `status` (`{type: integer}`) and `body` (`{type: string}`,
the raw response text) are **envelope fields**: declared, they bind from the
response directly and are never decoded from it, and declaring either with
another type is a compile error. Every other declared field is decoded from the
response body as JSON per §6.1. So a node declaring only `status:` records the
HTTP status and never parses the body (this is what the `escalate` node of
[`examples/triage-fanout`](../examples/triage-fanout/flows/triage.yml) does),
while a node that also wants the created ticket id declares
`{ status: {type: integer}, id: {type: string} }` and gets `id` from the decoded
body
(Decision [D56](#d56-inline-exechttp-node-results-are-envelopes-not-decoded-payloads)).

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
| `input` | map field→CEL | yes when the subflow has inputs | — | explicit bindings only |
| `writes` | map output-field→channel | no | name-based | keys are the subflow's `outputs` fields |
| `context` | `isolated` \| `inherit` | no | `isolated` | conversation-history scoping (PRD 5.7) |
| `policy` | `{ retry, timeout, on_error }` | no | — | override for the nodes *inside*, §9.3 |
| `retry` | block | no | — | policy for *this* node, §9.1 |
| `timeout` | duration | no | — | policy for *this* node, §9.2 |
| `on_error` | `fail` \| `skip` \| `{ fallback: … }` | no | — | policy for *this* node, §9.2 |

`context: inherit` shares the caller's conversation-history channel with the
subflow; `isolated` (the default) gives the subflow a fresh one. Nothing else
crosses a module boundary implicitly (PRD 5.7). `context:` is legal on `flow:`
nodes only.

**`policy:` and the node's own policy keys are different things**, and a `flow:`
node MAY carry both:

- `policy:` is level 1 of the resolution chain (§9.3) for **every node inside**
  the instantiated subflow, propagated into nested instantiations. It never
  applies to the instantiating node itself.
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
| `routes` | map tag→route | with `route_by` | — | keys MUST be variant tags |
| `default` | route | no | — | catch-all; legal only with `route_by` |
| `max_concurrency` | integer 1..256 | **yes** | — | node-wide bound (D28) |
| `on_item_error` | `fail` \| `skip` \| `retry` | no | `fail` | per item (PRD 5.6) |
| `input` | map field→CEL | no | whole item | per-item input binding |
| `writes` | map output-field→channel | no | name-based | target channels MUST be reduced |
| `detach` | boolean | no | `false` | fire-and-forget dispatch |

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
   target a channel with a declared `reduce` policy. Appended results are
   automatically index-tagged and reordered by source-item index before the join,
   so replay is deterministic.
6. **Join**: the downstream edge is the barrier — it fires when all instances
   have completed or been resolved by `on_item_error`. Sink routes are waited on
   like any other route.
7. **`detach`**: a detached route is fire-and-forget. A detached route MUST NOT
   declare `writes:` and MUST NOT write reduced state. In v0, `detach: true` is a
   validation error under any target whose execution state is durably
   checkpointed — every target except `local` (§14) — pointing at the roadmap
   (the outbox-pattern delivery is not v0 work). Checkpointing is a property of
   the target, not a spec construct, so this is a **target-dependent** check like
   backend alias resolution (§11.3): the same composition is legal under
   `--target local` and rejected under `--target staging`. Detached dispatches
   receive an `idempotency_key` derived from
   `execution_id + node + item_index`.
8. **`route_by` is a literal field name**, never a CEL expression — this keeps
   exhaustiveness decidable (PRD 5.6).
9. Node-level `input:` and node-level `writes:` are both ILLEGAL on a `map` node:
   a map node has no input or output of its own, only dispatched instances. The
   per-item binding lives in the `map:` block (or on a route), and so does the
   write remap. Node-level `retry`/`timeout`/`on_error` are legal and apply to
   the map node as a whole, while `on_item_error` governs individual items.

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
| `output` | field map (result surface, §3.6) | yes | routable structured output; resume payloads are validated against it (PRD 5.11) |
| `timeout` | duration | no | wall-clock wait budget |
| `on_timeout` | flow-local node id, or `end` | required with `timeout` | route taken on expiry; same targets as `on_error.fallback` (§2.4, §9.2) |

Node-level `timeout:` and `retry:` are ILLEGAL on a `human` node — a wait is not
an activity timeout and re-prompting a human is not a retry
(Decision [D52](#d52-human-node-shape)). `on_error:` remains legal (it covers
delivery failures). A flow reachable from a `respond: sync` http trigger MUST NOT
contain a reachable `human` node (PRD 5.11).

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

The op's output schema is derived from the store definition (§11.4) and is
written by name like any other node output.

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
| `fail` | abort the execution with the node's error (built-in default) |
| `skip` | the node produces no output and writes nothing; its **unconditional** and `else` outgoing edges still fire, and guards referencing the missing output evaluate to false |
| `{ fallback: <node id or end> }` | transfer control to another node in the same flow, or to `end`; never `start` |

`fallback` targets a **flow-local node id or `end`** (§2.4), keeping error
routing inside one graph where reachability analysis can see it — never `start`,
and never a node of another flow (Decision
[D21](#d21-on_error-strategies-and-fallback-targets)). `human.on_timeout` (§8.7)
accepts the same targets.

### 9.3 Resolution chain

For each policy field (`retry`, `timeout`, `on_error`) independently, highest
precedence first (PRD 5.5's `flow override > node > defaults > fail`):

1. **Flow override** — `policy:` on the `flow:` node that instantiated the
   enclosing flow, propagated into nested instantiations. It applies to the nodes
   *inside* that instance only; the instantiating `flow:` node resolves its own
   policy from levels 1–4 in its own flow (§8.5).
2. **Node** — the node's own `retry`/`timeout`/`on_error`.
3. **Defaults** — the composition's `defaults:` section.
4. **Built-in** — no retry, no timeout, `on_error: fail`.

```yaml
# main.yml
defaults:
  timeout: 60s
  retry: { max: 1, backoff: 1s }
  on_error: fail
```

`defaults:` accepts exactly `retry`, `timeout`, `on_error`, appears at most once
per composition (§1.5), and applies to every node in every flow
(Decision [D20](#d20-the-policy-resolution-chain-has-exactly-four-levels)).

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
  totals:
    type: object
    properties: { fixed: { type: integer }, skipped: { type: integer } }
    reduce: merge
```

### 10.1 Channels

A channel is a type node (§3.2) plus two channel-only keys:

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `reduce` | `append` \| `merge` \| `last_wins` | no | *unreduced* | concurrency policy |
| `default` | literal matching the type | no | — | initial value |

Channel names are identifiers and MUST NOT be reserved (§2.5). The channel set is
composition-global in **shape**; each flow instance holds its own **values**, so
two flows may both use `draft` without interfering, and a subgraph sees only what
its `input:` bindings and `writes:` remaps carry across (PRD 5.7).

### 10.2 Reduce policies

| Policy | Requires | Semantics |
|---|---|---|
| `append` | `type: array` | each write contributes **one element**; `map` writes are index-tagged and reordered by source-item index before the join (PRD 5.6) |
| `merge` | `type: object` | shallow key-wise merge of the written object into the channel; conflicting keys resolve last-writer-wins within one superstep |
| `last_wins` | any | last write in the superstep wins, explicitly declared as concurrency-safe |

A channel **without** `reduce:` is *unreduced*: single-writer, sequential. Writing
an unreduced channel from inside a `map` — or from two concurrent branches
(§7.3) — is a compile error (PRD 5.6). Declaring `reduce: last_wins` is how an
author opts into concurrent overwrite explicitly.

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
| `merge` | a partial object | an object whose properties are a subset of the channel's, with matching types |

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
| `value_schema` | field map (result surface, §3.6) | `kv` REQUIRED; `vector`/`blob` illegal | stored value shape |
| `metadata_schema` | field map (result surface, §3.6) | `vector`/`blob` optional; `kv` illegal | filterable metadata |
| `embed` | block | `vector` required; others illegal | §11.2 |
| `backend` | identifier (bare alias) | no | abstract slot; never provider config |
| `description` | string | no | LLM-facing for agent-attached stores |
| `agent_access` | `read` \| `read_write` | no (default `read_write`) | narrows the synthesized tool surface |

### 11.2 `embed` (vector only)

| Key | Type | Required | Notes |
|---|---|---|---|
| `model` | string | yes | provider-native embedding model id (a bare string, not a `model.*` ref — D36) |
| `provider` | `provider.*` ref | no | which connection serves it; default resolved from the target's backend |
| `dimensions` | integer ≥ 1 | no | asserted against the backend's index |

### 11.3 Backends and scope

- `backend:` names an **abstract alias** defined per target in
  `deploy/<target>.yml` → `storage_backends.aliases` (§14.2). Resolution order:
  explicit alias → per-kind `defaults:` → target built-in. `--target local`
  substitutes local storage for every store unconditionally (PRD 5.8).
- An alias referenced by a store but undefined in the active target is a compile
  error naming the target.
- `scope: session` requires the execution to have a session identity: using a
  session-scoped store in a flow whose triggers declare no `session_key:` is a
  compile error (PRD 5.8, 5.11).

### 11.4 Store-op nodes

Legal ops per kind, with their parameters and derived output schema. `V` is the
store's `value_schema` object; `M` its `metadata_schema` object.

| kind | `op` | Parameters | Output |
|---|---|---|---|
| `kv` | `get` | `key` (CEL string) | `{ value: V (optional), found: boolean }` |
| `kv` | `set` | `key`, `value` (map field→CEL matching `V`) | `{ key: string }` |
| `kv` | `delete` | `key` | `{ deleted: boolean }` |
| `kv` | `list` | `prefix` (CEL string), `limit` (integer 1..1000, required) | `{ keys: array<string> }` |
| `vector` | `search` | `query` (CEL string), `top_k` (integer 1..100, required), `filter` (map metadata-field→CEL, optional) | `{ matches: array<{ id: string, score: number, text: string, metadata: M }> }` |
| `vector` | `upsert` | `key`, `value` (CEL string — the text), `metadata` (map field→CEL, optional) | `{ id: string }` |
| `vector` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `put` | `key`, `value` (CEL string), `content_type` (string, optional) | `{ key: string }` |
| `blob` | `get` | `key` | `{ value: string (optional), found: boolean }` |
| `blob` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `list` | `prefix`, `limit` (required) | `{ keys: array<string> }` |

Rules (PRD 5.8):

- `value` on a `kv` `set` is schema-checked against `value_schema`; `filter` and
  `metadata` keys are schema-checked against `metadata_schema`.
- Store ops are **effects**: reads are recorded and replay consumes history, not
  the live store; writes are at-least-once carrying an idempotency key derived
  from `execution_id + node + item_index`.
- A store **write** performed inside a `map`-dispatched instance MUST derive its
  key from the item: the `key:` expression MUST reference `input.*` (the item, as
  bound into the dispatched flow) or `execution.item_index`. Unkeyed blob or
  global writes from concurrent instances are a compile error.

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

| Key | Type | Required | Notes |
|---|---|---|---|
| `kind` | enum (below) | yes | selects the provider plugin and its config schema |
| `api_key` | env-ref value | per kind | never a literal (§4.3) |
| `base_url` | env-ref value | required for `openai_compatible` | |
| `headers` | map name→string (interpolable) | no | extra request headers |
| kind-specific keys | per plugin | per kind | validated against the plugin's published schema |

v0 provider kinds and their kind-specific keys:

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
| `id` | string | yes | provider-native model id; no env refs |
| `settings` | object | no | validated against the provider plugin's settings schema |
| `description` | string | no | |

**Route form:**

| Key | Type | Required | Notes |
|---|---|---|---|
| `route` | array of `model.*` refs, ≥ 2 | yes | ordered fallback; members MUST be direct models (no nested routes) |
| `route_on` | array of enum | no (default `[rate_limit, overloaded, timeout]`) | `rate_limit`, `overloaded`, `timeout`, `server_error` |
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

Triggers define what causes an execution to exist; the entrypoints of a project
are exactly the flows its triggers point at (PRD 5.11). There is no `entrypoint:`
key.

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
| `input` | map flow-input-field→CEL over `payload` | per type | compile-checked against the flow's `inputs` |
| `session_key` | CEL over `payload` → string | no | supplies session identity for session-scoped stores and history (PRD 5.8) |
| `description` | string | no | |

Every REQUIRED field of the target flow's `inputs` MUST be bound by `input:`
(directly or by a schema default); a binding that can produce a value the flow
cannot accept is a compile error (PRD 5.11).

### 13.2 `manual` (active in v0)

```yaml
cli:
  type: manual
  flow: flow.review_loop
```

CLI/SDK invocation: `agent-compose run flow.review_loop --input goal=...`. A
manual trigger MUST NOT declare `input:` — CLI arguments are validated directly
against the flow's input schema
(Decision [D44](#d44-manual-triggers-carry-no-input-bindings)). `session_key:` is
legal (the CLI supplies `--session <key>`, exposed as `payload.session`).

### 13.3 `http` (active in v0)

| Key | Type | Required | Default | Notes |
|---|---|---|---|---|
| `path` | string starting `/` | no | `/triggers/<name>` | route of the generated app |
| `method` | `POST` \| `PUT` \| `GET` | no | `POST` | |
| `input` | map field→CEL over `payload` | no | — | |
| `respond` | `sync` \| `async` | no | `async` | |
| `timeout` | duration | no | `60s` | meaningful for `respond: sync` |
| `callback` | CEL over `payload` → string | no | — | completion webhook; `async` only |

`payload` shape: `payload.body` (decoded JSON object), `payload.query` (map of
string), `payload.headers` (map of string, lowercase names), `payload.path`
(string), `payload.method` (string).

- `respond: async` returns an execution id immediately; the optional `callback:`
  webhook fires on completion.
- `respond: sync` blocks and returns the flow's outputs. A flow exposed
  synchronously MUST be statically **interrupt-free**: no `human` node reachable
  from its entry (PRD 5.11). On timeout expiry the response **upgrades to async**
  (HTTP 202 + execution id + status URL); the execution continues durably.
- `callback:` with `respond: sync` is a compile error.
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
| `cron` | string, 5-field POSIX cron | yes | validated for shape at compile time |
| `timezone` | IANA tz name | no (default `UTC`) | |
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
error naming the target.

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

```yaml
# deploy/staging.yml
version: "0.1"

placements:
  agent.researcher: { runtime: isolated, network: egress }
  flow.review_loop: { runtime: colocated }

storage_backends:
  defaults:
    kv: { provider: redis, url: ${REDIS_URL} }
  aliases:
    docs_db: { provider: chroma, url: ${CHROMA_URL} }

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

Required inside every result schema (`agent.output`, `tool.output`,
`flow.outputs`, `human.output`, store schemas) and on any array a `map.over`
resolves to; optional in input schemas and state channels. **Rationale**: PRD 5.6
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
`input:`; on `flow:` nodes it is required whenever the subflow has inputs.
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
Concurrent branches obey the same reduced-channel rule as maps. *PRD 5.3, 5.6.*

### D18. Exhaustiveness is computed over enum-typed output fields

A node whose outgoing guards compare an enum field of its own output must cover
every variant or declare `else:`. **Rationale**: this is the decidable core of
PRD 5.3's promise; guards over non-enum values are unconstrained and simply
require an `else:` or an unconditional edge to avoid a dead end. *PRD 5.3.*

### D19. `max_iterations` semantics and the escape-edge rule

The budget counts traversals of one edge within one flow instance; an exhausted
edge is untraversable; the source node of a bounded edge MUST have an outgoing
edge that leaves the SCC. **Rationale**: PRD 5.4 requires bounded cycles, but a
bound with no escape merely converts an infinite loop into a runtime dead end.
Per-instance counting keeps `map`-dispatched subgraph instances independent.
*PRD 5.4, 5.6.*

### D20. The policy resolution chain has exactly four levels

Flow-node `policy:` override > node > `defaults:` > built-in `fail`.
**Rationale**: PRD 5.5 names exactly these four; the override is placed at the
*instantiation site* so a caller can harden a reused module — the only reading
under which "flow override" beating a node's own declaration makes sense. A
flow-definition-level default layer was considered and rejected to keep the chain
as settled. *PRD 5.5.*

### D21. `on_error` strategies and `fallback` targets

`fail` | `skip` | `{ fallback: <flow-local node id or end> }`, applied after
retries; `skip` suppresses writes and lets unconditional/`else` edges fire.
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
an agent's `tools:`; a flow reaching itself is an error. **Rationale**: PRD 5.1
makes a flow's I/O surface interchangeable with a tool's, which requires a
declared output; recursion has no termination proof analogous to the SCC rule and
would break the fan-out bounding guarantees. *PRD 5.1, 5.4.*

### D27. `context: inherit` is a `flow:`-node key only, default `isolated`

**Rationale**: PRD 5.7 settles isolation by default with opt-in inheritance at
the instantiation site; putting it on the definition would make a flow's
reusability depend on its own declaration rather than its caller. *PRD 5.7.*

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
*PRD 5.6, 5.10.*

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

Legal on routes and on the homogeneous form; a detached dispatch MUST NOT declare
`writes:` or write reduced state, and `detach: true` under a durably checkpointed
target is a v0 validation error (D59 fixes which targets those are).
**Rationale**: verbatim from PRD 5.6's settled position on detach under durable
execution; idempotency keys are supplied automatically. *PRD 5.6.*

### D32. Reduce policies are typed and `last_wins` is explicit

`append` requires an array channel, `merge` an object channel; an unreduced
channel written from a concurrent context is a compile error. **Rationale**: PRD
5.6 requires a *declared* reduce policy for concurrent writes; making `last_wins`
something an author writes down turns silent overwrite into a reviewed decision.
*PRD 5.6, 5.7.*

### D33. Reserved channel names

`input`, `state`, `execution`, `item`, `messages`, `output`, `payload` may not be
channel names. **Rationale**: they are CEL roots or the implicit history channel;
shadowing them would make expressions ambiguous. *PRD 5.5, 5.7.*

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

Default `read_write`, narrowable to `read`. **Rationale**: PRD 5.8 synthesizes
`get`/`set` pairs, so `read_write` is the settled default; a declarative way to
withhold writes costs one enum and serves the same least-privilege posture as
placement isolation. *PRD 5.8, 5.10.*

### D38. Provider kinds are a closed v0 set with per-kind required keys

`anthropic`, `openai`, `openai_compatible`, `azure_openai`, `bedrock`, `vertex`.
**Rationale**: PRD 5.9 describes provider plugins publishing config schemas; v0
ships the blessed set so the editor schema and the validator agree. New kinds
arrive with new plugins, additively. *PRD 5.9.*

### D39. Model defs are direct XOR route

No nested routes; route members are direct models; `route_on` defaults to
`[rate_limit, overloaded, timeout]` and adds `server_error`. **Rationale**: PRD
5.9's two forms, kept disjoint so failover order is a flat, traceable list;
nesting would make "served by fallback #1" ambiguous. *PRD 5.9.*

### D40. `settings:` is the only open object in the logical layer

**Rationale**: PRD 5.9 has provider plugins publish settings schemas the compiler
checks against; the editor schema cannot know them, so it types the common keys
and permits the rest. Everywhere else, unknown keys are errors. *PRD 5.9.*

### D41. Env-ref forms and the secret-field list

Value form (`^\$\{[A-Z_][A-Z0-9_]*\}$`) is mandatory for the credential and
connection fields listed in §4.3; interpolation is allowed in URLs, headers, and
exec env values; `$${` escapes. **Rationale**: PRD 5.9 forbids literals for
secrets and keeps refs unresolved in the IR; interpolation is still needed for
host-templated URLs, so the two forms are separated by field rather than banned
outright. *PRD 5.8, 5.9.*

### D42. Node outputs are readable only from edge guards and `map.over`

Node configuration CEL sees `input`, `state`, `execution` (and item bindings
inside a map). **Rationale**: PRD 5.3 scopes guards to the source node's output,
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
supply the same values. `session_key:` remains legal so CLI runs can join a
session. *PRD 5.11.*

### D45. `http` trigger defaults: path, method, `respond: async`, `timeout: 60s`

**Rationale**: PRD 5.11 settles async as the default and says sync "requires a
timeout (default 60s)" — reconciled as: the effective timeout always exists,
defaulting to 60s, and drives the documented async upgrade. Deriving the default
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

**Rationale**: PRD 5.4 bounds graph cycles statically; the intra-agent tool loop
is the one remaining unbounded loop in a compiled graph, and a declarative bound
keeps termination reasoning complete. *PRD 5.4, 5.5.*

### D52. `human` node shape

Schemas, `timeout`, and `on_timeout` live inside the `human:` block;
`on_timeout` is required with `timeout`; node-level `timeout`/`retry` are
illegal; `on_timeout` accepts a flow-local node id or `end`, exactly as
`on_error.fallback` does (D21). **Rationale**: PRD 5.5 gives the human node both
schemas plus timeout and route; separating the human wait from an activity
timeout prevents two keys named `timeout` meaning different things on one node.
Two control-transfer positions with different target sets would be a trap with no
rationale behind it. *PRD 5.5, 5.11.*

### D53. Flow outputs are name-based from state

Each `outputs:` field reads the channel of the same name; there is no `returns:`
binding. **Rationale**: PRD 5.7's default wiring is name-based, and `writes:`
already covers renames — a second output-binding construct would be the deferred
data-edge feature under another name. *PRD 5.7.*

### D54. `description` is required only where it is machine-consumed

Required on `tool.*` and on flows used as tools (LLM-facing); optional
everywhere else. **Rationale**: PRD 5.5 makes the description part of the tool
contract; forcing it on every definition would be documentation policy, not
grammar. *PRD 5.5.*

### D55. Definition order is irrelevant; IR order is canonical

Files and definitions may appear in any order; the resolver emits definitions
sorted by address and preserves author order only where it is semantic (edge
declaration order, `route:` lists, `imports:`). **Rationale**: PRD 5.12 requires
deterministic, byte-identical output for the same input. *PRD 5.12.*

### D56. Inline `exec`/`http` node results are envelopes, not decoded payloads

On an inline `exec:` node, `exit_code`/`stdout`/`stderr` are synthesized from the
child process; on an inline `http:` node, `status`/`body` are synthesized from the
response. They are never decoded from the payload, their types are fixed, and any
*other* declared field is decoded per §6.1. Envelope names carry no special
meaning in a `tool.*` result schema. **Rationale**: §6.1's decoding rule and the
kind defaults (`{exit_code, stdout}`, `{status, body}`) are otherwise in direct
contradiction — `npm test` would have to print `{"exit_code":0,…}` for the
*default* schema to work. The split follows the surfaces' purposes: a `tool.*`
declares a domain result, while an inline node is a one-off wrapper whose
interesting result is the process/response envelope. Fixing the envelope types
keeps the rule decidable per node and lets codegen emit the binding without
inference. *PRD 5.5.*

### D57. What counts as a CEL exit condition

An SCC is bounded by (1) an in-SCC edge carrying `max_iterations`, or (2) a node
in the SCC with a guarded edge leaving it *and* only guarded edges staying in it.
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

`else: false` is a compile error. **Rationale**: §7.3 gives meaning only to
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
- context-sensitive schema rules such as "`max_items` is required inside an
  agent output" — which the schema *does* enforce where the surface is
  syntactically identifiable (agent `output`, `map` sources are checked by the
  validator).

A file that passes the schema and fails `validate` is normal and expected; a file
that fails the schema always fails `validate`.

---

## Appendix C — Construct reference card

```yaml
# ---- spec file --------------------------------------------------------------
version: "0.1"                      # entrypoint + deploy files
imports: [ "<relative path>", ... ] # entrypoint only
defaults: { retry: {...}, timeout: <dur>, on_error: <policy> }
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

store.<name>:
  kind: kv|vector|blob              # required
  scope: execution|session|global   # required
  value_schema|metadata_schema: <field map>
  embed: { model, provider?, dimensions? }   # vector
  backend: <alias>
  agent_access: read|read_write

provider.<name>: { kind: ..., api_key: ${ENV}, base_url: ${ENV}, ... }
model.<name>:    { provider: provider.<p>, id: <string>, settings: {...} }
model.<name>:    { route: [model.<a>, model.<b>], route_on: [...] }

# ---- node shapes ------------------------------------------------------------
{ agent: agent.<a>,  input: <bindings>, writes: {...}, retry/timeout/on_error }
{ function: tool.<t>, input: <bindings>, writes: {...} }
{ flow: flow.<f>,    input: <bindings>, writes: {...}, context: isolated|inherit,
                     policy: {...},                    # for the nodes inside
                     retry/timeout/on_error }          # for the instance itself
{ exec: { command, args?, cwd?, env?, output? },  input: <bindings> }
{ http: { method, url, headers?, query?, body?, expect_status?, output? } }
{ human: { input, output, timeout?, on_timeout? }, input: <bindings> }
{ store: store.<s>, op: <op>, key?/value?/query?/top_k?/prefix?/limit?/filter? }
{ map: { over, as?, node | (route_by + routes + default?),
         max_concurrency, on_item_error?, input?, writes?, detach? } }

# ---- deploy file ------------------------------------------------------------
version: "0.1"
placements:       { <address>: { runtime: isolated|colocated, network? } }
storage_backends: { defaults: { kv|vector|blob: {...} }, aliases: { <alias>: {...} } }
event_sources:    { <name>: { kind: ..., ... } }
```

