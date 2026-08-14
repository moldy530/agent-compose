# agent-compose — Product Requirements Document

**Status:** Draft v0.5
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
- **G2.** A compiler pipeline: multi-file YAML → resolved, validated flat IR → deterministic LangGraph (Python) codegen.
- **G3.** Validation as a first-class product: reference checking, schema compatibility across edges, routing exhaustiveness, cycle-termination proofs — all pre-runtime, with LSP-quality error messages.
- **G4.** Deterministic routing over model-produced values ("the LLM decides the value; the interpreter decides the transition").
- **G5.** Forward-compatible grammar for cloud/distributed placement (parsed from day one, executed later).

## 4. Non-Goals (v0)

- Building a bespoke runtime. LangGraph is the execution engine; we emit code, we do not interpret.
- Graph structure optimization/learning (GPTSwarm/AFlow-style). The DSL may later become a *target* for generated graphs, but v0 is hand/agent-authored.
- Visual editor.
- Multi-language codegen targets (TS backend is a candidate for v2; Python first).
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
- Codegen emits Pydantic models + `with_structured_output`.

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
| `agent` | LLM call with structured output | node fn + Pydantic + structured output |
| `exec` | shell command (map input → env vars; string input → stdin) | subprocess wrapper |
| `http` | HTTP request | httpx wrapper |
| `function` | host-registered function by name (escape hatch; breaks spec portability — documented) | registry lookup |
| `flow` | subgraph instantiation | LangGraph subgraph |
| `map` | fan-out over an agent-produced collection, homogeneous or discriminator-routed (see 5.6) | LangGraph `Send` API |
| `human` | human-in-the-loop pause: input schema (what the human sees), output schema (what they return, routable like any structured output), `timeout` + `on_timeout` route | LangGraph `interrupt()` (grammar in v0; runtime support may land in M2, same reserved-grammar move as `placements`) |

**Tool vs function — separate keywords (settled).** `tools:` on agent defs (LLM-discovered, nondeterministically selected, requires an LLM-facing description) and `function` as a node type (graph-invoked, deterministic, args checked against a signature) stay distinct: they differ in call semantics, validation, and trace semantics. The *definition* is unified — a single `tool.web_search` component can be attached to an agent's tool list and invoked as a function node (Agent Spec's def/use split).

Temporal-inspired discipline: the compiled graph is the *workflow* (deterministic, replayable); nodes are *activities* (effectful, retryable). Per-node `retry` / `timeout` / `on_error` policy is declared in YAML with duckflux's resolution chain: flow override > node > defaults > fail. Strategies: `fail`, `skip`, `retry` (exponential backoff), `fallback: <node ref>`.

