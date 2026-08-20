# io-error

## What it protects

A file the composition names has to be there. Two positions raise this: an
`imports:` entry naming a file that cannot be read, and `--target <name>` where
`deploy/<name>.yml` does not exist.

The second is the one worth knowing about. Resolving a missing deploy file to
built-in backends would deploy against the wrong infrastructure with no
diagnostic at all, so a named target's deploy file is required rather than
optional. `local` is the exception, and only because it resolves no aliases: it
is the built-in target, and `--target local` with no file is the zero-config
path.

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

Grammar: `docs/grammar.md` §1.4, §14, Decision D87. Topics:
`agent-compose docs getting-started`, `agent-compose docs targets`.
