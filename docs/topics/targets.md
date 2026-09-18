# targets

A **target** is an environment. `deploy/<target>.yml`, selected with
`--target <name>`, is the only layer that forks per environment: `agents/`,
`flows/`, `stores/` and `tools/` never do. That invariant is what makes a
composition one artifact with several deployments rather than several
compositions.

A deploy file is never imported. It is a document kind of its own, and the two
kinds are disjoint — a spec file declaring `hub:`, `placements:`,
`storage_backends:`, `package_registry:`, `trace_sink:` or `event_sources:` is an
error, and so is a
deploy file declaring definitions, `imports:`, `state:`, `triggers:` or
`defaults:`.

```yaml deploy staging
# deploy/staging.yml
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}
  public_url: "https://hub.example"

placements:
  mac:
    members: [agent.researcher]
    description: the machine with the signing keys

storage_backends:
  defaults:
    kv: { provider: redis, url: "${REDIS_URL}" }
  aliases:
    docs_db: { provider: chroma, url: "${CHROMA_URL}" }

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
      token: ${NPM_CORP_TOKEN}

trace_sink:
  url: "https://collector.internal.example/v1/traces"
  format: otlp
  auth:
    bearer:
      token: ${TRACE_SINK_TOKEN}

event_sources:
  bug_reports:
    kind: redis_streams
    url: ${REDIS_URL}
    stream: bug-reports
    consumer_group: agent-compose
```

The spec side of that project needs nothing unusual — a store names an alias and
that is all:

```yaml spec
version: "0.1"
provider.openai:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.openai
  id: gpt-4o-mini

agent.researcher:
  model: model.smart
  prompt: Answer the question from the documents you are given.
  input:
    text: { type: string }
  output:
    answer: { type: string }

store.docs:
  kind: vector
  scope: global
  backend: docs_db              # an abstract slot, resolved per target
  embed:
    model: text-embedding-3-small
    provider: provider.openai

flow.ingest:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    save:
      store: store.docs
      op: upsert
      key: "input.text"
      value: "input.text"
  edges:
    - { from: start, to: save }
    - { from: save, to: end }
```

That file validates under `--target local` with no deploy file at all, and under
`--target staging` against the aliases above. The two are checked together: the
addresses `placements:` names — `agent.researcher` — have to resolve in the
composition the deploy file is a target of.

## `local` is reserved and built in

It is the target when `--target` is omitted, it runs with an **in-memory**
checkpointer, and it substitutes SQLite and local disk for **every** store
unconditionally. Four consequences:

- `deploy/local.yml` is **optional**, and `--target local` with no file is the
  zero-config path, not an error.
- When present it may declare `hub:`, `placements:`, `trace_sink:` and
  `event_sources:`. The first two are live grammar checked under every target — a
  `local` mesh is the hub and its workers on one machine, which is how you
  develop one — `trace_sink:` is live here too, because a laptop's `run` settles
  executions like any other target, and `event_sources:` is reserved grammar
  carried into the IR.
- It **must not** declare `storage_backends:`. That section is *active* grammar
  which `local` overrides unconditionally, so the block could only be an inert
  key whose author expected a substitution. A store that wants a real backend
  locally is a `--target` of its own.
- `--target <name>` for any other name **requires** `deploy/<name>.yml` to
  exist. A missing file is `io-error` naming the expected path, never a silent
  fall-back to built-ins.

Under `local` the two target-dependent binding checks are satisfied
**vacuously**: a store's `backend:` alias needs no definition, and an `event`
trigger's `source:` needs no `event_sources:` entry. That is the
zero-infrastructure guarantee — production infrastructure in
`deploy/staging.yml` and a project that still validates and runs on a laptop.

## Checkpointing is a property of the target

v0 declares no grammar for configuring a checkpointer. The rule the grammar
depends on is fixed instead: **`local` is not durably checkpointed; every other
target is.** Exactly one static check keys off it — `detach: true` on a `map`,
which is `unsupported-detach` under any checkpointed target. So the same
composition is legal under `--target local` and rejected under
`--target staging`, and `validate --target staging` is how you find that out
before deploying.

