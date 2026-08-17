# The CEL conformance corpus

Shared fixtures for the two CEL implementations this project ships against: the
Rust evaluator the compiler links (`cel`, pinned in
`crates/compose-core/Cargo.toml`) and the JS evaluator generated routers embed
(`compose_core::codegen::cel`, emitted as `src/cel.ts`). CLAUDE.md's validation
strategy requires both to be run against these files, with divergence failing
CI. Two runners, one corpus:

| runner | column |
|---|---|
| `crates/compose-core/tests/cel_conformance.rs` | the pinned `cel` crate, under `compose_core::cel::evaluation_context` |
| `crates/agent-compose/tests/compiled_graph_acceptance/cel-conformance.mjs` | the evaluator a built project embeds, under the pinned Node |

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

## What the corpus deliberately covers

**64-bit integers.** CEL's `int` is an `int64`, and a JS evaluator backed by
`number` folds every value past 2⁵³ to its nearest double. `operators.json`
pins a literal, an addition, and an inequality either side of that boundary,
plus the overflow at `int64`'s own edge, so an evaluator that reaches for
`number` instead of `BigInt` fails here rather than silently rounding somebody's
id. The Rust answers are the specification's, so these are ordinary green cases.

## What is deliberately not here

`size()` over non-ASCII strings — the one gap the JS evaluator's arrival did not
close. The CEL specification defines `size` on a string as its number of **code
points**; `cel` 0.14.3 returns its number of **bytes** (`size('héllo')` is 6
there and 5 by the specification; the emitted evaluator spells it
`[...value].length`, so it answers 5, and 1 for `'👍'` where the crate answers
4). Pinning the specification's answer would leave a red test, and pinning the
crate's would institutionalise a defect in the artifact whose whole job is to
keep two implementations honest, so the corpus states only ASCII `size()` cases
— where all three answers agree.

Three things were tried before settling for that, and the record matters because
the obvious fix does not work: `Context::add_function("size", …)` does **not**
override a built-in (the standard set is consulted first), so this project
cannot supply a conformant `size` the way it supplies the missing `matches`
below; an upstream fix is not available at the pinned version; and refusing
`size()` over a string at validate time would refuse `size(state.feedback) > 0`,
which grammar 4.1 and 7.3.1 both write out as the ordinary shape of a guard.

What makes the gap **safe to leave** is where each evaluator runs: the compiler
never evaluates an expression (`compose_core::cel` type-checks and stops), so
the Rust column is the corpus's reference implementation rather than a runtime,
and the only evaluator a compiled graph ever runs is the emitted one — which
follows the specification. The row is carried in the divergence ledger in
`compose_core::codegen::cel`, and
`the_size_of_a_non_ascii_string_still_diverges_from_the_specification` pins what
the crate does today, so an upstream fix or a pin bump goes red and sends
whoever made it back here to add the cases.

**A `matches()` pattern carrying `.` or `\b`** — the other two the corpus cannot
state, and for the same reason. The pattern is a *second* language inside the
expression, and its two implementations are not CEL's two: the specification
defines `matches` over RE2, the crate runs the `regex` crate, and the emitted
evaluator has only `new RegExp(…)`. Most of the difference is refused rather than
recorded — `compose_core::codegen::diagnostics` refuses to **build** a
composition whose `matches()` pattern only one engine can *parse* (inline flags,
`(?P<…>)`, look-around, a backreference), and refuses a *computed* pattern, which
it cannot read at all. What survives a refusal is the constructs both engines
parse and read differently:

- `.` matches one **code point** in the crate and one UTF-16 **code unit** in
  `RegExp`, and excludes **LF alone** in the crate against every **line
  terminator** in `RegExp` — so `'👍'.matches('^.$')` and `'\r'.matches('^.$')`
  are `true` there and `false` here;
- `\b` sits between **Unicode** word characters in the crate and **ASCII** ones
  in `RegExp` — so `'caté'.matches('\bcat\b')` is `false` there and `true` here.

Both are the rows `compose_core::codegen::schema`'s ledger already carries for
`pattern:` (`dot-matches-a-code-unit`, `dot-excludes-a-line-terminator`,
`word-boundary-is-unicode-aware`), reached through a guard instead of a schema;
`compose_core::codegen::cel`'s ledger carries them for this surface, and
`the_dot_and_the_word_boundary_in_a_matches_pattern_still_read_differently` pins
the crate's answers. So `strings.json` states patterns built from anchors,
classes, alternation, repetition, groups and escaped metacharacters — where both
engines agree — and neither construct appears in one.

`matches` in its **global** spelling is the gap that **was** closed. CEL's
standard definitions give the predicate two overloads, `s.matches(p)` and
`matches(s, p)`; `cel` 0.14.3 implements only the first, while the compiler's
front-end accepts both — grammar 4.1 puts the standard function set on the
surface, and the specification is what defines it — which left one expression
`validate` accepts and this project's Rust evaluator could not run.
`compose_core::cel::evaluation_context` now registers the crate's *own*
implementation under the global name, so the two spellings cannot answer
differently, and `strings.json` states both.
