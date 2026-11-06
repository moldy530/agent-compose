# invalid-cron

## What it protects

A `schedule` trigger's `cron:` is evaluated by `serve` itself: the emitted
scheduler reads the five fields, works out when the next tick is in the
trigger's `timezone:`, and starts an execution at it. So a field that scheduler
could not read is not a style problem — it is a schedule that would fail for the
first time at the start of a `serve`, or, worse, be read some way its author did
not mean. `validate` holds every field to the grammar the scheduler evaluates and
refuses the rest by name.

The grammar is POSIX's five fields and nothing more:

| field | values | names |
|---|---|---|
| minute | `0`-`59` | — |
| hour | `0`-`23` | — |
| day of the month | `1`-`31` | — |
| month | `1`-`12` | `JAN`-`DEC` |
| day of the week | `0`-`7`, both `0` and `7` Sunday | `SUN`-`SAT` |

Each field is a list of entries separated by `,`, and an entry is `*`, a value,
a range `a-b` (smaller end first), or a step `*/n` or `a-b/n` with `n` at least
`1`. Names are three letters, in any case, and only in the two fields that
take them.

What is refused, and why each one is refused rather than guessed at:

- **a value outside its field** — `60` minutes, hour `24`, day `0`;
- **a step over a single value** — `5/15`. Some dialects read it as "from 5,
  every 15", others refuse it; write `5-59/15` or `*/15`, which every reader
  agrees on;
- **a range that runs backwards** — `22-2` is not "late evening into the night"
  here; write `22-23,0-2`;
- **an empty list entry**, a name where no name is taken, and the extensions of
  other dialects — `?`, `L`, `W`, `#`, `@daily` — which this evaluator does not
  implement;
- **an expression that never fires** — `0 0 30 2 *` names a day no month it
  names has. Accepting it would be a trigger that waits for ever and says
  nothing.

The **shape** — five whitespace-separated fields and nothing around them — is
checked first and reported as `invalid-value`; this code is about what is inside
the fields.

**The day rule is POSIX's**, and worth knowing because it is the one rule that
surprises: when *both* day fields are restricted (neither is exactly `*`), a day
matches if **either** does. `0 0 13 * 5` fires on the 13th and on every Friday,
not only on Friday the 13th.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.sweep:
  outputs: {}
  nodes:
    run:
      exec: { command: "true" }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  quarter_hourly:
    type: schedule
    flow: flow.sweep
    cron: "5/15 * * * *"
```

## The fix

Say which values the step runs over. From minute 5, every 15 minutes, is the
range from 5 to the end of the hour:

```yaml spec
version: "0.1"
flow.sweep:
  outputs: {}
  nodes:
    run:
      exec: { command: "true" }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  quarter_hourly:
    type: schedule
    flow: flow.sweep
    cron: "5-59/15 * * * *"
```

The same expression is evaluated in the trigger's `timezone:` (default `UTC`),
and a wall-clock time that daylight saving skips fires at the first instant after
the gap, while one it repeats fires once. Ticks that fell while no `serve` was
running are skipped, never back-filled: a job catches itself up from
`payload.scheduled_at`.

Grammar: `docs/grammar.md` §13.4, Decision D162. Topic: `agent-compose docs triggers`.