Checkpointing is **not** durability, and the two are easy to run together.
That rule is about a LangGraph checkpointer; what makes an execution survive a
restart here is a journal the runtime writes itself (below). `local` is still
the un-checkpointed target, and it is also a durable one.

## Durability is bound by the target too, and `local` binds SQLite

Every invocation of every flow is journaled — no key turns it on, and none turns
it off. Under `--target local` the journal is one SQLite file beside the
project:

```text
<project>/.agent-compose/journal.sqlite
```

beside the stores, moved whole by `AGENT_COMPOSE_DATA_DIR`, and deleted by
deleting it. The path is the same for `run`, `serve` and `resume` of one project
and one target, which is what lets one command finish what another started.

What is in it is every **effect** a run issued as it issued it: the model answers
it got, the results its tools produced, what its stores read and wrote, and what
a person answered a `human` node. A resumed execution re-runs the same graph and
**consumes** that record instead of re-issuing it, up to the frontier — the
first effect the journal does not hold — where it goes live again.

```sh
agent-compose run main.yml flow.review --input goal=ship
# execution: exec_9f1c8a3e-1b7d-4a20-9d61-1f0e8a2c4d55
# … the machine reboots …

agent-compose resume main.yml exec_9f1c8a3e-1b7d-4a20-9d61-1f0e8a2c4d55
# the two model calls it had already made are not made again
```

`serve` needs no such command: on start it recovers every execution the journal
holds open, before it accepts a connection, and one parked on a `human` pause
re-parks under the same wait id — so a resume request prepared against the
process that died still finds its wait. Triggers are not re-fired. It does not
wait for the replays, so a request that arrives before one is back at its pause
is refused with `recovering: true` and told to send it again.

Which is also why `resume` is the verb for an execution **nothing is running**:
one process at a time writes a project's journal, and a `serve` that is up has
already taken every open execution it holds. Finish those through its
`POST /executions/:id/resume` route.

Because the journal holds what a trace deliberately does not — completions, tool
results, a person's answer — it is **private recovery data with the same
sensitivity as this project's stores**, never an observability artifact. Nothing
uploads it and no command prints it.

A distributed target will bind Postgres behind the same interface; every target
this release can build is process-local, so every one of them binds SQLite and
there is nothing for a deploy file to say. `docs/durability.md` is normative,
and `docs/grammar.md` Decision D121 records why there is no grammar for it.

## `storage_backends`

| Key | Shape |
|---|---|
| `defaults` | kind (`kv`/`vector`/`blob`) → backend config, a per-kind fallback |
| `aliases` | alias identifier → backend config, the slots stores name via `backend:` |

A backend config requires `provider` and accepts provider-specific keys, checked
against the storage plugin's schema. Connection strings and credentials are
env-ref values only.

| Kind | v0 `provider` values |
|---|---|
| `kv` | `memory`, `sqlite`, `redis`, `postgres` |
| `vector` | `sqlite_vec`, `chroma`, `pgvector`, `qdrant` |
| `blob` | `local_fs`, `s3`, `gcs` |

Capability checks apply at the alias definition: a `vector` store bound to a
non-vector-capable provider is a compile error.

## `package_registry` — where the installer resolves packages

A built project is a plain npm project: somebody runs `bun install` in it, and on
a mesh every worker runs one over the artifact it just materialised. On a network
that mandates an internal mirror, the public registry is blocked, and a
`bunfig.toml` you drop beside the emitted project is not in the artifact — so the
workers never see it. Where a package resolves from is a placement fact, so it
lives here.

| Key | Shape |
|---|---|
| `url` | required; an absolute `http`/`https` URL naming a host, no wildcard, and no credential before an `@` — that is what `token` is for |
| `token` | an `${ENV}` reference, never a literal; omit it for a mirror that reads through without one |
| `scopes` | scope (written with its `@`) → `{ url, token? }`, for the scopes that resolve somewhere of their own |

