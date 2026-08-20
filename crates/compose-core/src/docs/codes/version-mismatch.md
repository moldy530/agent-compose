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

If the imported file really was written against another version, bringing its
syntax forward is the work and the key is the last edit — the mismatch is the
symptom, not the problem. Overwriting the field on a file nobody has read since
is how a stale document joins a composition claiming to belong to it.

Grammar: `docs/grammar.md` §1.3. Topic:
`agent-compose docs getting-started`.
