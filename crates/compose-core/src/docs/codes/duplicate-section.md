# duplicate-section

## What it protects

`defaults:`, `state:` and `triggers:` are **singleton sections**: each may
appear in at most one file of a composition. Declaring `state:` in two imported
files is refused, naming both.

Merging them implicitly is the action at a distance the deploy layer already
refuses. Two `state:` blocks would mean the channel set — and with it every
name-based read and write in the project — depended on which files happened to
be imported.

## A spec that triggers it

Not expressible in one file: two `state:` keys in one mapping are
`duplicate-key`, caught by the parser. This code is for two **files** — the
entrypoint declaring `state:` and an imported `channels.yml` declaring it too.

## The fix

Merge them into one file. `state:` conventionally lives in the entrypoint beside
`imports:` and `defaults:`, which keeps the whole channel set on one screen —
and the channel set is exactly the thing you want on one screen, because every
name-based write in the project lands in it.

Grammar: `docs/grammar.md` §1.5, Decision D2. Topic:
`agent-compose docs state`.