`agent-compose build main.yml --target staging` turns that into **two** generated files beside
`package.json`: a `bunfig.toml` for Bun, which is the default installer, and an
`.npmrc` for npm and pnpm, which is the supported fallback. Both are on the
emitted file list — `build` overwrites them, `build --check` compares them, and a
build into a directory where your own `.npmrc` already sits is refused naming it.
A target that declares no `package_registry:` gets **neither** file: there are no
empty stubs.

Four things worth knowing:

- **A token is a name, not a secret.** Each file carries the environment
  variable's *reference* in that installer's own spelling — `${NPM_TOKEN}` in
  `.npmrc`, `$NPM_TOKEN` in `bunfig.toml` — and the installer expands it when it
  runs. Nothing secret is in the built directory, nothing secret crosses a mesh,
  and rotating the token does not change the artifact's hash, so workers are not
  redeployed over a credential change.
- **An unset variable is not an install-time error.** Both installers send the
  reference as text and the registry answers `401`. What names the variable
  instead is the environment manifest: this credential is on the hub's list *and*
  on every placement's, because every process installs — so `readEnvironment()`
  refuses at launch naming it, and a worker without it is refused at join.
- **npm authenticates by address, not by scope.** The `.npmrc` credential line is
  `//host/path/:_authToken=${VAR}`, keyed by the registry's authority and its
  whole path. That address is the one npm derives from its own request rather
  than the text you wrote — it appends the package name to your registry and
  walks *up* the result, so the host folds to lowercase, a default port drops
  away, `..` resolves, and a trailing `/` makes no difference. So
  `https://NPM.Example/repo/`, `https://npm.example/repo/` and
  `https://npm.example/repo` are one address, and `validate` compares them as
  one. Two shapes are refused naming both entries, each under its own code: two
  entries at one address with two different variables (an ini parser would keep
  the last) is `conflicting-registry-credential`, and an entry with **no**
  `token` at or under a tokened entry's address — it would spend the other's
  there while Bun sends nothing — is `missing-registry-token`. Give each entry
  its own path on the mirror, or give it the `token` it should spend.
- **A credential belongs in `token`, never in the URL.**
  `https://user:pass@npm.example/` is a spelling installers accept and the one
  Bun's own documentation shows, so `validate` refuses it here on purpose: it
  would put a literal secret in `.npmrc`, in `bunfig.toml`, in the emitted
  `README.md` and in the artifact hash over all three — and npm would then send
  those Basic credentials and ignore your `token` entirely.

## `trace_sink` — where every trace goes

Every other way of reading a trace is somebody asking for **one**: `run --format
json`, the trace file, `GET /executions/:id`, a trigger's `callback:`.
`trace_sink:` is the deployment saying, once, where all of them go — one address
every settled execution's trace envelope is POSTed to.

| Key | Shape |
|---|---|
| `url` | required; an absolute `http`/`https` URL naming a host, no wildcard — your collector, not a pattern |
| `format` | `envelope` (default) or `otlp` |
| `auth` | the outbound signing block an `http` trigger's `callback_auth:` takes: `bearer`, `hmac`, or both |

`format: envelope` POSTs the trace envelope itself — the exact object the trace
file holds. `format: otlp` POSTs an OTLP/JSON `ExportTraceServiceRequest` mapped
from that same envelope, hand-emitted over OTLP/HTTP with no OpenTelemetry SDK in
the generated project: the execution as the root span, every entry a span under
whatever ran it, model calls and dispatches as child spans with their tool calls
and failovers as events, routing decisions as attributes, and a flow-as-tool join
as a span link. `agent-compose docs trace` and `docs/trace.md` §12 are where the
mapping is written down — the span tree, the id derivation, the status table and
the resource attributes a later metrics exporter can correlate on — and a backend
that speaks only protobuf is served by pointing an OTel Collector at the sink.

