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
- **Flows are modules**: a subgraph declares an input/output schema; its I/O surface is interchangeable with a tool's, so flows are callable as tools by agents (the Agent Spec / PayPal recursion). Instantiable with bindings like Terraform modules.
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
| `agent` | LLM call with structured output; the tool-call loop is bounded by `max_tool_iterations` (default 8, §9.14) | node fn + Zod schema + structured output |
| `exec` | shell command (map input → env vars; string input → stdin) | child-process wrapper |
| `http` | HTTP request | fetch wrapper |
| `function` | host-registered function by name (escape hatch; breaks spec portability — documented) | registry lookup |
| `flow` | subgraph instantiation | separate compiled graph, invoked per instantiation (§9.17) |
| `map` | fan-out over an agent-produced collection, homogeneous or discriminator-routed (see 5.6) | in-task dispatch inside the map node (§9.17) |
| `human` | human-in-the-loop pause: input schema (what the human sees), output schema (what they return, routable like any structured output), `timeout` + `on_timeout` route | LangGraph `interrupt()` (grammar in v0; runtime support may land in M2, same reserved-grammar move as `placements`) |

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
- **Resume is an invocation.** With `human` nodes (5.5), an interrupted execution must be able to receive the human's response. The generated invocation surface therefore has two verbs — `start(flow, inputs)` and `resume(execution_id, payload)` — and the `http` trigger exposes both. Resume payloads are validated against the interrupting `human` node's output schema. This unifies triggers with the HITL story rather than bolting on a separate callback mechanism.
- **`event` backends (settled)**: the grammar is backend-agnostic — a trigger declares a logical `source:`; an `event_sources:` config section binds logical names to infrastructure (Redis Streams, SQS, NATS, …), mirroring `placements`' logical-vs-infra separation. Backends implement a minimal consumer contract (subscribe → payload stream + ack/nack) via a plugin interface. Delivery is at-least-once with inbound dedupe on message id — symmetric with the outbound `detach` idempotency-key design (5.6). M3 ships the interface plus one blessed reference backend (Redis Streams); further backends are plugins.
- **`respond: sync` (settled)**: sync is a compile-time-constrained mode. A flow exposed via `respond: sync` must be **statically interrupt-free** — no `human` node reachable from its entry, no wait-style constructs — enforced by graph reachability analysis (same static machinery as SCC and exhaustiveness). Sync triggers require a `timeout:` (default 60s); on expiry the response **upgrades to async** (HTTP 202 + execution id + status URL) while the execution continues durably — nothing cancelled, no work lost. Node retries consume the timeout budget with no special casing.
- **Codegen**: `manual` → the `run` CLI; `http` → a generated Fastify app wrapping the compiled graph (start / resume / status routes), reusing the same generated Zod schemas for payload validation. Auth on `http` triggers is deliberately out of scope for v0 (deploy behind your own gateway); a declarative auth story belongs with `placements` in M3.

### 5.12 Compiler contract

- **Generated code is a build artifact, never hand-edited.** The DSL is the single source of truth.
- **Deterministic codegen**: same DSL → byte-identical output (stable ordering), so regeneration diffs are meaningful.
- **Eject path**: for graphs that outgrow the DSL, copy generated code out and own it (CRA model).
- **Pinned backend versions**: each compiler release targets a pinned LangGraph (JS) version; upgrades are explicit and versioned, like Terraform providers. DSL semantics must never drift silently with upstream API churn.
- **Toolchain split (settled)**: the compiler/CLI is a single static Rust binary — parse, resolve, validate, and codegen carry no language-runtime dependency, which keeps `validate` in the millisecond budget and makes the toolchain installable anywhere a coding agent runs. Node.js is required only to execute generated output.

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

**M2 — Ergonomics**
- `agent-compose plan` (topology + validation diff between two specs).
- Tracing conventions in generated code (routing decisions as trace data).
- `eject` command.

**M3 — Distribution (design-gated)**
- `--target distributed` via `RemoteGraph`; execute `placements`.
- Postgres checkpointer wiring; isolation/credential story per placement.
- Execute `schedule` and `event` triggers; declarative auth for `http` triggers.
- Production `storage_backends` (Redis, pgvector, S3) behind the store plugin interface.
- Least-privilege env distribution: isolated deployments receive only statically-referenced secrets.

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
4. **Human-in-the-loop** → `human` node type in v0 grammar; runtime may land M2 (5.5).
5. **Spec versioning** → required `version:` field; semver on the spec; compiler declares a supported range; breaking changes gated behind a major bump + `agent-compose migrate` codemod; old syntax is never silently reinterpreted — refuse and point at the migration. Pre-1.0, minor bumps may break with a migration provided.
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

## 10. Open Questions

_None. New questions raised during grammar/spec work land here and must be resolved (moved to §9) before implementation of the affected area begins._

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
