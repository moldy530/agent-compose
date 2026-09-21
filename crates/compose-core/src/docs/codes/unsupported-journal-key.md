# unsupported-journal-key

## What it protects

A `journal:` block that declares a key the provider it binds does not take:
`url:` under `provider: sqlite`.

A SQLite journal is one file beside the project's stores —
`<project>/.agent-compose/journal.sqlite`, moved as a whole by
`AGENT_COMPOSE_DATA_DIR`, retained by leaving it and deleted by deleting it.
There is nothing to dial, so a `url:` written here is a key nobody reads: the
author who wrote it expected their executions to be recorded on a server, and
every one of them would go to the file instead, silently, for as long as the
target stays deployed. That is the inert key this grammar refuses everywhere
(D50, D61) — the harm is not the extra line, it is the belief it creates.

It is its own code rather than an `unknown-key` because `url:` **is** a key of
this construct: it is required one line up, under `provider: postgres` or
`provider: mysql`. Telling its author the block does not define it would be false
about the file in front of them. What is refused is the pair, which is the shape
`unsupported-server-tools` and `unsupported-detach` already have — a key that is
legal, met under a value that refuses it.

The opposite mistake has its own code too: a remote provider declaring no `url:`
is `missing-journal-url`.

## A spec that triggers it

The composition is beside the point — nothing in it names a journal, which is the
whole design: the target binds it. The deploy file is what refuses:

```yaml triggers
version: "0.1"

flow.pipeline:
  outputs: {}
  nodes:
    step: { exec: { command: "true" } }
  edges:
    - { from: start, to: step }
    - { from: step, to: end }
```

```yaml deploy staging
version: "0.1"

journal:
  provider: sqlite
  url: ${JOURNAL_URL}
```

## The fix

**Drop the key**, if the file is what this target wants. `provider: sqlite` takes
no further keys, and a target that declares no `journal:` block at all binds the
same thing — so the whole block is optional and its absence says exactly what
writing it says.

**Or bind the provider that dials**, if the address is what was meant. Both
remote providers take the `url:` as an `${ENV}` reference, never a literal:

```yaml deploy fixed
version: "0.1"

journal:
  provider: postgres
  url: ${JOURNAL_URL}
```

Grammar: `docs/grammar.md` §14.7, §4.3, Decisions D50, D148. Durability:
`docs/durability.md` §2, §10. Topic: `agent-compose docs targets`.
