# duplicate-definition

## What it protects

A typed address is defined **once composition-wide**. `agent.reviewer` is one
name whichever file declares it — multi-file layout is authoring convenience,
and the resolver flattens everything into one artifact.

Two definitions of one address would leave the meaning of every reference to it
depending on import order, which is deliberately not a thing that affects
semantics here.

## A spec that triggers it

Not expressible in one file: two declarations of one address inside a single
mapping are `duplicate-key`, caught by the parser. This code is for two
**files** — `first.yml` and `second.yml` both declaring `provider.p`, both
imported by the entrypoint.

## The fix

Rename one, or drop the file that duplicates the other. The diagnostic names
both sites, so the second label is the file you did not have open.

Where two definitions really are meant to be the same thing, delete one and let
both files reference the surviving address: an address is global, so an
`agent.*` in `agents/reviewer.yml` is visible to every flow in the project with
no import of its own.

Grammar: `docs/grammar.md` §1.5, §2.2. Topic:
`agent-compose docs getting-started`.
