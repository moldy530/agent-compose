# version-mismatch

## What it protects

`version:` is optional in an imported spec file, and when present must be
**byte-identical** to the entrypoint's. One composition is written against one
version of the grammar; a file claiming another is either stale or was copied
from a different project, and both are worth a diagnostic naming the two files.

## A spec that triggers it

Not expressible in one file — it is a disagreement between two. The entrypoint
declares `version: "0.1"` and an imported file declares something else.

## The fix

Make them match, or drop the key from the imported file: it is optional there,
and leaving it out is the usual choice, since the entrypoint is the one place
the composition's version is worth stating.

If the imported file really was written against another version, run
`agent-compose migrate` on it rather than editing the field — the mismatch is
the symptom, not the problem.

Grammar: `docs/grammar.md` §1.3. Topic:
`agent-compose docs getting-started`.