A request that arrives with a W3C `traceparent` header carries its caller's trace
into the export: the root span adopts the caller's trace id and hangs off the
caller's span, so a graph embedded in somebody else's system appears inside their
trace rather than beside it. A malformed header is ignored, which is the W3C
behaviour and costs the execution nothing.

Four things worth knowing:

- **A sink outage costs deliveries a retry, never an execution.** Delivery is at
  settle, on the journal's delivery ledger — bounded retry, ordered per
  execution, and never blocking or failing the run it describes.
- **It applies wherever executions settle under this target**, which includes a
  one-shot `agent-compose run main.yml flow.triage` and not only a served
  process. The sink is a property of the target, not of a `serve`.
- **Only the envelope ships.** The journal's payloads — completions, tool
  results, a person's answer — are private recovery data and stay home, so the
  trace format's exclusions hold for the sink by construction.
- **There is no allowlist, and none is wanted.** `callback_allow:` exists because
  a callback URL comes out of a request payload and is attacker-controlled by
  construction. This one you wrote yourself, in this file, beside the database
  credentials — it is trusted on the same terms a connection string is.

The `auth:` credential is a deploy-layer variable, so it belongs to the hub's
environment manifest: the hub owns the trace and is the process that ships it, so
no worker is ever asked for a token it would never spend. It is held to the same
rule every other credential is: one that resolves to the **empty string** refuses
the app at launch, naming the variable, because an empty token is an
`Authorization: Bearer ` with nothing after it and an empty HMAC key signs a body
anybody can sign. A `run` has no launch to refuse at, so it says the same thing
on stderr and leaves the export in the journal for a start that can sign it.

## `hub` and `placements` — the distributed surface

A **placement** is a logical name a worker claims at an authenticated join, and
the components that claim runs. Nothing here is an address: which machine
satisfies `mac` is decided by whoever joins asserting it, so a placement is
capability affinity — the machine with the signing keys, the GPU, the licensed
tool — rather than load assignment.

| Key | Shape |
|---|---|
| `placements.<name>.members` | non-empty list of distinct `agent.*` / `tool.*` addresses that must resolve |
| `placements.<name>.description` | documentation |
| `hub.join_token` | the bearer credential a worker joins with, as an `${ENV}` reference — required wherever `placements:` is non-empty |
| `hub.public_url` | the absolute `http`/`https` base every ingress URL derives from; no wildcard, because this is your URL rather than an allowlist pattern |

Six rules, each a compile error — grammar §14.1's five about a placement, in
its numbering, and then §14.2's about the token:

- **§14.1 rule 1** — `members:` is required, non-empty, and names each component
  once.
- **rule 2** — a `flow.*` member is refused, naming the deferral: v1 places the
  leaves that hold a machine's capability, and a flow is a subgraph the hub
  schedules. Place the nodes it reaches instead.
- **rule 3** — placements are **disjoint**. One component in two of them is two
  answers to which worker runs it.
- **rule 4** — an attachment **colocates with its agent**. The whole artifact reaches every
  worker, so a placement decides which process runs a node, and an agent's
  `tools:` list is what runs a component inside the agent's own process. A tool
  placed somewhere other than the agent attaching it — including a placed tool on
  an *unplaced* agent, which would run on the hub — is refused; so is a placement
  reached only through a `flow.*` that agent attaches, because a flow-as-tool
  call starts its instance in the same tool loop. What the hub schedules is
  untouched: a placed tool reached from a `function:` node, or anything inside a
  flow instantiated by a `flow:` node, keeps its own placement, and there its own
  placement is the whole answer.
