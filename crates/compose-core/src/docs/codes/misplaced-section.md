# misplaced-section

## What it protects

There are exactly two document kinds and they are **disjoint**. A spec file
carries `version`, `imports` (entrypoint only), `defaults`, `state`, `triggers`
and definition keys. A deploy file carries `version`, `hub`, `placements`,
`storage_backends`, `journal`, `package_registry`, `trace_sink` and
`event_sources`.

That split is the mechanical enforcement of the per-target invariant: **only the
deploy layer forks per environment**. A `state:` section in a deploy file, or a
`storage_backends:` in a spec file, would put composition-level meaning in a
per-environment document, and the same project would then mean different things
under different targets for reasons no reviewer could see in the flows.

The other half of this code: an imported file that turns out to be a deploy
file. Deploy files are selected with `--target`, never imported.

**And the third: a section in the right kind of document and the wrong target.**
`local` is built in, and it overrides two of those sections *unconditionally* —
it substitutes SQLite and local disk for every store, and it binds the SQLite
journal file beside the project. So `storage_backends:` and `journal:` in
`deploy/local.yml` are refused by name: neither could be honored without
contradicting the zero-infra guarantee, and honoring neither while accepting the
key would leave an author with a block they wrote, a substitution they expected,
and no diagnostic anywhere. A store or a journal that wants real infrastructure
is a `--target` of its own. Every other deploy section is live under `local`,
`hub:` and `placements:` included — a `local` mesh is this process as the hub
with a worker beside it, which is how a mesh is developed.

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

If the section is a `storage_backends:` or a `journal:` in `deploy/local.yml`,
the move is to a **target**: write `deploy/staging.yml` with the block in it and
build with `--target staging`. `local` keeps its zero-infra defaults, so the same
composition goes on validating and running on a laptop with nothing installed.

Grammar: `docs/grammar.md` §1.2, §1.5, §14, §14.7, Decisions D87, D148. Topic:
`agent-compose docs targets`.
