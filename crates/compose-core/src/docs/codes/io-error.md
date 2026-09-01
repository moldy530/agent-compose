# io-error

## What it protects

A file the composition names has to be there. Three positions raise this: an
`imports:` entry naming a file that cannot be read, `--target <name>` where
`deploy/<name>.yml` does not exist, and a `module:` binding whose implementation
is not on disk.

The deploy file is the one worth knowing about. Resolving a missing deploy file
to built-in backends would deploy against the wrong infrastructure with no
diagnostic at all, so a named target's deploy file is required rather than
optional. `local` is the exception, and only because it resolves no aliases: it
is the built-in target, and `--target local` with no file is the zero-config
path.

The module implementation is the one raised most often, and the only one whose
repair is a **command** rather than an edit: `agent-compose build <spec>`
scaffolds a referenced module it cannot find — the typed signature, the contract
as a doc comment, and a body that throws — writes it once, and never writes that
file again. So `validate` and `build --check` refuse a binding whose file is
absent, because a composition missing an implementation does not run, while a
plain `build` does not refuse: writing that file is what it is for.

## A spec that triggers it

```yaml triggers
version: "0.1"
imports:
  - agents/reviewer.yml      # nothing at this path

flow.f:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
```

## The fix

Check the path. `imports:` entries are relative to the entrypoint's own
directory — the project root — and must stay inside it. There is no directory
scanning, so a file that exists but is not listed is not in the composition
either.

For a missing deploy file, write `deploy/<name>.yml`, or validate against
`local`, which needs none.

For a missing module implementation, run `agent-compose build <spec>` and fill in
the stub it writes at the bound path — it is scaffolded from the tool's own
schemas, so the signature is already the right one. If the path is the mistake
rather than the file, fix the binding first: a build against a mistyped path
writes a stub nothing will ever call.

Grammar: `docs/grammar.md` §1.4, §6.1, §14, Decisions D87, D132. Topics:
`agent-compose docs getting-started`, `agent-compose docs targets`,
`agent-compose docs tools`.
