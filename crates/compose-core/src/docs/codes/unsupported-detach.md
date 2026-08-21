# unsupported-detach

## What it protects

`detach: true` under a target whose execution state is durably checkpointed.
This is a **target-dependent** check: `local` runs with an in-memory
checkpointer and is not durably checkpointed, and every other target is — so the
same composition is legal under `--target local` and rejected under
`--target staging`.

A detached dispatch is resolved the moment it is issued and its outcome is never
observed. Under a checkpointer that means an effect that can be re-issued on
resume with nothing tracking whether it already happened, and v0 has no
outbox-pattern delivery to make that safe.

## A spec that triggers it

Not expressible in one file: the check needs a target, and `local` — the target
when none is named — is exactly the one it does not fire under. The composition
is an ordinary detached map, validated with `--target staging` against a
`deploy/staging.yml`:

```yaml
work:
  map:
    over: "state.tasks"
    node: agent.sink
    detach: true
    max_concurrency: 2
```

## The fix

Drop `detach:`, so the map joins on the dispatch like any other. The cost is
that the map node waits for it; the benefit is an outcome somebody observed.

Or validate against `local`, which is the honest answer while the composition is
still being developed — but the code will come back the moment it is checked
against the target it will actually be deployed to, which is what `validate
--target <name>` is for.

Grammar: `docs/grammar.md` §8.6, §14, Decisions D59, D87. Topics:
`agent-compose docs maps`, `agent-compose docs targets`.