The spec never contains executable code (Agent Spec's security posture). Code logic lives in three layers:
1. **CEL expressions** for conditions and mappings — non-Turing-complete, sandboxed, type-checked at parse time.
2. **First-class control constructs** in YAML: guarded edges, bounded loops, `map` fan-out, `when` guards.
3. **Escape hatches**: `exec`, `http`, `function` — implementation outside the spec, referenced by identifier.

CEL implementation choice (v0): embed `cel-python` in generated routers rather than transpiling CEL→Python. Semantic fidelity between `validate` and runtime beats zero-dep purity; transpilation is a later optimization.

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
- **Schema narrowing**: each route's target node is validated against its variant's payload shape only (discriminated-union narrowing; Pydantic tagged unions in codegen), not a lowest-common-denominator item type.
- **Join via reducers**: instances run in isolated item-scoped contexts; writes to shared state must target channels with a declared `reduce` policy (`append`, `merge`, `last_wins`). Writing to a non-reduced channel from inside a `map` is a compile error. The downstream edge is the barrier: it fires when all instances complete, per `on_item_error` policy.
- **Deterministic ordering**: completion order is nondeterministic, so appended results are automatically index-tagged and reordered by source-item index before the join. Unordered reduces would silently break replay.
- **Sink routes and `detach`**: routes may target non-compute sinks (queues, webhooks, ticketing — this is how agent graphs talk to non-agent infrastructure). Default join semantics wait on *all* routes, including sinks (a failed enqueue is a surfaced failure). Per-route `detach: true` opts into fire-and-forget; detached routes cannot write to reduced state (compile error). **Replay interaction (settled)**: the design is idempotency-key delivery — an `idempotency_key` derived from `execution_id + node + item_index` is passed to the sink automatically, and sinks are documented to dedupe on it. The v0 implementation restricts: `detach` + checkpointing enabled is a validation error pointing at the roadmap (outbox-pattern delivery is not v0 work).
- **`route_by` is a literal discriminator field in v0**, not an arbitrary CEL expression — this keeps exhaustiveness checking decidable and the agent's contract legible in its schema. CEL-routed maps are a possible later escape hatch that forfeits exhaustiveness (mandatory `default:`).
- **Codegen**: one construct — the compiled router emits `Send(routes[item.kind], item)` per item onto LangGraph's `Send` API; reducer channels get standard LangGraph reducers; superstep semantics provide the join. Heterogeneous is homogeneous with a lookup table.
- **Placement synergy**: map instances are the natural unit for `runtime: isolated` (5.8) — one worker per sandbox with zero change to the logical definition.

### 5.7 State — three tiers

1. **Node-scoped I/O**: typed inputs/outputs per node.
2. **Shared graph state**: a `state:` section generates the LangGraph `State` schema (TypedDict/Pydantic) with per-channel reducers. Default wiring is name-based (outputs write channels of the same name — Agent Spec's optional-data-edge insight). Subgraphs receive parent state only through explicit bindings (PayPal's `passVariables` discipline).
3. **Conversation history**: an implicit append channel available to agent nodes. **Scoping (settled): isolated by default across subgraph boundaries** — a flow's behavior must not depend on the caller's conversation, or reusability and the flow-as-tool equivalence break. Opt-in `context: inherit` on the flow-node instantiation for genuine continuation cases. Nothing crosses a module boundary implicitly.

**Data-edge syntax (settled): deferred.** Full explicit data edges are not in v0. Instead, a per-node `writes:` remapping (`writes: { summary: reviewer_summary }`) covers the two real failure modes of name-based wiring — channel collisions between nodes and renames across subgraph boundaries — at a fraction of the grammar cost. Full data edges remain an additive later feature if demand appears.

Persistence (`storage:` section) targets LangGraph checkpointers (Postgres for durable/distributed). Novelty opportunity: the literature consistently punts on cross-session memory and datastores — this is a differentiation area post-v0.

### 5.8 Cloud & distribution — placement annotations, not distributed edges

Logical definition and placement are orthogonal (Kubernetes/Terraform lesson):

```yaml
# deploy.yml
placements:
  agent.researcher:
    runtime: isolated      # own instance/container; sandbox, creds, network policy
  flow.review_loop:
    runtime: colocated
```

- `--target local`: one Python process.
- `--target distributed` (post-v0): N deployable LangGraph apps, boundaries stitched with `RemoteGraph`.
- Already-guaranteed properties make this nearly free: structured I/O on every node ⇒ every edge is serializable ⇒ any edge can become a network boundary without semantic change; shared checkpointer ⇒ one durable execution history across instances.
- Isolation is also a **security** feature (per-agent sandboxing, credentials, blast-radius containment for tool-wielding agents) — a declarative story no current framework has.
- v0: `placements` is parsed and validated but **no-op**.

### 5.9 Invocation & triggers — how executions come to exist

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
- **Codegen**: `manual` → the `run` CLI; `http` → a generated FastAPI app wrapping the compiled graph (start / resume / status routes), reusing the same generated Pydantic models for payload validation. Auth on `http` triggers is deliberately out of scope for v0 (deploy behind your own gateway); a declarative auth story belongs with `placements` in M3.

### 5.10 Compiler contract

- **Generated code is a build artifact, never hand-edited.** The DSL is the single source of truth.
- **Deterministic codegen**: same DSL → byte-identical output (stable ordering), so regeneration diffs are meaningful.
- **Eject path**: for graphs that outgrow the DSL, copy generated code out and own it (CRA model).
- **Pinned backend versions**: each compiler release targets a pinned LangGraph version; upgrades are explicit and versioned, like Terraform providers. DSL semantics must never drift silently with upstream API churn.

## 6. Example (illustrative sketch, not final grammar)

```yaml
# main.yml
version: "0.1"
imports:
  - agents/researcher.yml
  - agents/reviewer.yml
  - tools/web_search.yml
  - flows/review_loop.yml
  - triggers.yml            # see 5.9 — entrypoints are the flows triggers target

state:
  draft: { type: string }
  feedback: { type: string }

# agents/reviewer.yml
agent.reviewer:
  model: { provider: anthropic, id: claude-sonnet-4-6 }
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
- Grammar spec + published JSON Schema (editor autocomplete/validation for free). Grammar includes `human` nodes, `writes:` remaps, `context: inherit`, `triggers` (manual/http active; schedule/event reserved), and `placements` (parsed; no-op).
- Parser → resolver (imports, address scheme) → flat IR.
- Static checks: reference/type resolution, schema compatibility across edges, routing exhaustiveness (edges and map variants), SCC cycle-termination, fan-out bounding, reducer-channel write rules inside maps, trigger input-binding compatibility, sync-trigger interrupt-free reachability, unreachable nodes, undefined state channels.
- `agent-compose validate` with precise, actionable error messages. Error UX is a feature, not polish — it is the coding-agent feedback loop.

**M1 — Codegen (single process)**
- IR → deterministic LangGraph Python: state models (incl. tagged unions), node fns, routers with embedded CEL, bounded cycles, homogeneous + discriminator-routed `map`→`Send` with index-tagged reducers, subgraphs, retry/timeout policy.
- `agent-compose build`, `agent-compose run` (manual trigger), `agent-compose serve` (generated FastAPI app for http triggers: start/resume/status), golden-file codegen tests.

**M2 — Ergonomics**
- `agent-compose plan` (topology + validation diff between two specs).
- Tracing conventions in generated code (routing decisions as trace data).
- `eject` command.

**M3 — Distribution (design-gated)**
- `--target distributed` via `RemoteGraph`; execute `placements`.
- Postgres checkpointer wiring; isolation/credential story per placement.
- Execute `schedule` and `event` triggers; declarative auth for `http` triggers.

## 8. Risks

| Risk | Mitigation |
|---|---|
| LangGraph API churn breaks codegen | Pin backend version per release; explicit versioned upgrades; golden-file tests per pinned version |
| DSL expressiveness ceiling → users bypass it | `function` escape hatch + `eject`; treat repeated ejects as grammar feedback |
| Hand-edited generated code forks source of truth | Generated-file headers, `build --check` in CI, docs discipline |
| CEL↔Python semantic drift | Embed cel-python (no transpilation) in v0 |
| Scope creep toward bespoke runtime | Hard non-goal; LangGraph owns execution |

## 9. Resolved Questions (log)

Formerly open, now settled — rationale lives in the referenced sections:

1. **Data edges** → deferred; per-node `writes:` remap ships in v0 (5.7).
2. **Conversation-history scoping** → isolated by default; opt-in `context: inherit` (5.7).
3. **Tool vs function** → separate keywords, unified definitions (5.5).
4. **Human-in-the-loop** → `human` node type in v0 grammar; runtime may land M2 (5.5).
5. **Spec versioning** → required `version:` field; semver on the spec; compiler declares a supported range; breaking changes gated behind a major bump + `agent-compose migrate` codemod; old syntax is never silently reinterpreted — refuse and point at the migration. Pre-1.0, minor bumps may break with a migration provided.
6. **`detach` under durable execution** → idempotency-key design, v0 restriction (5.6).
7. **`event` backend abstraction** → pluggable consumer interface, backend-agnostic grammar via `event_sources:`, at-least-once + inbound dedupe, one blessed reference backend in M3 (5.9).
8. **`respond: sync` semantics** → compile-time interrupt-free requirement + mandatory timeout with async upgrade (202 + execution id) (5.9).

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
