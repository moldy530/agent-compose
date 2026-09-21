# missing-journal-url

## What it protects

A `journal:` block that binds a provider which **dials out** — `postgres`,
`mysql` — and declares no `url:` for it to dial.

The journal is what makes an execution survive the process that started it:
every effect a compiled graph issues is written there as it happens, and a
resumed execution consumes that record read-only up to the frontier. A remote
journal is the release of that promise a single host cannot keep — the hub's
record survives the hub's disk, so a `serve` restarted on a fresh machine
recovers every open execution from the database. None of that is available
through an address nobody wrote.

Writing one key to require a second is rare here but not unique: a target that
declares `placements:` needs `hub.join_token:` (`missing-join-token`), a
`callback_auth:` makes `callback_allow:` mandatory
(`missing-callback-allowlist`), and a provider reaching a vendor's own endpoint
needs `api_key:` or a gateway's `base_url:` (`missing-credential`). What each has
in common is the shape of the failure: whether the key is required is decided by
a **sibling**, and the repair is a choice of two.

**The value is an `${ENV}` reference and never a literal.** That is the posture
every credential in this grammar takes, and it is what keeps `validate` from ever
seeing a URL: the deploy file names the slot, the environment holds the
credential, and the artifact carries the variable's *name*. A literal here is
`invalid-env-ref`, not this code. Presence is checked at **launch** rather than
at build — `build` reads no environment, and `run`/`serve`/`resume` fail fast
before invoking the graph, naming the variable.

The opposite mistake has its own code: a `url:` under `provider: sqlite`, which
opens a file beside the project and dials nothing, is `unsupported-journal-key`.

## A spec that triggers it

The composition is beside the point — nothing in it names a journal, which is
the whole design: the target binds it. The deploy file is what refuses:

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
  provider: postgres
```

## The fix

**Name the variable that holds the connection.** Both remote providers take one
`url:`, spelled as an environment reference:

```yaml deploy fixed
version: "0.1"

journal:
  provider: postgres
  url: ${JOURNAL_URL}
```

**Or bind `provider: sqlite`**, which needs no address at all — one file beside
the project's stores, moved as a whole by `AGENT_COMPOSE_DATA_DIR`, and deleted
by deleting the file. That is also what a target that declares no `journal:`
block gets, so the shortest repair of all is to remove the block: durable by
default, zero configuration.

Grammar: `docs/grammar.md` §14.7, §4.3, Decision D148. Durability:
`docs/durability.md` §2, §10. Topic: `agent-compose docs targets`.
