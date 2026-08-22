# agent-compose — Product Requirements Document

**Status:** Draft v0.9
**Author:** moldy
**Last updated:** 2026-08-14

---

## 1. Vision

`agent-compose` is a declarative YAML DSL for defining agent graphs, compiled to executable LangGraph code. It is to agent graphs what `docker-compose` is to containers and what Terraform is to infrastructure: a human- and agent-writable specification layer that separates *what the graph is* from *how it runs*.

The DSL serves two audiences simultaneously:

1. **Developers**, who get a compact, diffable, reviewable representation of agent systems.
2. **Coding agents**, who can author and modify graph definitions at a fraction of the token cost of raw framework code, with a fast validation loop (`agent-compose validate`) that surfaces errors in milliseconds instead of runtime stack traces.

## 2. Problem

Building multi-agent systems today means writing imperative framework code (LangGraph, AutoGen, CrewAI). This has recurring costs:

- Graph structure is buried in code; reviewing "what changed in the topology" requires reading router functions and state mutations.
- Coding agents burn tokens generating and re-generating boilerplate, and get error feedback late (at runtime) rather than early (at validation).
- Definitions are not portable across deployment shapes: local prototype vs. distributed/isolated cloud execution requires structural rewrites.
- Routing decisions made by LLMs are opaque runtime behavior rather than inspectable data.

Prior art validates the declarative direction but leaves gaps:

- **Oracle Agent Spec** (arXiv 2510.04173): strong component model (control-flow vs data-flow edges, `$component_ref`), but a configuration format without executable control-flow semantics; memory/datastores punted to roadmap.
- **PayPal's declarative DSL** (arXiv 2512.19769): production-proven (60% dev-time reduction, sub-100ms orchestration overhead), JSON IR + static analysis, tools-as-pipelines — but DAG-only and enterprise-internal.
- **duckflux**: excellent minimal YAML ergonomics, CEL expressions, first-class error policy — but workflow-oriented, not agent-graph-oriented, and runtime-coupled rather than compile-to-framework.
- **LangGraph**: the right runtime primitives (cycles, checkpointing, subgraphs, `Send`, `RemoteGraph`) but no declarative authoring layer.

`agent-compose` combines: Agent Spec's component/edge model, PayPal's compile-to-IR + static analysis, duckflux's CEL + error-policy ergonomics, and LangGraph as the execution substrate.

## 3. Goals

- **G1.** A YAML DSL expressing agent graphs: agent defs, tool defs, deterministic nodes, edges (including cycles), shared state, and structured I/O.
- **G2.** A compiler pipeline, shipped as a single Rust binary: multi-file YAML → resolved, validated flat IR → deterministic LangGraph (TypeScript) codegen.
- **G3.** Validation as a first-class product: reference checking, schema compatibility across edges, routing exhaustiveness, cycle-termination proofs — all pre-runtime, with LSP-quality error messages.
- **G4.** Deterministic routing over model-produced values ("the LLM decides the value; the interpreter decides the transition").
- **G5.** Forward-compatible grammar for cloud/distributed placement (parsed from day one, executed later).

## 4. Non-Goals (v0)

- Building a bespoke runtime. LangGraph is the execution engine; we emit code, we do not interpret.
- Graph structure optimization/learning (GPTSwarm/AFlow-style). The DSL may later become a *target* for generated graphs, but v0 is hand/agent-authored.
- Visual editor.
- Multi-language codegen targets (a Python backend is a candidate for v2; TypeScript first).
- Executing the `placements` section (reserved keywords only in v0).

## 5. Core Design Decisions (settled in research phase)

### 5.1 Multi-file composition — explicit, Terraform-style

- Layout: `main.yml` + `agents/*.yml`, `tools/*.yml`, `flows/*.yml`, `state.yml`, `deploy.yml`.
- **Explicit imports** in `main.yml` — no directory scanning. "What is in this graph" must be unambiguous, and multiple entrypoints may share a definitions library.
- **Typed global address scheme**: `agent.researcher`, `tool.web_search`, `flow.review_loop`. References are type-checked ("this edge expects an agent, you gave it a tool").
- **Flows are modules**: a subgraph declares an input/output schema; its I/O surface is interchangeable with a tool's, so flows are callable as tools by agents (the Agent Spec / PayPal recursion). Instantiable with bindings like Terraform modules. Both call sites run: a `flow:` node instantiates deterministically, and a `flow.*` in an agent's `tools:` instantiates once per model tool call — under the instance identity §9.19 fixes and joined to the trace the way §9.20 fixes, with the flow's `inputs`/`outputs` as the tool's parameter/result schemas.
- The compiler resolves all files into **one flat IR artifact** (JSON): the diffable, versionable deploy artifact. Multi-file is authoring UX; the runtime/codegen sees a single resolved document.

### 5.2 Structured I/O — mandatory outputs, optional inputs

- **Every agent must declare an output schema** (JSON Schema). This is load-bearing for routing (5.3) and for distribution (5.7: every edge is serializable).
- Input schemas optional; default is string-in for entrypoint agents (duckflux's "string by default, schema opt-in").
- Codegen emits Zod schemas for state typing and reply parsing. The schema a provider is *constrained by* is the compiler's own JSON-Schema lowering — the same column the reply is parsed against — sent on the wire directly rather than through `withStructuredOutput`, whose Zod→JSON-Schema conversion silently drops constraints (§9.16).

### 5.3 Routing — runtime always routes; the question is who produced the value

The central reframe: even "agent-decided" routing is reified as a field in the agent's structured output (e.g. `next: enum[approve, revise, escalate]`). Edges carry optional CEL guards over the source node's output. The LLM decides the **value**; the compiled router decides the **transition**.

Consequences:
- The runtime stays fully deterministic (LLMs are unreliable routers: they forget steps, miscount iterations, skip transitions — duckflux thesis, Temporal replay requirement).
- Routing decisions appear in traces as data, not opaque model behavior.
- **Exhaustiveness checking**: enum with 3 variants + node with guarded edges + optional default → compile error if any variant is unroutable. No existing framework offers this.
- "Agent-routed" and "logic-routed" collapse into one mechanism; the guard simply references either a model-produced field or a computed one.

### 5.4 Cycles — allowed, with statically verified termination

Loops are prominent (evaluator-optimizer, ReAct, plan-revise); DAG-only loses. Design:
- Back-edges permitted in flow graphs.
- **Every cycle must carry a termination guard**: `max_iterations` and/or a CEL exit condition on at least one edge in the cycle.
- Compiler detects cycles (Tarjan SCC) and **rejects any SCC with no bounded edge**.
- Codegen emits an iteration counter into graph state per bounded cycle.
- Iteration boundaries are natural checkpoint/resume points.

### 5.5 Node taxonomy

| Node type | Semantics | Codegen |
|---|---|---|
| `agent` | LLM call with structured output; the tool-call loop is bounded by `max_tool_iterations` (default 8, §9.14), and a tool call the contract *refuses* returns to the model as a tool error rather than failing the node, spending one of that bound (§9.22) | node fn + Zod schema + structured output |
| `exec` | shell command (map input → env vars; string input → stdin) | child-process wrapper |
| `http` | HTTP request | fetch wrapper |
| `function` | host-registered function by name (escape hatch; breaks spec portability — documented) | registry lookup |
| `flow` | subgraph instantiation; also an agent's tool, one instance per model tool call (5.1, §9.19) | separate compiled graph, invoked per instantiation (§9.17) and per flow-as-tool call, whose instance joins the trace as a dispatch record (§9.20) |
| `map` | fan-out over an agent-produced collection, homogeneous or discriminator-routed (see 5.6) | in-task dispatch inside the map node (§9.17) |
| `human` | human-in-the-loop pause: input schema (what the human sees), output schema (what they return, routable like any structured output), `timeout` + `on_timeout` route | the node's task parks on a promise an answer settles — the app's resume route, or a terminal `run`'s prompt (M2; **not** LangGraph `interrupt()` — see resolved q4, q21) |

**Tool vs function — separate keywords (settled).** `tools:` on agent defs (LLM-discovered, nondeterministically selected, requires an LLM-facing description) and `function` as a node type (graph-invoked, deterministic, args checked against a signature) stay distinct: they differ in call semantics, validation, and trace semantics. The *definition* is unified — a single `tool.web_search` component can be attached to an agent's tool list and invoked as a function node (Agent Spec's def/use split).

