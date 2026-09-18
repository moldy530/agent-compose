# misplaced-section

## What it protects

There are exactly two document kinds and they are **disjoint**. A spec file
carries `version`, `imports` (entrypoint only), `defaults`, `state`, `triggers`
and definition keys. A deploy file carries `version`, `hub`, `placements`,
`storage_backends`, `package_registry`, `trace_sink` and `event_sources`.

That split is the mechanical enforcement of the per-target invariant: **only the
deploy layer forks per environment**. A `state:` section in a deploy file, or a
`storage_backends:` in a spec file, would put composition-level meaning in a
per-environment document, and the same project would then mean different things
under different targets for reasons no reviewer could see in the flows.

The other half of this code: an imported file that turns out to be a deploy
file. Deploy files are selected with `--target`, never imported.

## A spec that triggers it

```yaml triggers
version: "0.1"

storage_backends:
  aliases:
    docs_db: { provider: chroma, url: "${CHROMA_URL}" }
```

## The fix

Move the section to the document kind that takes it. Backend configuration
belongs in `deploy/<target>.yml`; the spec side names an abstract alias with
`backend: docs_db` and nothing more.

If an `imports:` entry resolved to a deploy file, drop the entry — `--target
<name>` is how that file is selected.

Grammar: `docs/grammar.md` §1.2, §1.5, §14. Topic:
`agent-compose docs targets`.
