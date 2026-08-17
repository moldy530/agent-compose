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
- `error: true` — instead of `result`, for a case that must produce no value.
  What is pinned is *that* it fails, never the message or the stage: the
  implementations are held to the same accept/reject decision and to the same
  values, not to each other's wording. **Refusing to parse counts** — `'\0'` is
  an escape neither CEL nor either implementation has, and the Rust column
  reports it from `Program::compile` where the JS column reports it from its own
  lexer. A case declaring `result` still has to parse in both.

## What the corpus deliberately covers

**64-bit integers.** CEL's `int` is an `int64`, and a JS evaluator backed by
`number` folds every value past 2⁵³ to its nearest double. `operators.json`
pins a literal, an addition, and an inequality either side of that boundary,
plus the overflow at `int64`'s own edge, so an evaluator that reaches for
`number` instead of `BigInt` fails here rather than silently rounding somebody's
id. The Rust answers are the specification's, so these are ordinary green cases.

**The escape table.** `escapes.json` states every escape CEL's *Lexis* admits —
the octal `\OOO` (three digits, the first `0`–`3`), `\a`, `\?`, the backtick,
`\xHH`, `\uHHHH`, `\UHHHHHHHH`, the named control characters, a raw string's
un-read backslashes — and the refusals beside them: `'\0'`, `'\00'`, `'\400'`,
`'\z'`, `'\u041'`, `'\X41'`, a lone surrogate, a code point past `U+10FFFF`. A
lexer is where two readings of one language diverge over an ordinary literal
rather than over an exotic construct: `'\011'` is a tab to both engines or it is
`\0` followed by `11` to one of them, and a guard comparing a channel to it then
answers differently in the validator and in the router the validator admitted.
Bytes literals are stated through `size()` and equality, because a `bytes` value
has no JSON form for a `result` to spell.

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

**A `matches()` pattern whose engine has to be Unicode-aware** — the other gap the
corpus cannot state, and for the same reason. The pattern is a *second* language
inside the expression, and its two implementations are not CEL's two: the specification
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
  in `RegExp` — so `'caté'.matches('\bcat\b')` is `false` there and `true` here;
- `\w`, `\d` and `\s` are the **Unicode** classes in the crate and ECMA-262's in
  `RegExp` — so `'é'.matches('^\w$')` and `'١'.matches('^\d$')` are `true` there
  and `false` here.

The first two are the rows `compose_core::codegen::schema`'s ledger already
carries for `pattern:` (`dot-matches-a-code-unit`,
`dot-excludes-a-line-terminator`, `word-boundary-is-unicode-aware`), reached
through a guard instead of a schema. The third is **only** here, and the reason
is the point: a `pattern:` is read by a JSON Schema validator, and JSON Schema
*defines* `pattern` as ECMA-262, so that column translates the Perl classes and
both sides of grammar 3.8's table agree; nothing translates for `matches()`.
`compose_core::codegen::cel`'s ledger carries all three for this surface, and
`the_unicode_aware_constructs_of_a_matches_pattern_still_read_differently` pins
the crate's answers. So `strings.json` states patterns built from anchors,
written-out classes, alternation, repetition, groups and escaped metacharacters —
where both engines agree — and none of the three constructs appears in one
except over ASCII, where they do agree.

`matches` in its **global** spelling is the gap that **was** closed. CEL's
standard definitions give the predicate two overloads, `s.matches(p)` and
`matches(s, p)`; `cel` 0.14.3 implements only the first, while the compiler's
front-end accepts both — grammar 4.1 puts the standard function set on the
surface, and the specification is what defines it — which left one expression
`validate` accepts and this project's Rust evaluator could not run.
`compose_core::cel::evaluation_context` now registers the crate's *own*
implementation under the global name, so the two spellings cannot answer
differently, and `strings.json` states both.

**The three predicates that have no global spelling**, and `has()` over a value
with no fields — the gaps that were closed on the *other* side. `matches` is the
only string predicate CEL's standard definitions give a global overload, so
`startsWith('abc', 'a')` is `Undeclared reference to 'startsWith'` in the Rust
column and the compiler's front-end refuses the spelling too; the emitted
evaluator used to answer `true`. `has(state.patches.goal)` over a list is the
same shape: the crate errors, the front-end refuses it by type, and the emitted
evaluator used to answer `false`. Neither is reachable through `validate` +
`build`, so neither could have bitten anybody — but an evaluator that accepts
more than the surface is drift pointing the other way (a composition that runs
and does not validate), which is a claim `compose_core::codegen::cel`'s header
makes and this corpus has to be able to keep. `strings.json` states the three
global spellings and `collections.json` the two `has()` refusals, all five as
`error: true` cases both columns now answer the same way.

**A quote escaped inside the other quote's literal** — the one row the escape
table leaves behind. CEL admits `\"` and `\'` in either kind of literal and gives
each one meaning, the quote; `cel` 0.14.3 keeps the backslash for the redundant
spelling, so `'\"'` is two characters there and one by the specification. The
emitted evaluator reads the specification's, which is the same call `size()`
above gets and for the same reason — the compiler never evaluates. `escapes.json`
states the two spellings where a quote is escaped inside its *own* literal, where
the columns agree, and
`a_quote_escaped_inside_the_other_quotes_literal_still_keeps_its_backslash` pins
the crate's answers. Where the crate refuses what the specification admits — a
`\n` inside `b'…'`, which its `parse_bytes` has no arm for — nothing reaches a
router either way: every expression is compiled with the crate before a project
is emitted, so the crate's refusal is the compiler's.
