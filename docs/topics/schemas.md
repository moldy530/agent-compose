# schemas

Schemas are the load-bearing construct. They are what makes routing decidable,
fan-out bounded, edges serializable, and store ops checkable — and what a model
is constrained by on the wire, so an agent's answer is parsed with the same
document it was constrained by.

The type language is a closed subset of JSON Schema in a compact shorthand.
Anything outside it (`anyOf`, `allOf`, `$ref`, `patternProperties`, `if`/`then`,
`not`) is a compile error.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.triage:
  model: model.m
  prompt: Read the report and classify it.
  input:
    report:     { type: string, min_length: 1 }
    reported_at: { type: string, format: date-time }
  output:
    severity:   { enum: [low, high, critical] }
    confidence: { type: number, minimum: 0, maximum: 1 }
    reporter:
      type: object
      properties:
        name:  { type: string }
        email: { type: string, format: email }
      optional: [email]
    findings:
      type: array
      max_items: 20
      items:
        discriminator: kind
        variants:
          auto_fixable: { file: { type: string }, patch_hint: { type: string } }
          needs_human:  { summary: { type: string } }

flow.triage:
  inputs:
    report: { type: string, min_length: 1 }
  outputs: {}
  nodes:
    classify:
      agent: agent.triage
      input: { report: "input.report", reported_at: "'2026-01-01T00:00:00Z'" }
  edges:
    - { from: start, to: classify }
    - { from: classify, to: end }
```

## Field maps

Every declaration surface — `output:`, `input:`, `inputs:`, `outputs:`,
`state:`, `value_schema:`, `metadata_schema:` — takes a **field map**: field name
to type node. A field map denotes a **closed object**: exactly the declared
properties and no others. `{}` is an object with no properties.

Field maps appear only at declaration surfaces. A nested mapping is not
shorthand for an object; write `type: object` out (below).

## Type nodes

A type node carries exactly one discriminating key — `type`, `enum`, or
`discriminator` — plus an optional `description` (which reaches the model in
structured-output schemas, so it is worth writing).

```yaml
title:      { type: string, min_length: 1, max_length: 200 }
confidence: { type: number, minimum: 0, maximum: 1 }
attempts:   { type: integer, minimum: 0 }
urgent:     { type: boolean }
verdict:    { enum: [approve, revise, escalate] }
started_at: { type: string, format: date-time }
```

| Key | Applies to | Notes |
|---|---|---|
| `min_length` / `max_length` | string | integer ≥ 0 |
| `pattern` | string | **RE2** — no backreferences, no lookaround |
| `format` | string | `date-time` `date` `time` `duration` `email` `uri` `uuid` `hostname` `ipv4` `ipv6` |
| `minimum` / `maximum` | integer, number | inclusive |
| `exclusive_minimum` / `exclusive_maximum` | integer, number | |
| `multiple_of` | integer, number | > 0 |

`enum` takes a non-empty array of unique strings and implies `type: string`;
writing `type:` beside `enum:` is an error. **Enum-typed output fields are what
routing exhaustiveness is computed over** — see `agent-compose docs routing`.

### Objects

```yaml
author:
  type: object
  properties:
    name:  { type: string }
    email: { type: string, format: email }
  optional: [email]
```

`properties` is required (may be `{}`). Every declared property is **required
unless listed in `optional:`**, and every name in `optional:` must be a property
the same object declares. Objects are closed — there is no
`additional_properties`. Nesting is limited to 8 levels.

Reading a property a value legitimately omits **fails the execution**; ask first
with CEL's `has()`.

### Arrays

```yaml
tasks:
  type: array
  max_items: 20
  min_items: 1
  unique_items: true
  items: { type: string }
```

`items` is required. `max_items` is an integer in `1..=10000`.

### Discriminated unions

```yaml
items:
  discriminator: kind
  variants:
    auto_fixable: { file: { type: string } }
    needs_human:  { summary: { type: string } }
```

At least two variants. A variant's field map must **not** declare the
discriminator field — the compiler synthesizes it as a string constant equal to
the tag, so instances carry `kind: auto_fixable`. A union is legal as an array's
`items:` and as a property's type, never as the top level of a declaration
surface. `map.route_by` consumes them (`agent-compose docs maps`).

## Result surfaces versus input surfaces

This split decides two rules, and it is the one thing to remember about schemas.

**Result surfaces** — `agent.output`, `tool.output`, `flow.outputs`,
`human.output`, an inline `exec:`/`http:` node's `output`, `store.value_schema`,
`store.metadata_schema`:

- **`max_items` is REQUIRED** on every array inside them. Model-produced
  cardinality must be bounded, and the bound keeps every edge payload finite.
- **`default:` is ILLEGAL.** A defaulted result would manufacture a routing
  value the model never emitted.

**Input surfaces** — `agent.input`, `tool.input`, `flow.inputs`, `human.input`,
and `state` channels — do the opposite on both counts: `max_items` is optional,
and `default:` is legal on scalars, enums, objects, and arrays (never on a
union, which would have to name a variant). A property with a `default:` is
implicitly optional at its surface.

`max_items` is also required on the array a `map.over` resolves to, wherever it
was declared: unbounded fan-out is a compile error.

## Emptiness

`{}` is legal at every surface except the two agent ones. `agent.output` must
declare at least one property, because it is what routing reads; `agent.input:
{}` is an error because *omitting* `input:` is how you say "no declared input"
(the string-in default — `agent-compose docs agents`). A result surface with no
fields is a real contract: a tool whose effect is its whole purpose.

Normative source: `docs/grammar.md` §3, §3.1–3.9
