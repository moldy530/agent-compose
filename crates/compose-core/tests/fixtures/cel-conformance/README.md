# The CEL conformance corpus

Shared fixtures for the two CEL implementations this project ships against: the
Rust evaluator the compiler links (`cel`, pinned in
`crates/compose-core/Cargo.toml`) and the JS evaluator M1 embeds in generated
routers. CLAUDE.md's validation strategy requires both to be run against these
files, with divergence failing CI; `crates/compose-core/tests/cel_conformance.rs`
is the Rust runner.

## Format

Every `*.json` file is an array of cases. Nothing but data lives here, so a
runner in another language needs no port of anything.

```json
{
  "name": "size-of-a-list",
  "expression": "size(state.items)",
  "input": { "state": { "items": ["a"] } },
  "result": 1
}
```

- `name` — unique across the whole corpus; it is what a divergence is reported
  by.
- `expression` — CEL source, evaluated with the members of `input` bound as
  root variables.
- `input` — a JSON object mapping each root name to its value. Absent means no
  variables are bound.
- `result` — the expected value, as JSON. **Numeric types are distinguished as
  CEL distinguishes them**: `1` is an `int` and `1.0` a `double`, and a case
  that mixes them is an error rather than a coercion.
- `error: true` — instead of `result`, for a case that must fail to evaluate.
  What is pinned is *that* it fails, never the message: the implementations are
  held to the same accept/reject decision and to the same values, not to each
  other's wording.

## What is deliberately not here

`size()` over non-ASCII strings. The CEL specification defines `size` on a
string as its number of **code points**; `cel` 0.14.3 returns its number of
**bytes** (`size('héllo')` is 6 there, and 5 by the specification — a JS
evaluator spelling it `[...s].length` would answer 5 too). Pinning the
specification's answer would leave a red test, and pinning the crate's would
institutionalise a defect in the artifact whose whole job is to keep two
implementations honest, so the corpus states only ASCII `size()` cases and this
paragraph is the record. It has to be resolved — upstream fix, pin bump, or a
`size` overload supplied by this project — before the JS evaluator lands.