Temporal-inspired discipline: the compiled graph is the *workflow* (deterministic, replayable); nodes are *activities* (effectful, retryable). Per-node `retry` / `timeout` / `on_error` policy is declared in YAML with duckflux's resolution chain: flow override > node > defaults > fail. Strategies: `fail`, `skip`, `retry` (exponential backoff), `fallback: <node ref>`.

The spec never contains executable code (Agent Spec's security posture). Code logic lives in three layers:
1. **CEL expressions** for conditions and mappings — non-Turing-complete, sandboxed, type-checked at parse time.
2. **First-class control constructs** in YAML: guarded edges, bounded loops, `map` fan-out, `when` guards.
3. **Escape hatches**: `exec`, `http`, `function` — implementation outside the spec, referenced by identifier.

CEL implementation choice (v0): the compiler parses and type-checks expressions with the Rust CEL implementation (`cel` crate); generated routers embed a JS CEL evaluator rather than transpiling CEL→TS. Two interpreters means a semantic-drift risk between `validate` and runtime — mitigated by a shared conformance fixture corpus run against both in CI. Transpilation is a later optimization.

### 5.6 Fan-out — agent-controlled cardinality, deterministic dispatch

The routing reframe (5.3) extends from *which edge* to *how many instances* and *which destination per item*. An agent never spawns work directly; it emits an **array in its structured output**, and the `map` construct dispatches over it deterministically. Cardinality and destination are data.

**Homogeneous fan-out:**

```yaml
nodes:
  plan: { agent: agent.planner }        # output.tasks: array, max_items: 20
  work:
    map:
      over: plan.output.tasks           # CEL path to the array
      as: task                          # per-instance input binding
      node: agent.worker                # any node ref, including flow.* subgraphs
      max_concurrency: 5
      on_item_error: skip               # fail | skip | retry per item
```

**Heterogeneous fan-out (per-item routing)** — motivating case: a bug-triage agent sends some findings to fixer agents and others to a manual-review queue. Items are a **tagged union** with a discriminator field; the map routes on it:

```yaml
agent.triage:
  output:
    findings:
      type: array
      max_items: 50
      items:
        discriminator: kind
        variants:
          auto_fixable: { file: { type: string }, patch_hint: { type: string } }
          needs_human:  { summary: { type: string }, severity: { enum: [low, high, critical] } }

nodes:
  dispatch:
    map:
      over: triage.output.findings
      route_by: kind
      routes:
        auto_fixable: { node: agent.fixer, max_concurrency: 5 }
        needs_human:  { node: tool.review_queue }   # sink route: enqueue via http/function
```

Rules and guarantees:

- **Bounding is mandatory** (mirrors the SCC cycle rule): `max_items` on the source array schema (enforced at structured-output validation — the model cannot return more) plus `max_concurrency` at execution. An unbounded fan-out is a compile error.
- **Exhaustiveness per variant**: every discriminator variant must have a route or an explicit `default:` — compile error otherwise. The agent structurally cannot produce an unroutable item.
- **Schema narrowing**: each route's target node is validated against its variant's payload shape only (discriminated-union narrowing; Zod discriminated unions in codegen), not a lowest-common-denominator item type.
- **Join via reducers**: instances run in isolated item-scoped contexts; writes to shared state must target channels with a declared `reduce` policy (`append`, `merge`, `last_wins`). Writing to a non-reduced channel from inside a `map` is a compile error. The downstream edge is the barrier: it fires when all instances complete, per `on_item_error` policy.
- **Deterministic ordering**: completion order is nondeterministic, so appended results are automatically index-tagged and reordered by source-item index before the join. Unordered reduces would silently break replay.
- **Sink routes and `detach`**: routes may target non-compute sinks (queues, webhooks, ticketing — this is how agent graphs talk to non-agent infrastructure). Default join semantics wait on *all* routes, including sinks (a failed enqueue is a surfaced failure). Per-route `detach: true` opts into fire-and-forget; detached routes cannot write to reduced state (compile error). **Replay interaction (settled)**: the design is idempotency-key delivery — an `idempotency_key` derived from `execution_id + node + item_index` is passed to the sink automatically, and sinks are documented to dedupe on it. The v0 implementation restricts: `detach` + checkpointing enabled is a validation error pointing at the roadmap (outbox-pattern delivery is not v0 work).
- **`route_by` is a literal discriminator field in v0**, not an arbitrary CEL expression — this keeps exhaustiveness checking decidable and the agent's contract legible in its schema. CEL-routed maps are a possible later escape hatch that forfeits exhaustiveness (mandatory `default:`).
- **Codegen**: one construct — the map node dispatches instances inside its own task with a bounded-admission scheduler (§9.17); reducer channels get standard LangGraph reducers with index-tagged, source-order folding; the map node's completion is the join. Heterogeneous is homogeneous with a lookup table.
- **Placement synergy**: map instances are the natural unit for `runtime: isolated` (5.10) — one worker per sandbox with zero change to the logical definition.

### 5.7 State — three tiers

1. **Node-scoped I/O**: typed inputs/outputs per node.
2. **Shared graph state**: a `state:` section generates the LangGraph state schema (`Annotation` channels, Zod-typed) with per-channel reducers. Default wiring is name-based (outputs write channels of the same name — Agent Spec's optional-data-edge insight). Subgraphs receive parent state only through explicit bindings (PayPal's `passVariables` discipline).
3. **Conversation history**: an implicit append channel available to agent nodes. **Scoping (settled): isolated by default across subgraph boundaries** — a flow's behavior must not depend on the caller's conversation, or reusability and the flow-as-tool equivalence break. Opt-in `context: inherit` on the flow-node instantiation for genuine continuation cases. Nothing crosses a module boundary implicitly.

**Data-edge syntax (settled): deferred.** Full explicit data edges are not in v0. Instead, a per-node `writes:` remapping (`writes: { summary: reviewer_summary }`) covers the two real failure modes of name-based wiring — channel collisions between nodes and renames across subgraph boundaries — at a fraction of the grammar cost. Full data edges remain an additive later feature if demand appears.

Persistence of *execution state* targets LangGraph checkpointers (Postgres for durable/distributed). Durable *attachable* storage — the cross-session memory and datastore story the literature punts on — is first-class: see 5.8.

### 5.8 Attachable storage — stores as first-class components

Graph state (5.7) is execution state. **Stores** are durable, attachable storage resources — the concrete answer to cross-session memory and datastores, which the surveyed prior art uniformly defers.

```yaml
store.user_prefs:
  kind: kv
  value_schema: { theme: { type: string }, verbosity: { enum: [low, high] } }
  scope: session

store.docs:
  kind: vector
  embed: { model: text-embedding-3-small }
  metadata_schema: { source: { type: string } }
  scope: global
```

- **Address scheme**: new `store.*` namespace; def/use split as everywhere else. Kinds in v0: `kv`, `vector`, `blob`. Relational/SQL is deliberately excluded — it drags query-language semantics into the spec; a `function` node wrapping a DB client is the escape hatch.
- **Two consumption modes** (mirrors tool vs function, 5.5):
  1. *Agent-attached* — `stores: [store.docs]` on an agent def; codegen synthesizes LLM-facing tools from the store's schema (`docs_search`, `user_prefs_get/set`). Nondeterministic, agent-invoked, recorded as tool calls.
  2. *Store-op nodes* — `{ store: store.user_prefs, op: get, key: state.user_id }`. Deterministic, graph-invoked (load context before an agent, persist after). Same definition, two usage surfaces, surface-specific validation.
- **Least-privilege attachment (§9.13)**: a `stores:` entry may declare `agent_access: read` (default `read_write`) to withhold the synthesized write tools from the model — the store-tool counterpart of placement isolation's blast-radius containment.
- **Scope**: `execution` (dies with the run) | `session` (persists across executions sharing a session key) | `global`. **Triggers supply session identity**: a trigger may declare `session_key: <CEL over payload>`; all session-scoped stores — and conversation history — key off it. Cross-session memory = session-scoped store + trigger-supplied session key, declaratively.
- **Backend binding (settled — alias-only)**: a store optionally declares `backend: <alias>` — a bare string naming an abstract slot, never provider config. All physical configuration lives in the per-target deploy layer:

  ```yaml
  # stores/docs.yml — logical, env-invariant
  store.docs:
    kind: vector
    scope: global
    embed: { model: text-embedding-3-small }
    backend: docs_db            # abstract alias; omit to use the kind default

  # deploy/staging.yml                 # deploy/prod.yml
  storage_backends:                    storage_backends:
    defaults:                            defaults:
      kv: { provider: redis, url: ${REDIS_URL} }
    aliases:                             aliases:
      docs_db:                             docs_db:
        provider: chroma                     provider: pgvector
        url: ${CHROMA_URL}                   url: ${DOCS_DB_URL}
  ```

  - **Resolution**: explicit alias → per-kind `defaults:` → target built-in. `--target local` substitutes SQLite/local disk for every store unconditionally — the zero-infra guarantee.
  - **Per-target invariant**: `--target <name>` loads `deploy/<name>.yml`. Only the deploy layer (storage_backends, placements, event_sources) forks per environment; `agents/`, `flows/`, `stores/`, `tools/` never do. Terraform-workspace discipline: env differences live in one layer.
  - **Compile-time validation**: provider config blocks are checked against provider-published schemas; an alias referenced by a store but undefined in the active target is a compile error naming the target; capability checks (e.g. `vector` store → vector-capable provider) apply at the alias definition. Provider config takes `${ENV_VAR}` references only — the spec never contains credentials (syntax checked at `validate`; presence checked at launch — see 5.9).
  - Rejected alternatives, for the record: address-keyed override maps in deploy.yml (action-at-a-distance) and inline provider config on stores (bakes env-specific choices into logical files, forking them per environment).
- **Replay discipline**: store ops are effects (activities). Reads are recorded — replay consumes history, not the live store. Writes are at-least-once with idempotency keys derived from `execution_id + node + item_index` — the third application of the **execution-derived idempotency key** principle (detach sinks 5.6, event dedupe 5.11), now a named cross-cutting rule.
- **Compile checks**: schema-checked ops against `value_schema`/`metadata_schema`; store writes inside a `map` require an item-derived key or a keyed `kv` write — unkeyed blob/global writes from concurrent instances are a validation error; `session`-scoped store usage in a flow with no session-keyed trigger is a validation error.

### 5.9 Providers & models — LLM configuration as first-class components

Two namespaces with the usual def/use split, because the layers change for different reasons: **`provider.*`** holds *connection* (kind, base_url, key ref), **`model.*`** holds *behavior* (model id + settings). Agents reference only `model.*`; swapping a project from hosted to local inference is a one-line provider edit.

```yaml
# providers.yml
provider.anthropic:
  kind: anthropic                  # plugin: anthropic | openai | openai_compatible | bedrock | ...
  api_key: ${ANTHROPIC_API_KEY}

provider.local:
  kind: openai_compatible          # ollama, vllm, proxies
  base_url: ${LOCAL_LLM_URL}
  api_key: ${LOCAL_LLM_KEY}

# models.yml
model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-6
  settings:                        # validated against the provider plugin's schema
    max_tokens: 8000
    thinking: { budget_tokens: 4000 }

model.fast:
  provider: provider.anthropic
  id: claude-haiku-4-5
  settings: { temperature: 0.2 }

model.default:
  route: [model.smart, model.fast]   # ordered fallback
  route_on: [rate_limit, overloaded, timeout]
```

- **Provider-specific settings**: each provider `kind` plugin publishes a settings schema; `settings:` blocks are compile-checked against it (`thinking:` on an OpenAI provider → validation error at the model def). Same mechanism as storage provider config (5.8).
- **Capability checking**: agents require structured output (5.2), so every referenced model's provider must declare structured-output/tool-use capability — an agent bound to a model that cannot honor the contract is a compile error.
- **Routes**: a `model.*` is either a direct binding or an ordered `route:` with `route_on:` failure conditions (rate limit, overload, timeout). Failover is deterministic runtime behavior recorded in the trace ("served by model.fast, fallback #1"). Route members are validated for capability equivalence so a fallback cannot silently break structured output. Content-based routing is explicitly out of scope — that is what graph edges are for.
- **No inline settings overrides on agents** — the backends lesson (5.8): one binding form. Different settings ⇒ define another named model. Every LLM configuration in a project stays greppable in one file; codegen stays deterministic.
- **Providers are logical-layer, not per-target**: unlike storage backends, providers rarely differ structurally per environment (same Anthropic everywhere; keys/URLs vary via env refs). The alias mechanism exists to extend if a real case appears; not pre-built.

**Secrets** (extends the credentials rule of 5.8):
- Config takes `${ENV_VAR}` references only, never literals. **Env refs survive into the IR unresolved** — resolution happens at process start in generated code, never at compile — so the flat IR stays committable/diffable and generated code never contains a key.
- `validate` and `build` check ref syntax only — an artifact must be buildable anywhere, including CI boxes holding no secrets (least-privilege distribution in 5.10 depends on this). Presence is a launch-time check: generated code verifies its environment at process start, and `run`/`serve` fail fast before invoking the graph, naming the missing variable (§9.15).
- **Least-privilege distribution**: under `--target distributed` (5.10), an isolated node's deployment receives only the env vars its resolved providers/backends reference — computable statically from the IR, since every secret is a named ref. Blast-radius containment for keys falls out of the design.

### 5.10 Cloud & distribution — placement annotations, not distributed edges

Logical definition and placement are orthogonal (Kubernetes/Terraform lesson):

```yaml
# deploy/<target>.yml — selected by --target <name>; see 5.8 per-target invariant
placements:
  agent.researcher:
    runtime: isolated      # own instance/container; sandbox, creds, network policy
  flow.review_loop:
    runtime: colocated
```

- `--target local`: one Node.js process.
- `--target distributed` (post-v0): N deployable LangGraph apps, boundaries stitched with `RemoteGraph`.
- Already-guaranteed properties make this nearly free: structured I/O on every node ⇒ every edge is serializable ⇒ any edge can become a network boundary without semantic change; shared checkpointer ⇒ one durable execution history across instances.
- Isolation is also a **security** feature (per-agent sandboxing, credentials, blast-radius containment for tool-wielding agents) — a declarative story no current framework has.
- v0: `placements` is parsed and validated but **no-op**.

### 5.11 Invocation & triggers — how executions come to exist

The logical graph defines *what runs*; **triggers** define *what causes an execution to exist*. A trigger is a (source, input binding, response mode) tuple declared in a `triggers:` section. This generalizes `entrypoint`: a project may declare multiple triggers targeting different flows — the entrypoints of a project are exactly the flows that triggers point at.

```yaml
# triggers.yml
triggers:
  cli:
    type: manual                      # implicit for every flow; shown for clarity
    flow: flow.review_loop

  on_request:
    type: http
    flow: flow.review_loop
    input:
      goal: payload.body.goal         # CEL over the trigger payload
    respond: async                    # sync | async
    callback: payload.body.callback_url   # optional completion webhook (async only)

  nightly:                            # reserved grammar in v0 (the `placements` move)
    type: schedule
    flow: flow.triage
    cron: "0 3 * * *"
    input: { scope: "'full'" }
```

Rules and semantics:

- **Input binding is compile-checked**: trigger `input:` mappings are CEL over the trigger payload, validated against the target flow's input schema — the same machinery as edge schema compatibility. A trigger that can produce an input the flow can't accept is a compile error.
- **`manual`** (v0): CLI/SDK invocation — `agent-compose run <flow> --input k=v`. Implicit for every flow with an input schema.
- **`http`** (v0): the graph as an endpoint. `respond: sync` blocks and returns the flow's output (only sane for fast graphs); `respond: async` (default) returns an execution id immediately, with an optional completion `callback:` webhook. Async is the natural pairing with durable execution.
- **`schedule`** and **`event`** (queue/subscription): reserved grammar in v0 — parsed and validated, no-op. Implementing them means owning a scheduler/consumer process (M3 territory), and event-backend plurality (SQS, Redis streams, NATS…) is a design project of its own.
- **Resume is an invocation.** With `human` nodes (5.5), an interrupted execution must be able to receive the human's response. The generated invocation surface therefore has two verbs — `start(flow, inputs)` and `resume(execution_id, payload)` — and the `http` trigger exposes both. Resume payloads are validated against the interrupting `human` node's output schema. This unifies triggers with the HITL story rather than bolting on a separate callback mechanism. The **status** route carries the question as well as the state: an `interrupted` report lists the pauses the execution is holding, each with what the human is shown, the schema their answer is validated against, and the URL that takes it — so a UI is built from the report rather than from the composition. An execution may hold more than one pause (a `map` over a flow that pauses), so a resume names which one it answers; a payload that fails validation does not consume the wait.
- **An answer has a second delivery surface: the terminal (settled, §9.21).** `agent-compose run` at a terminal renders each pause it reaches and reads the answer from standard input, holding it to the same `output:` schema against the same wait board — so a manual-trigger-only flow with a `human` node in it is completable without an app. It is the *delivery* that differs and nothing else: which pause an answer addresses, what refuses it, and what the trace records are decided once for both surfaces. A run with no terminal (and `AGENT_COMPOSE_INTERACTIVE=0`, which forces it) keeps reporting the pause and exiting `3`.
- **`event` backends (settled)**: the grammar is backend-agnostic — a trigger declares a logical `source:`; an `event_sources:` config section binds logical names to infrastructure (Redis Streams, SQS, NATS, …), mirroring `placements`' logical-vs-infra separation. Backends implement a minimal consumer contract (subscribe → payload stream + ack/nack) via a plugin interface. Delivery is at-least-once with inbound dedupe on message id — symmetric with the outbound `detach` idempotency-key design (5.6). M3 ships the interface plus one blessed reference backend (Redis Streams); further backends are plugins.
- **`respond: sync` (settled)**: sync is a compile-time-constrained mode. A flow exposed via `respond: sync` must be **statically interrupt-free** — no `human` node reachable from its entry, no wait-style constructs — enforced by graph reachability analysis (same static machinery as SCC and exhaustiveness). Sync triggers require a `timeout:` (default 60s); on expiry the response **upgrades to async** (HTTP 202 + execution id + status URL) while the execution continues durably — nothing cancelled, no work lost. Node retries consume the timeout budget with no special casing.
- **Codegen**: `manual` → the `run` CLI; `http` → a generated Fastify app wrapping the compiled graph (start / resume / status routes), reusing the same generated Zod schemas for payload validation. Auth on `http` triggers is deliberately out of scope for v0 (deploy behind your own gateway); a declarative auth story belongs with `placements` in M3.

### 5.12 Compiler contract

- **Generated code is a build artifact, never hand-edited.** The DSL is the single source of truth.
- **Deterministic codegen**: same DSL → byte-identical output (stable ordering), so regeneration diffs are meaningful.
- **Eject path**: for graphs that outgrow the DSL, copy generated code out and own it (CRA model).
- **Pinned backend versions**: each compiler release targets a pinned LangGraph (JS) version; upgrades are explicit and versioned, like Terraform providers. DSL semantics must never drift silently with upstream API churn.
- **Toolchain split (settled)**: the compiler/CLI is a single static Rust binary — parse, resolve, validate, and codegen carry no language-runtime dependency, which keeps `validate` in the millisecond budget and makes the toolchain installable anywhere a coding agent runs. A JavaScript runtime is required only to execute generated output.
- **Generated-artifact runtime (settled)**: that runtime is **Bun** by default — the documented install (`bun install`), launch (`bun src/index.ts`), and type gate (`bun run typecheck`) of every emitted project, and what `run`/`serve` and CI use. **Node.js >= 22.18 is a supported fallback**, declared in the emitted `engines.node` and kept honest by two gates: one installs a golden with npm and type-checks, constructs and runs it under Node, and one answers both shared conformance corpora — the schema table's Zod column and the CEL corpus — with that engine, since a `format:` regex and the emitted evaluator's arithmetic are the engine's rather than the emitted code's. Generated code itself is runtime-neutral: no `Bun` global, no `bun:` specifier, no import outside the pinned set — a static gate over the whole golden corpus, not a convention (§9.18).
- **Release distribution (settled)**: a tagged commit is the release. `v<version>` builds four artifacts — Linux `x86_64` and `aarch64` against **musl**, macOS `x86_64` and `aarch64` — each an `agent-compose-<version>-<target>.tar.gz` holding the one binary, published with a single `SHA256SUMS` over all four; the repository's `install.sh` verifies a download against it and installs nothing the checksum does not vouch for. musl rather than glibc because the single-binary promise is about *where it runs*: the job asserts static linkage rather than trusting the target name. The tag must equal the crate version, and `--version` carries the crate version, the short commit and the target triple, so a binary that was downloaded rather than built can say which build it is. The build, check and package steps are a callable workflow that a pull-request dry run calls with no tag and no publish, so the pipeline is exercised before the day it must not fail (§9.24).

## 6. Example (illustrative sketch, not final grammar)

```yaml
# main.yml
version: "0.1"
imports:
  - providers.yml
  - models.yml
  - agents/researcher.yml
  - agents/reviewer.yml
  - tools/web_search.yml
  - flows/review_loop.yml
  - triggers.yml            # see 5.11 — entrypoints are the flows triggers target

state:
  draft: { type: string }
  feedback: { type: string }

# agents/reviewer.yml
agent.reviewer:
  model: model.smart              # see 5.9 — no inline provider/settings
  tools: [tool.web_search]
  output:
    verdict: { enum: [approve, revise] }
    feedback: { type: string }

# flows/review_loop.yml
flow.review_loop:
  inputs:  { goal: { type: string } }
  outputs: { draft: { type: string } }
  nodes:
    write:  { agent: agent.researcher }
    review: { agent: agent.reviewer, retry: { max: 2, backoff: 5s } }
  edges:
    - { from: start, to: write }
    - { from: write, to: review }
    - { from: review, to: write,
        when: "review.output.verdict == 'revise'",
        max_iterations: 3 }          # termination guard on the cycle
    - { from: review, to: end,
        when: "review.output.verdict == 'approve'" }
```

Validator guarantees for this file: all refs resolve and are correctly typed; `verdict` routing is exhaustive (`revise` and `approve` both routed; compile error otherwise); the write↔review cycle is bounded; edge payloads are schema-compatible.

## 7. Milestones

**M0 — Spec & validator (the product's core loop)**
- Grammar spec + published JSON Schema (editor autocomplete/validation for free). Grammar includes `human` nodes, `writes:` remaps, `context: inherit`, `triggers` (manual/http active; schedule/event reserved), `store.*` definitions with both usage surfaces, `provider.*`/`model.*` definitions with routes, and `placements` (parsed; no-op).
- Parser → resolver (imports, address scheme) → flat IR.
- Static checks: reference/type resolution, schema compatibility across edges, routing exhaustiveness (edges and map variants), SCC cycle-termination, fan-out bounding, reducer-channel write rules inside maps, trigger input-binding compatibility, sync-trigger interrupt-free reachability, store-op schema checks and map-write keying, session-scope/session-key coherence, provider settings-schema and capability checks, route capability equivalence, env-ref syntax, unreachable nodes, undefined state channels.
- `agent-compose validate` with precise, actionable error messages. Error UX is a feature, not polish — it is the coding-agent feedback loop.

**M1 — Codegen (single process)**
- IR → deterministic LangGraph TypeScript: state models (incl. tagged unions via Zod), node fns, routers with embedded CEL, bounded cycles, homogeneous + discriminator-routed `map`→`Send` with index-tagged reducers, subgraphs, retry/timeout policy, store-op nodes + synthesized store tools with SQLite/local-disk backends, model routing with trace-recorded failover, env-ref presence checks at process start.
- `agent-compose build`, `agent-compose run` (manual trigger), `agent-compose serve` (generated Fastify app for http triggers: start/resume/status), golden-file codegen tests.
- Mock provider server + e2e harness: compiled graphs execute end-to-end in CI with scripted model responses, no API keys.

**M2 — Ergonomics & local adoption**

Exit criterion: a user can download a released binary, hand their coding agent the skill, and build/run flows locally — no clone of this repo, no Rust toolchain.

- `agent-compose plan` (topology + validation diff between two specs); `docs/plan.md` is normative for the document it writes, and is what a consumer pins `plan_version` on.
- Tracing conventions in generated code: a documented, stable trace format; routing decisions (which edge fired and the guard values that decided it, which map variant a discriminator chose, what terminated a cycle) as trace data.
- `human` node runtime (interrupt/resume through `serve`, and the same answer typed at an interactive `run`; grammar shipped in M0, runtime deferred here per resolved q4, second delivery surface per resolved q21).
- Flow-as-tool runtime (unblocked by resolved q19/q20).
- Progressive discovery: the binary teaches its own grammar. CLI surfaces sized for coding-agent consumption — `docs [<topic>]` (topic-scoped grammar documents), `explain <code>` (an expanded explanation per diagnostic code), `schema` (the published JSON Schema), `init [<dir>]` (a scaffold that validates) — so an agent can go from zero to authoring specs against only the installed binary (resolved q23).
- Installable agent skill: `agent-compose skill`, a packaged skill definition teaching the CLI and the discovery loop rather than the grammar, installable into a user's agent (Claude Code, Codex, …) (resolved q23).
- Release distribution: tagged releases publish prebuilt binaries for Linux and macOS (x86_64 + arm64) on GitHub Releases, with a checksum-verified install script and a dry run of the pipeline on every pull request (resolved q24).

**M3 — Distribution (design-gated; durability first — owner call)**

Sequenced by owner priority: durable execution leads, and ships on its own
before any distributed-placement work.

- **Durable execution**: executions survive a process restart — open `human`
  waits included — and resume across process generations. The mechanism is
  gated on §10's open durability questions; note that "Postgres checkpointer
  wiring" (this milestone's original spelling) predates resolved q17's in-task
  instance shaping and resolved q4's wait board, both of which put most of a
  run's live state where a LangGraph checkpointer cannot see it.
- `--target distributed` via `RemoteGraph`; execute `placements`; isolation/credential story per placement.
- Execute `schedule` and `event` triggers; declarative auth for `http` triggers.
- Production `storage_backends` (Redis, pgvector, S3) behind the store plugin interface.
- Least-privilege env distribution: isolated deployments receive only statically-referenced secrets.

Deferred out of the milestone sequence entirely (owner call, 2026-08-21): the
`eject` command. The eject *path* (PRD 5.12 — copy the directory, stop
regenerating) remains the documented escape hatch.

## 8. Risks

| Risk | Mitigation |
|---|---|
| LangGraph API churn breaks codegen | Pin backend version per release; explicit versioned upgrades; golden-file tests per pinned version |
| LangGraph JS lags LangGraph Python on needed primitives (`Send`, `interrupt`, `RemoteGraph`) | Primitive-parity audit as part of each pinned-backend upgrade; required primitives exercised by the e2e harness |
| DSL expressiveness ceiling → users bypass it | `function` escape hatch + `eject`; treat repeated ejects as grammar feedback |
| Hand-edited generated code forks source of truth | Generated-file headers, `build --check` in CI, docs discipline |
| CEL semantic drift between Rust validator and JS runtime | Shared conformance corpus run against both interpreters in CI; both implementations pinned per compiler release |
| Scope creep toward bespoke runtime | Hard non-goal; LangGraph owns execution |

## 9. Resolved Questions (log)

Formerly open, now settled — rationale lives in the referenced sections:

1. **Data edges** → deferred; per-node `writes:` remap ships in v0 (5.7).
2. **Conversation-history scoping** → isolated by default; opt-in `context: inherit` (5.7).
3. **Tool vs function** → separate keywords, unified definitions (5.5).
4. **Human-in-the-loop** → `human` node type in v0 grammar; runtime landed in M2, and it **parks a promise rather than calling LangGraph's `interrupt()`**. Two properties of the shaping decide it. `interrupt()` resumes a run from a checkpointer, which this release does not configure — durable execution is M3 — so it would buy nothing the parked promise does not, at the cost of a durability story the runtime cannot honour; and it announces itself out of the *top-level* stream, while map instances and `flow:` subflows run inside their node's own task as separate runs of separately compiled graphs (resolved q17), so a pause two instances down would have to be lifted through two run boundaries built to keep their instances' business inside them. Parking leaves the pause where it happened. What follows is the M2 durability boundary, stated in the emitted `README.md` and in grammar §8.7: a wait lives in the serving process, and a `serve` restarted while a human was thinking has lost it. Making waits survive a restart is durable execution's job (M3), and is the one thing a checkpointer-based shaping would then be reconsidered for (5.5, 5.11).
5. **Spec versioning** → required `version:` field; semver on the spec; compiler declares a supported range; breaking changes gated behind a major bump + `agent-compose migrate` codemod; old syntax is never silently reinterpreted — refuse and say what the reader must do. Pre-1.0, minor bumps may break with a migration provided. **Amended in M2 (resolved q23), and only about the message**: the refusal named `agent-compose migrate`, a verb no build has shipped, which G3 makes worse than saying less — a reader who acts on the help line meets a usage error, and the discovery surface's own bind (registry ⊇ documents) refuses a document that names a verb clap does not have. Until the codemod ships, the refusal names the versions this build accepts and says the port is of *syntax*, not of one field; the codemod commitment is unchanged and the message names it when there is one to name. `docs/grammar.md` §1.3 states the same thing where an author meets it.
6. **`detach` under durable execution** → idempotency-key design, v0 restriction (5.6).
7. **`event` backend abstraction** → pluggable consumer interface, backend-agnostic grammar via `event_sources:`, at-least-once + inbound dedupe, one blessed reference backend in M3 (5.11).
8. **`respond: sync` semantics** → compile-time interrupt-free requirement + mandatory timeout with async upgrade (202 + execution id) (5.11).
9. **Store backend binding** → alias-only: stores name abstract slots; per-target deploy files define them; no inline provider config, no address-keyed overrides (5.8).
10. **LLM providers & models** → `provider.*` (connection) / `model.*` (behavior) split; schema-validated provider settings; ordered failover routes on infrastructure conditions only; no inline overrides on agents; env-ref-only secrets surviving unresolved into the IR (5.9).
11. **Codegen target** → LangGraph TypeScript first; a Python backend is a possible v2 target (§4, 5.12).
12. **Compiler implementation** → Rust single-binary CLI (parse/resolve/validate/codegen); CEL via the Rust `cel` crate at validate time and a JS evaluator at runtime, kept in lockstep by a shared conformance corpus (5.5, 5.12).
13. **`agent_access` on store attachment** → accepted: a `stores:` entry may narrow the synthesized tool surface to read-only (`agent_access: read`; default `read_write`). One enum buys declarative least-privilege for store tools, the same posture placement isolation takes for compute (5.8, 5.10; grammar D37).
14. **`max_tool_iterations` on agents** → accepted, default 8: the intra-agent tool loop was the one remaining unbounded loop in a compiled graph; a declarative bound completes the static-termination story that 5.4 starts (5.4, 5.5; grammar D51).
15. **Env-ref presence checking** → launch-time, not build-time. 5.8/5.9 originally said `build` checks presence while the M1 milestone said process start; the artifact-portability posture decides it — a build must succeed anywhere, including CI holding no secrets, or committable IRs and least-privilege distribution (5.10) break. Generated code verifies its environment at process start; `run`/`serve` fail fast before invoking the graph. `validate` and `build` check syntax only (5.8, 5.9).
16. **The schema a provider is handed** → the compiler's JSON-Schema lowering, on the wire, not a `withStructuredOutput` conversion of the emitted Zod. The load-bearing property is constrain==parse equality: the schema a model is constrained by must be the schema its answer is parsed with, or an agent can honor its contract and still fail the parse. The pinned `@langchain/core` Zod conversion silently drops refinement-spelled constraints (`format:`, `uniqueItems`, length bounds), breaking that equality; the two columns are equal by construction only for the JSON lowering, proven document-by-document in CI. A permanent gate measures the dropped-constraint behavior so the choice is revisited if the conversion ever stops lossy-dropping (5.2; codegen schema module).
17. **Fan-out and subgraph shaping** → map instances run inside the map node's own LangGraph task (bounded admission, index-tagged joins) rather than via top-level `Send`, and `flow:` nodes invoke separately compiled graphs rather than in-graph subgraphs. Rationale: instance isolation (item-scoped context, per-instance history, §9.4 instance paths) and the grammar's per-node §7.6 semantics are guaranteed directly instead of recovered from scheduler internals; grammar 7.6.4 clause 1 was verified to hold under this shaping. The 5.5/5.6 codegen columns are updated; `Send`/subgraph remain available shapings if LangGraph's semantics ever make them cheaper to prove (codegen ledger).
18. **Runtime and package manager for generated artifacts** → **Bun by default; Node >= 22.18 a supported fallback; generated code free of Bun-only APIs.** Owner decision ("default to bun runtimes and pkg manager for generated artifacts"), interpreted as a statement about the *surfaces around* an emitted project rather than about the code inside it. Bun is what the emitted `README.md` documents first, what the generated-code gates, the property harness and the acceptance suite execute compiled graphs with, and what `run`/`serve` will launch; the gates install from a committed `bun.lock` with `--frozen-lockfile`, and CI pins the Bun version exactly for the reason 5.12 pins LangGraph exactly — a runtime that moved underneath CI would change what a green suite means with no commit saying so. The fallback is not a courtesy: `engines.node` carries the floor, `@types/node` stays on the same major, one gate installs a golden with npm and type-checks, constructs and runs it under Node (including re-deriving the inherited-property refusal list, which is the JavaScript *engine's* and must hold on both), a second answers both shared conformance corpora under that engine (a `format:` is a `RegExp` and the emitted CEL evaluator is `BigInt` arithmetic, so a corpus run under one engine says nothing about a reader on the other), and a static gate refuses any emitted module that names a `Bun` global, a `bun:` specifier, or an import outside the pinned set. The manifest itself stays installer-neutral: no `packageManager` field — it is corepack's, corepack does not manage Bun, and emitting it would pin every generated project to one Bun release while buying no determinism the exact version pins do not already give — and no emitted lockfile, because the reader's lockfile is the reader's (5.12).

19. **Instance identity for model-invoked subflows** → a per-call ordinal, fixed by the grammar. A flow-tool invocation contributes the frame `<tool_name>/<call_ordinal>` beneath the invoking agent node's own frame, where the ordinal counts prior invocations of that flow-tool within this agent-node execution. Distinct calls in one tool loop get distinct instance paths (the collision §9.4 exists to prevent), and an agent-node retry restarts the ordinals, so the Nth call of a retried attempt reuses the Nth key of the failed one. Because the emitter is a nondeterministic model, that reuse is **positional, not semantic** — the accepted at-least-once compromise, stated openly in the grammar. The alternatives lose outright: a provider tool-use id is fresh on every retry, so key-reuse dies and every agent retry re-fires every child flow's side effects; an argument hash merges two intentional identical calls. Like every other frame, the key is derived, never authored — no spec construct configures it and there is nothing for `validate` to check (grammar §9.4).

20. **Trace join for model-invoked subflows** → both surfaces, split by role — the span-link pattern. The child instance's full trace attaches as a canonical **dispatch record** keyed by its instance path (q19), exactly as a `flow:` node's instance does; the tool-call entry inside the agent's `ModelCall` records the call, the result the model saw, and the child's instance path as a **link**. Both invariants survive: every subflow instance in a trace is findable as a dispatch record (one shape for `plan`, replay, and the M2 tracing conventions to walk), and the tool loop's story is complete inside the `ModelCall` at the cost of one indirection. Nesting the child under the `ModelCall` alone would break the first invariant; a bare dispatch record alone would leave a tool call whose result came from nowhere.

21. **Answering a `human` pause from `run`** → accepted, and it closes a gap the human-node runtime (resolved q4) left open: a flow with a `human` node in it and no `http` trigger on it was **un-completable**. `run` reached the pause, wrote the trace with `status: "interrupted"` and exited `3` pointing at `serve`'s resume route — and a wait lives in the serving *process*, so the route that answers it belongs to a process `run` never started. Owner decision: **when a `run` parks at a pause and standard input is a terminal, the emitted CLI renders the node's `input:` — what the human is shown — reads the answer there, validates it against the declared `output:` exactly as the resume route does (an answer that does not fit re-prompts and does *not* consume the wait), and the execution carries on in the same process.** `timeout:`/`on_timeout:` budgets keep running while it waits, so an expiry routes exactly as it would under `serve` and the withdrawn prompt says so. A run whose standard input is **not** a terminal keeps the exit-`3` behaviour, so nothing hangs headless. Mechanically it is **the same wait board with a second delivery surface** — parallel to the way grammar §9.4 fixes one idempotency-key delivery surface per binding kind: which pause an answer addresses, what schema it is held to and what each refusal means are decided once, for both surfaces, so the two cannot disagree about one board. Four details are the terminal's own and are stated where a reader meets them (grammar §8.7, the emitted `README.md`). The framing is **one JSON value per line**, because a value spanning lines has no terminator a prompt could recognize without either guessing or hanging on a malformed one. The pauses of one execution are asked **one at a time**, each question being the **lowest-id pause open when it is asked** — the order the status route publishes them in, so a set of pauses waiting together is asked in the order the composition fixes rather than the one the scheduler parked them in. It is that rather than a total order over the run's pauses because a question already on the screen is not taken back: a pause that opens while one is being asked is asked after it, whatever its id sorts as. `AGENT_COMPOSE_INTERACTIVE` (`1` / `0`) decides the surface where a terminal cannot: it is how a **script** answers a pause, and how a supervisor keeps a run on the exit-`3` path it reads; a value that is neither is a usage error, not a setting nobody read (grammar D50). And standard input **ending** withdraws the surface — every pause still waiting, and every one the run opens after it, becomes the same interrupt a run with no surface raises — so a script that answered too few questions ends where it stood instead of parking for ever. Durability is unchanged: this is still one process and one parked promise, and making a wait survive a restart is durable execution's job (M3) (5.5, 5.11).

22. **What a tool call the contract refuses does** → **it returns to the model as a tool_result error so the model can self-correct**, rather than failing the agent node the way the flow-as-tool runtime (resolved q19, q20) originally shipped it. Owner ratification, and the scope is exactly two events wide. **Bounces**: arguments the tool's declared schema refuses — uniformly on all three surfaces a tool can be attached from, a `tool.*`'s `input:`, a synthesized store tool's argument row, and a `flow.*`'s `inputs:` — and a call naming a tool the agent was not offered. **Still fails the node**: failures of the tool's *execution* — an `exec:` exiting outside its accepted list, an `http:` request refused, a child flow instance that failed, a store backend that could not answer — where `retry:`/`on_error:` keep governing exactly as before. The line is what the model could do about it: a schema refusal is a statement about the *call*, which the model chose and can choose differently, while an execution failure is a statement about the world, and handing that back would ask a model to route around a broken system — the decision 5.3 takes away from models. The rule this replaces made a composition's reliability depend on a model never mis-typing an argument, and pushed authors toward loosening schemas until nothing was checked, which costs the constrain == parse property §9.16 is built on. **Termination is unchanged and is what makes the bounce affordable**: a correction is another model call, so a bounce spends one of `max_tool_iterations` (§9.14) — no second counter — several refusals in one answer are corrected together for one iteration, and a model that never corrects spends the bound and fails the node, with the node's error naming the last refusal it was holding so the run is diagnosable — *holding* read strictly, so a loop whose last call worked names none and the message never reports a mistake the model already corrected. Three consequences are stated where a reader meets them (grammar §5.4, §6, §11.5, D119): every call of an answer comes back to the model, refused or not — under the call's own id on both provider wire shapes, which refuse a request that leaves one unanswered, except for the one call Chat Completions will not let a request name at all (a tool the agent was never offered), whose `tool_call` is dropped from the replayed turn and whose refusal travels there as a `user` turn instead; a refused call spends **no** call ordinal — §9.19's ordinal counts a flow-tool's *invocations* and a refusal reaches no flow — so instance paths do not move when a model makes a mistake before them, and a node `retry:` whose second attempt needs no correction re-derives the first attempt's keys instead of re-firing every child flow's effects; and the refusal names what a compiler diagnostic would name — the tool as the *wire* offered it (a `tool.*`'s local name, the only spelling a corrected call could use), the field, the constraint, an excerpt of the offending value (G3) — which is one sentence written for the model that must act on it and for the person reading `ToolCallRecord.error`. The same tool reached from a `function:` node keeps naming its address and keeps failing the node: nothing there proposed a call, so there is nobody to hand it to (grammar §6, §8.4). The trace records it as a new `outcome` member, `"refused"`, rather than reusing `"failed"`, which bumped `trace_version` to `4`: version `3` defines `"failed"` as a call that ended the node and whose answer the model never saw, and both halves are false of a refusal (`docs/trace.md` §10.3.3) (5.1, 5.2, 5.3, 5.5, 5.8).

23. **The shape of the discovery surface, and of the skill** → **five verbs, three of them bound to a registry the compiler already keeps.** §7 M2's exit criterion is that a user downloads a released binary, hands their coding agent the skill, and builds and runs flows locally with no clone of this repo — so every byte an agent needs is embedded in the binary (5.12) and nothing is looked up at runtime. `docs [<topic>]` prints a topic index or one topic document; `explain <code>` prints an expanded explanation of one diagnostic code; `schema` prints the published JSON Schema byte-identically; `init [<dir>]` scaffolds a project that validates clean; `skill` prints or installs the skill. **The topics are curated derivatives, not slices**, and that is the load-bearing choice: `docs/grammar.md` is normative and is written to be *complete*, which is the wrong shape for a reader with a context window — so each topic is example-led, one screen of runnable YAML before any rule, and closes by naming the grammar sections it derives from. A derivative can drift from its source silently, so **three binds hold it**: every topic file is registered and every registered topic has a file; every numbered heading of `docs/grammar.md` is claimed by at least one topic's `Normative source` line, and every claim names a heading that exists, so a new grammar subsection fails a test that names it until somebody decides which topic teaches it; and the `cli` topic names every verb the binary has and every environment variable it reads. That last bind runs **both ways round**: documents ⊇ registry catches a verb nobody documented, and registry ⊇ documents catches the opposite and more damaging failure — a document instructing a reader to run something this compiler does not have — so every `agent-compose <verb>` and `agent-compose docs <topic>` written in code voice anywhere in the embedded documents must name a verb clap has and a topic the curriculum has — and, since a real verb missing a required argument is the same usage error by a likelier route, must also name at least as many arguments as clap's own usage line for that verb requires, everywhere a reader is being told what to type. The exception is a **table cell**, where these documents name a verb as a noun ("the terminal of an `agent-compose run`"): there the invocation is the row's subject rather than a line to run, and the alternative — arguments everywhere — would force those sentences into worse prose. The carve is a structural proxy with a named hole (an instructing invocation written inside a table), bounded by a table cell being a poor place to put a command anybody should type; naming a verb that does not exist is refused in a table cell as anywhere else. The alternative — generating topics from the grammar — was refused for the reason the derivative exists: a generated topic is the grammar again, at the grammar's size. **`explain` binds to the diagnostic registry by an exhaustive `match` over `DiagnosticCode`**, so a new failure class does not compile until it has an explanation; the reverse direction (no orphan explanation) is a test. Each explanation says what the check protects, shows a minimal spec that triggers it — and the spec is *run*, so an explanation whose example stopped triggering its code is a test failure rather than a lie shipped in a binary — then the fix and the cross-reference — which is itself bound, by the same parser that reads a topic's `Normative source` line: every grammar section and every decision an explanation cites must be one `docs/grammar.md` has, so a renumbered subsection or a retired decision cannot leave dozens of embedded documents pointing at nothing. *Coverage* stays the topics' obligation alone; what an explanation owes is a pointer that resolves. **Every published example is run**, marked by its fence's info string: a topic's opening spec must validate clean, and a deploy file (```` ```yaml deploy <target> ````, the one document kind `validate` cannot be pointed at alone) is written beside its topic's own composition and resolved under that target. An explanation may also write its **repair** out as a whole spec, held to the clean verdict a topic's example is: the fix bullets are the one part of an explanation nothing runs, and a repair that clears the rule it is about and lands the reader on a *second* diagnostic is worse than no repair — they followed the instruction they were given, inside a binary they cannot correct. Every explanation whose fix has a neighbouring check to fail carries one, and which those are is another set equality rather than a floor. Which documents *lack* an example is held to **set equality** against a named list with a reason each, not to a count: a count is satisfied by the documents that kept theirs while the ones that lost theirs go unchecked. **Every human report that carried a code** gains one trailing line pointing at `explain`, once per run rather than once per diagnostic: `validate`'s; the ones `build`, `run` and `serve` print before refusing, since those three validate first and print the same block through the same renderer; and a `plan`'s, which carries codes by two routes of its own — a `validation` section listing the findings the two sides disagree about, and, for a spec that does not resolve, the same diagnostic block `validate` would print. A reader who met a code without having typed `validate` is *further* from the verb that would explain it, not closer, which is where the pointer is worth most, and a `plan` is the report where that reader is likeliest of all — they were reviewing a change, not validating a spec. The JSON reports are untouched, because a machine reader gets the code and does not need to be told a verb exists. **The skill teaches the loop, never the grammar.** It is one self-contained document about `init` → edit → `validate` → `explain` → `docs <topic>` → `plan` → `build`/`run`/`serve`, with a verb table and the pointer that the topics are the curriculum; restating a grammar rule inside it would put a copy of a normative sentence in the one artifact this project cannot re-publish, since it is installed into somebody else's agent. Drift tests hold it to naming every verb and to listing the curriculum in the curriculum's own order — the two **registries the binary owns**, which are exactly what it may safely restate, as against a grammar rule, which it may not. **The install is one write under two roots.** `--agent claude` writes `.claude/skills/agent-compose/SKILL.md` and `--agent codex` writes `.codex/skills/agent-compose/SKILL.md` — the same path shape under each agent's own skills directory, project-local or under `$HOME` with `--global` — because both agents auto-detect the **same portable `SKILL.md`**: YAML frontmatter, then the document. So there is one installed document and one code path, parameterized by the root directory alone, and an agent that adopts the convention is a row in a table rather than a posture of its own. Each root is a directory whose whole content is skills, so either install refuses to overwrite a file that differs and no-ops on one that matches — differing by **bytes**, so a file that is not text at all is the refusal's case (exit `1`, the answer is no) rather than an unreadable one (exit `2`, the command could not run): what the refusal protects is that path's contents, and "could not decode" is not an answer to whether those contents are ours. *Owner correction (2026-08-20): this entry first recorded an asymmetric pair, `--agent codex` **printing** the document under a header on the reading that Codex's only surface was the user-authored `AGENTS.md`, which this compiler does not edit. Codex has first-class skills in the portable format, so that reading made the codex arm a worse install for no reason; the two arms are symmetric, and the print-only posture is gone.* (§7 M2, G3, 5.12).

24. **The shape of the release pipeline** → **four static artifacts, a hand-rolled workflow, and a dry run of it on every pull request.** §7 M2's exit criterion opens with "a user downloads a released binary", so the deliverable is the download: `agent-compose-<version>-<target>.tar.gz` for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-apple-darwin` and `aarch64-apple-darwin` — each holding the one binary, and the repository's `LICENSE` when there is one to hold — plus a single `SHA256SUMS` over all four, attached to a published (never draft) GitHub release with generated notes. **musl rather than glibc**, because 5.12's single binary is a claim about *where it runs*: a glibc build carries the build machine's symbol versions, so it fails on an older distribution and on alpine outright, while a static musl build survives both — and the build job asserts static linkage (`file` reporting `statically linked` or `static-pie linked`) rather than trusting the target's name. **Hand-rolled GitHub Actions rather than `cargo-dist`**: the product is one binary with no runtime dependencies, so packaging is a `tar` and a `sha256sum`, and owning ~200 lines of YAML outright beats pinning a generator whose version churn would dwarf the surface it generates. Revisit if the target list multiplies, or if an artifact stops being one file — installers, code signing and notarization, package-manager taps are where a generator starts earning its pin. **The tag and the crate version are one number.** A guard job reads `cargo metadata` and refuses to build when the tag and the crate version disagree, naming both; and because an artifact named for a version the binary does not report is the failure a user cannot diagnose, each smoke test holds `--version` to the version its archive is named for *and* to its own target triple. That line is new and carries three facts — crate version, short commit, target — since a binary that was downloaded rather than built has no other way to say which build it is; `GIT_SHA` wins over the checkout so a release names the sha it was cut from, and a tree with no `.git` beside it says `unknown` rather than failing to build. **The acceptance test is the dry run.** Build, check and package are a **callable** workflow, and both paths call it — the tag path with a tag to guard against, the pull-request path with neither a tag nor a publish step — so the two cannot drift into agreeing only until one is edited. **The pull-request gate builds the two Linux legs; the two macOS legs run on a tag and on a manual `workflow_dispatch` of the dry run.** This repository is private, where a macOS runner minute bills at ten times a Linux one, and two Darwin legs on every push of every pull request is a standing charge against a rare failure: the crate is pure Rust with no platform-specific code, so what a macOS leg catches that a Linux one does not is a linkage or a toolchain difference rather than a compile error. The rehearsal trigger is the compensation — `workflow_dispatch` runs the same workflow over all four legs from any branch, so a release can be practised in full without a tag being pushed for it — and the dry run cancels superseded runs of its own ref rather than paying for answers about commits that have already been replaced. The pull-request path additionally runs `install.sh` end to end against the artifacts it just built, in a local-artifact mode the script documents, including both refusals a checksum can produce. What the gate does **not** cover, stated rather than assumed: `gh release create` itself, which only a tag runs; **a macOS-only break**, which now surfaces at the manual rehearsal or at the tag rather than on the pull request that caused it — the honest price of the paragraph above; **the tag guard's refusal branch**, since a pull request passes no `expected-version` and so exercises the guard reading `cargo metadata` but never its disagreement; and — on the one leg where an arm64 macOS runner turns out to have no Rosetta — the *execution* of the x86_64 macOS binary, which then keeps its architecture and linkage assertions, loses its smoke test, and says so in a warning rather than passing quietly. **An unverified download is never installed.** `install.sh` (POSIX sh, `curl` or `wget`, `sha256sum` or `shasum`) resolves a release, downloads the archive and the `SHA256SUMS`, matches the archive's name as a whole field in it, and hard-fails both when the checksum disagrees and when the file says nothing about the archive — leaving the install directory empty either way. **The documented install is two commands rather than a pipe**: fetch the script, then run it. A pipe discards `curl`'s exit status, so a URL that answers `404` — which is what a private repository and an unreleased one both answer — pipes an empty body into a shell that runs it and exits `0`, installing no compiler and reporting no failure; the two-command form fails where the failure is, and leaves the script on disk to be read before it is run. The binary is **staged inside the install directory and renamed into place**, because a temporary directory is routinely another filesystem (`/tmp` is a tmpfs on most Linux distributions) where `mv` degrades to a copy over the destination — not atomic, and a truncating write onto a binary that may be executing. Until the repository is public and has a release, the README says plainly that those URLs `404` and names the two routes that work meanwhile: building from source, and the dry run's uploaded artifacts. Two mechanical choices are worth recording because each removed a dependency rather than adding one: aarch64 Linux links with **`rust-lld` out of the toolchain** — every dependency of this workspace is pure Rust, so no cross C toolchain, container or third-party build action is involved — and **`qemu-user-static` runs the resulting static binary directly**, with no container and no `binfmt` registration, so the one leg an x86_64 runner cannot execute natively is smoke-tested rather than merely inspected. Two actions outside the `actions/*` namespace are used, both already in `ci.yml`: `dtolnay/rust-toolchain`, which installs the *pinned* toolchain with an added target (the alternative is hand-written `rustup` lines that would drift from CI's), and `Swatinem/rust-cache`, which the pull-request gate turns on for speed and the release path leaves off — nothing a previous run left behind can reach a binary somebody downloads. **Both are pinned by full commit sha rather than by `@stable` or `@v2`**, in the release workflow and in `ci.yml` alike, since one supply-chain posture repo-wide is easier to hold than two: a branch and a tag are both whatever their owner moved them to last, and on the path that produces bytes other people download that is somebody else's push landing inside our release. The toolchain the action installs is named in the workflow rather than by the branch the action came from, so what the pin holds still is the action's own code and nothing else; bumping either sha is a commit that says so, in both workflows together. **The posture is repo-wide rather than release-only**, so the third action outside that namespace — `oven-sh/setup-bun`, which only `ci.yml` runs, since nothing a release builds needs a JavaScript runtime — is pinned by sha on the same terms: `@v2` is a tag its owner moves, and a rule with one exception left in it is not a rule (§7 M2, 5.12).

25. **What an `anthropic` or `openai` provider that holds no key means** → **`api_key` is required when the provider points at the vendor's own endpoint and optional when `base_url:` names something else**, and an absent key means the compiled runtime sends **no auth header at all** rather than an empty one. Owner ratification, and the case is corporate: a gateway injects the vendor credential server-side and employees never hold a key, so `kind: anthropic` with a `base_url:` and nothing else is the shape those deployments write — and 5.9's "swapping a project from hosted to local inference is a one-line provider edit" was untrue for the two kinds whose row made the key unconditional, which is what forced `openai_compatible` and a re-pointed model on a deployment that is still talking to Claude. The rule is one sentence with a reason on both halves: **pointing at the vendor's endpoint needs a key** — nothing else authenticates there, and the runtime's default `https://api.anthropic.com` is what an omitted `base_url:` resolves to — while **a gateway that injects one is named by `base_url:`**, which is the only thing in the definition that can say the traffic is not going to the vendor. A gateway that wants its *own* token still has `headers:`, which is interpolable, so `${PROXY_TOKEN}` reaches the wire without pretending to be a vendor credential. **The rejected alternative is simple-optional**: making `api_key` merely optional on both kinds admits `kind: anthropic` alone, which builds, ships, and then 401s on its first live call — moving a forgotten key from `validate` to production, in exchange for one fewer rule. G3 makes error UX a product feature and the whole point of the static pass is that a composition that cannot work does not compile, so the conditional keeps the forgotten-key failure where it was and buys the gateway shape with a diagnostic that names both repairs. `azure_openai`, `bedrock` and `vertex` are untouched — Azure has no default endpoint to reach and no keyless posture to describe, and the two SDK-reached kinds authenticate through their cloud's own credential chain — and `openai_compatible` was already the fully flexible kind. The compiler reports it as `missing-credential`, the published schema enforces the same conditional per Appendix B, and `docs/grammar.md` §12.1 and Decision D120 state it where an author meets it (5.9, G3).

## 10. Open Questions

_New questions raised during grammar/spec work land here and must be resolved (moved to §9) before implementation of the affected area begins._

26. **What mechanism makes an execution durable?** Two shapes are on the table. **(a) LangGraph's checkpointer interface** (Postgres/SQLite savers), the milestone's original spelling — but resolved q17 shapes subflow and map instances *inside* a task, and resolved q4 parks `human` waits on an in-process wait board, so the majority of a mid-run execution's live state is in places a graph-level checkpointer never sees; adopting it would mean restructuring both to be checkpointer-visible. **(b) A journal + replay of the trace this runtime already keeps**: every effect carries an idempotency key (resolved q10), store reads are recorded so "a replay has something to consume" (`docs/trace.md` §6, normative), every model call's attempts are recorded, and traversal ordinals are deterministic — i.e., the trace format was designed as a replay log, and durability is making the runtime *write it as one and read it back*. Lead recommendation: **(b)** — it fits the architecture as built, adds no LangGraph-version coupling to the durability story, and turns `docs/trace.md`'s existing replay discipline from documentation into the tested contract.

27. **Where does the journal live, and how is that chosen?** Recommendation: the journal backend is a **deploy-target slot**, exactly as `storage_backends` are — the composition says nothing, the target binds it. `--target local` binds a SQLite journal file beside the project (durable by default, zero configuration, one file to delete), and a distributed target binds Postgres; both sit behind one journal interface so the runtime cannot tell which it got. The rejected alternative is a grammar-level knob (`durability: on|off` in the spec), which would make the same composition durable on one machine and not another *by author declaration* — placement facts belong in deploy files (5.10), and whether an execution survives a restart is a placement fact.

28. **What does v1 durability scope cover?** Recommendation, three clauses: **`serve` auto-recovers** — on process start it replays every execution the journal holds open, including executions parked on `human` waits, which re-park with their wait ids intact (the wait id is deterministic — node path + ordinal — so a resume request that arrives after the restart still finds its wait); **`run` journals but does not auto-resume** — a crashed one-shot run is resumed explicitly by a new verb (`agent-compose resume <execution>`), because a CLI invocation ending is not evidence the user wants it re-run; and **triggers fire once** — recovery replays executions that exist, it does not re-fire the trigger that created them. Out of v1 scope, stated: cross-process migration (a journal written by one host resumed on another is M3-distribution's problem), and journal compaction.

29. **What does replay execute, and what does it not?** Recommendation: **replay is read-only up to the frontier**. A replayed execution consumes recorded model answers, recorded store reads, and recorded tool results by idempotency key — byte-for-byte, no re-issue — and only the frontier (the first effect the journal does not hold) reaches the network. Consequence worth stating now: this makes determinism of everything *between* effects (routing, CEL evaluation, ordinal assignment) a durability-correctness requirement rather than a trace-niceness, so the conformance corpus and the ordinal rules become part of the recovery contract, and a divergence found at replay time (recorded answer fails the current contract) fails the resume with a diagnostic naming the divergent step rather than silently re-executing.

## 11. References

- Oracle, *Open Agent Specification* — arXiv 2510.04173
- Daunis (PayPal), *A Declarative Language for Building And Orchestrating LLM-Powered Agent Workflows* — arXiv 2512.19769
- ADL — arXiv 2504.14787; SPL — arXiv 2607.07727
- Zhuge et al., *GPTSwarm: Language Agents as Optimizable Graphs* — arXiv 2402.16823
- Qian et al., *MacNet: Scaling LLM-based Multi-Agent Collaboration* — arXiv 2406.07155
- Yue et al., *From Static Templates to Dynamic Runtime Graphs* (survey) — arXiv 2603.22386
- Anthropic, *Building Effective Agents*
- duckflux spec — github.com/duckflux/spec
- LangGraph docs — docs.langchain.com
