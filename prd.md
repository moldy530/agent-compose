# agent-compose — Product Requirements Document

**Status:** Draft v0.1
**Author:** moldy
**Last updated:** 2026-08-13

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
| `map` | fan-out over a collection | LangGraph `Send` API |

Temporal-inspired discipline: the compiled graph is the *workflow* (deterministic, replayable); nodes are *activities* (effectful, retryable). Per-node `retry` / `timeout` / `on_error` policy is declared in YAML with duckflux's resolution chain: flow override > node > defaults > fail. Strategies: `fail`, `skip`, `retry` (exponential backoff), `fallback: <node ref>`.

The spec never contains executable code (Agent Spec's security posture). Code logic lives in three layers:
1. **CEL expressions** for conditions and mappings — non-Turing-complete, sandboxed, type-checked at parse time.
2. **First-class control constructs** in YAML: guarded edges, bounded loops, `map` fan-out, `when` guards.
3. **Escape hatches**: `exec`, `http`, `function` — implementation outside the spec, referenced by identifier.

CEL implementation choice (v0): embed `cel-python` in generated routers rather than transpiling CEL→Python. Semantic fidelity between `validate` and runtime beats zero-dep purity; transpilation is a later optimization.

### 5.6 State — three tiers

1. **Node-scoped I/O**: typed inputs/outputs per node.
2. **Shared graph state**: a `state:` section generates the LangGraph `State` schema (TypedDict/Pydantic) with per-channel reducers. Default wiring is name-based (outputs write channels of the same name — Agent Spec's optional-data-edge insight); explicit data edges available for complex graphs. Subgraphs receive parent state only through explicit bindings (PayPal's `passVariables` discipline).
3. **Conversation history**: an implicit append channel available to agent nodes.

Persistence (`storage:` section) targets LangGraph checkpointers (Postgres for durable/distributed). Novelty opportunity: the literature consistently punts on cross-session memory and datastores — this is a differentiation area post-v0.

### 5.7 Cloud & distribution — placement annotations, not distributed edges

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

### 5.8 Compiler contract

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

entrypoint: flow.review_loop

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
- Grammar spec + published JSON Schema (editor autocomplete/validation for free).
- Parser → resolver (imports, address scheme) → flat IR.
- Static checks: reference/type resolution, schema compatibility across edges, routing exhaustiveness, SCC cycle-termination, unreachable nodes, undefined state channels.
- `agent-compose validate` with precise, actionable error messages. Error UX is a feature, not polish — it is the coding-agent feedback loop.

**M1 — Codegen (single process)**
- IR → deterministic LangGraph Python: state models, node fns, routers with embedded CEL, bounded cycles, `map`→`Send`, subgraphs, retry/timeout policy.
- `agent-compose build`, `agent-compose run` (thin wrapper), golden-file codegen tests.

**M2 — Ergonomics**
- `agent-compose plan` (topology + validation diff between two specs).
- Tracing conventions in generated code (routing decisions as trace data).
- `eject` command.

**M3 — Distribution (design-gated)**
- `--target distributed` via `RemoteGraph`; execute `placements`.
- Postgres checkpointer wiring; isolation/credential story per placement.

## 8. Risks

| Risk | Mitigation |
|---|---|
| LangGraph API churn breaks codegen | Pin backend version per release; explicit versioned upgrades; golden-file tests per pinned version |
| DSL expressiveness ceiling → users bypass it | `function` escape hatch + `eject`; treat repeated ejects as grammar feedback |
| Hand-edited generated code forks source of truth | Generated-file headers, `build --check` in CI, docs discipline |
| CEL↔Python semantic drift | Embed cel-python (no transpilation) in v0 |
| Scope creep toward bespoke runtime | Hard non-goal; LangGraph owns execution |

## 9. Open Questions

1. Data-edge syntax beyond name-based state wiring — needed in v0 or defer?
2. Conversation-history scoping across subgraph boundaries (inherit, isolate, or explicit binding?).
3. Tool invocation taxonomy surface: expose PayPal's tool (LLM-invoked) vs function (graph-invoked) distinction as separate keywords, or unify with an `invoked_by` attribute?
4. Human-in-the-loop nodes (LangGraph interrupts) — v0 grammar or M2?
5. Spec versioning/migration policy for the DSL itself.

## 10. References

- Oracle, *Open Agent Specification* — arXiv 2510.04173
- Daunis (PayPal), *A Declarative Language for Building And Orchestrating LLM-Powered Agent Workflows* — arXiv 2512.19769
- ADL — arXiv 2504.14787; SPL — arXiv 2607.07727
- Zhuge et al., *GPTSwarm: Language Agents as Optimizable Graphs* — arXiv 2402.16823
- Qian et al., *MacNet: Scaling LLM-based Multi-Agent Collaboration* — arXiv 2406.07155
- Yue et al., *From Static Templates to Dynamic Runtime Graphs* (survey) — arXiv 2603.22386
- Anthropic, *Building Effective Agents*
- duckflux spec — github.com/duckflux/spec
- LangGraph docs — docs.langchain.com