- **rule 5** — a store on a **process-local** backend is refused where a
  placement can open it. `memory`, `sqlite`, `sqlite_vec` and `local_fs` have no
  server in the middle: the process that reaches the store opens the bytes, and a
  mesh runs a placed component in more than one process by design — several
  workers may claim one placement, and the hub dispatches whatever else reaches
  the same store. Each would open its own copy, so a write on one side is never a
  read on another. The rule reads rule 4's closure rather than `members:`, refuses
  at every `scope:`, and applies under `--target local` too, where a hub and its
  workers are still separate processes. The repair is to bind a **networked**
  backend — whose variables the environment partition already carries to every
  placement that reaches the store — or to take the component that binds it out
  of `placements:`. Under `local` there is no `storage_backends:` to edit, so the
  repair there is a target of its own. **This release opens only the local
  backends**, so the second repair is the one a build of it runs: a networked
  backend compiles and refuses at the first store op until the store plugin
  interface lands in M3.
- **§14.2** — `hub.join_token` is required wherever placements are, and takes an
  `${ENV}` reference, never a literal. **Holding it is being trusted with the
  mesh**: the whole artifact, the right to claim any placement, and the journal's
  effect stream.

A component in **no** placement executes on the hub. That is the default, is
never a diagnostic, and is why the list above stops where it does —
`placements:` names the exceptions — so a target with no `placements:` is an
ordinary single-process deployment.

**The rules above are enforced, and the protocol behind them is built.** A build
of a target that declares `placements:` emits the hub — the five `/workers/*`
routes, the dispatch board, the artifact server — and the `worker` verb is the
process that joins it:

```
agent-compose worker --hub <url> --claim <name> --token-env <VAR>
```

A placed node is dispatched rather than called: it parks on the board until a
worker claiming its placement takes it, the worker runs it out of the artifact
the hub served, and its effects are journaled by the hub. The wire is normative
in `docs/distributed.md`, and `agent-compose docs cli` is where the verb's flags
are.

## `event_sources` — reserved

Maps the logical `source:` names of `event` triggers to infrastructure. `kind:`
is `redis_streams`, `sqs`, or `nats`, plus per-plugin keys such as `url`,
`stream`, `consumer_group`, `queue_url`, `subject`.

## What "reserved" means

Reserved constructs are **fully specified, parsed, type-checked, and carried
into the IR**, and execute as no-ops in v0. Using one is never an error; relying
on its runtime effect is a documented no-op.

| Construct | Status in v0 |
|---|---|
| `event_sources` | parsed + validated, no-op |
| `triggers.<t>.type: schedule` | parsed + validated, no-op |
| `triggers.<t>.type: event` | parsed + validated, no-op |

`human` nodes were on this list and have left it: the runtime landed, so a
compiled project really pauses and resumes — and their durability has left it
too: a wait that a restart interrupted comes back with its id intact, because
the wait id is the node's instance path and the journal is what a resumed
execution reads its answers out of.

**`placements` left it by being re-cut, and its runtime landed afterwards.** The
old shape keyed placements by component address and gave each a `runtime:
isolated|colocated` and a reserved `network:`. The distributed design settled a
different model — a named claim a worker asserts at an authenticated join — and
reserved grammar exists to be re-shapeable before its first execution: nothing
had run, so nothing broke. What replaced it went through a third posture on its
way here, live static grammar with a runtime still to come, and is now neither:
every rule above is enforced, a build emits the hub, the `worker` verb joins it,
and `docs/distributed.md` is what both halves are held to.

An `http` trigger's `auth:`, `callback_auth:` and `callback_allow:` have left it
as well, and they were the rows worth reading twice while they were here: a
no-op `schedule` is visible the first morning it does not fire, while a no-op
`auth:` would serve every caller and look exactly like a guarded route. The
generated app verifies callers, signs deliveries and refuses a callback URL
outside the list — see `agent-compose docs triggers`. What that adds to a
*target's* story is the credential list: those blocks' `${ENV}` references are in
the manifest a built project checks at process start, so a deployment receiving
only the secrets its own surfaces name receives these too.

Normative source: `docs/durability.md`, `docs/distributed.md`, `docs/trace.md`, `docs/grammar.md` §14, §14.1–14.6, §15
