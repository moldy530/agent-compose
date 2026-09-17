# agent-compose — Graph Document

**Graph version:** `2`
**Status:** Normative for the document `agent-compose visualize` emits
**Companion artifacts:** [`docs/grammar.md`](grammar.md) (the DSL this describes compositions of), [`docs/trace.md`](trace.md) (the same discipline over a run), [`prd.md`](../prd.md) §5.5, §5.6, §5.7, §5.8, §5.9, resolved q56

`agent-compose visualize <path>` renders a validated composition's flows as one
self-contained HTML page. What that page draws is a **graph document**: a JSON
surface derived from the resolved IR, carrying every node, every routing
decision an execution could take, and the resolved configuration behind each
one. `--format json` prints exactly that document, so anything else that wants
a picture of a composition reads the same bytes the picture does.

This document defines it: what every field means, and what a reader may rely on
across compiler releases.

**Conformance language.** MUST / MUST NOT / REQUIRED / SHOULD / MAY are used in
the RFC 2119 sense.

**Where the fields are implemented.** Every record type here is a Rust struct in
`crates/compose-core/src/graph/document.rs`, and
`crates/compose-core/tests/graph_format_inventory.rs` holds the two together:
the top-level keys the compiler writes and the fields §2 names are the same set,
and every record type below is named in the section that specifies it.

**Presence vocabulary.** Every field table has a *presence* column, written in
the same small vocabulary [`docs/trace.md`](trace.md) fixes, because §9.1 makes
a reader entitled to both halves of what such a cell says — the cases a field
appears in, and therefore the cases it does not.

| in a presence column | what it says |
|---|---|
| **always** | the key is on every record of this type, without exception |
| a **condition** | the key is on exactly the records the condition describes, and on no others |
| **, possibly empty** | a qualifier on either of the above: where the key is present, the array it holds may have no elements |

A presence cell without the qualifier promises a value with something in it. An
absent key is never spelled as an empty array, and an empty array is never
spelled as absence.

**Every leaf is the text the author wrote.** A duration is `"90s"`, an address
is `"agent.triage"`, an expression is `"state.patches"` — the IR's own spelling,
not a re-rendering of it. The exceptions are the integers the grammar declares
as integers (`max_items`, `max_concurrency`, a retry's `max`) and a `settings:`
value, which is an arbitrary literal and reaches JSON as JSON.

---

## Table of contents

1. [What it is derived from](#1-what-it-is-derived-from)
2. [The envelope](#2-the-envelope)
3. [A flow](#3-a-flow)
4. [Schemas and bindings](#4-schemas-and-bindings)
5. [Nodes](#5-nodes)
6. [Edges](#6-edges)
7. [Fan-out](#7-fan-out)
8. [Agents](#8-agents)
9. [Stability](#9-stability)
10. [What is not part of this format](#10-what-is-not-part-of-this-format)

---

## 1. What it is derived from

The **resolved IR** (PRD 5.1), for a composition the validator accepted.
`visualize` validates first and emits only on a clean report, exactly as `build`
does, so every document this format describes is a document about a graph the
compiler proved runnable.

The IR is deliberately *faithful rather than convenient*: it materializes no
default, resolves no policy chain, and leaves a model as an address. This
document is the opposite — it is read by a renderer, so the compiler answers the
questions first:

| question | where the answer is | which rule decides it |
|---|---|---|
| what is this node's `timeout:` | [`policy`](#5-nodes) | grammar §9.3's resolution chain, levels 2–4 |
| which provider does this agent reach | [`agent.model`](#8-agents) | grammar §12.2 |
| what tools are on the wire | [`agent.tools`](#8-agents) | grammar §5.5, §11.5 |
| which variants does `default:` catch | [`map.routes[].covers`](#7-fan-out) | grammar §8.6 rule 4 |
| where can control go from here | [`edges`](#6-edges) | grammar §7.2, §7.3, §7.8 |

**One resolution is deliberately not made.** Grammar §9.3's level 1 — a `flow:`
node's `policy:` — belongs to the *instantiation site*, and one flow may be
instantiated from several with different overrides, so the nodes inside an
instance have no single answer. The override is reported on the instantiating
node instead ([`subflow.policy`](#5-nodes)), and a node's own `policy` resolves
levels 2 through 4 with the level named.

**Nothing is looked up at run time and nothing is substituted.** An `${ENV}`
reference reaches this document exactly as the author wrote it, because the IR
never resolves one (PRD 5.9, resolved q15): a picture of a composition is a file
people paste into pull requests.

---

## 2. The envelope

`GraphDocument`.

| field | type | presence | meaning |
|---|---|---|---|
| `graph_version` | integer | always | The format this document is written in. `2` is this document. It is the **first** key, so a consumer can dispatch on it before reading anything else. See [Stability](#9-stability). |
| `entrypoint` | string | always | The spec entrypoint, relative to the project root — the IR's own `entrypoint` (grammar §1.4). |
| `target` | string | always | The deploy target the composition was resolved for. `visualize` takes no `--target`, so it is the built-in `local` (grammar §14); the key is written all the same, because a reader of the document should not have to know which verb produced it. |
| `spec_version` | string | always | The DSL version the composition declares (grammar §1.3). |
| `flows` | array of [flows](#3-a-flow) | always, possibly empty | One entry per `flow.*` the document covers, **sorted by address**. Every flow of the composition, or the one `--flow` named. Empty only for a composition that declares no flow at all. |

---

## 3. A flow

`FlowGraph`. One canvas.

| field | type | presence | meaning |
|---|---|---|---|
| `address` | string | always | The flow's typed address, `flow.<name>` (grammar §2.2). |
| `description` | string | when the flow declares one | `description:` — required only where the flow is used as an agent tool (grammar §5.4, Decision D26). |
| `inputs` | array of [fields](#4-schemas-and-bindings) | always, possibly empty | `inputs:` in declaration order. Empty where the flow declares none. |
| `outputs` | array of [fields](#4-schemas-and-bindings) | always, possibly empty | `outputs:` in declaration order (grammar §7.5). |
| `triggers` | array of [triggers](#31-a-trigger) | always, possibly empty | Every **declared** trigger targeting this flow, in trigger-name order (grammar §13, Decision D64). Implicit `manual` invocation contributes no entry: every flow is runnable from the CLI whether or not a trigger names it. |
| `nodes` | array of [nodes](#5-nodes) | always | The flow's own nodes in declaration order, the two pseudo-nodes, and one satellite per `map` route. See §5. |
| `edges` | array of [edges](#6-edges) | always | Every transfer an execution could take. See §6. |

### 3.1 A trigger

`TriggerView`. The delivery surface, summarized — `docs/grammar.md` §13 is where
a trigger is specified in full.

| field | type | presence | meaning |
|---|---|---|---|
| `name` | string | always | The trigger's name. |
| `type` | `"manual"` \| `"http"` \| `"schedule"` \| `"event"` | always | Which of grammar §13's four it is. `schedule` and `event` are reserved grammar: parsed and carried, executing as no-ops in v0 (grammar §15). |
| `summary` | string | always | The one line that says how an execution arrives — `POST /reports · respond async`, `cron 0 3 * * * (UTC) · reserved in v0`. Written for a person; §9.1 makes it text a reader MUST NOT parse. |
| `session_key` | string | when the trigger declares one | The CEL supplying session identity, which session-scoped stores and history key off (grammar §11.3). |
| `description` | string | when the trigger declares one | `description:`. |
| `input` | array of [bindings](#4-schemas-and-bindings) | when the trigger declares any | `input:` — flow input field to CEL over `payload`, in declaration order. |

---

## 4. Schemas and bindings

Four small records the rest of the document is built from.

`FieldView` — one field of a declared schema.

| field | type | presence | meaning |
|---|---|---|---|
| `name` | string | always | The field name. |
| `type` | string | always | The type, summarized as one line: the shape (`string`, `array<string>`, `enum [a, b]`, `object { a, b }`, `union on kind [one, two]`) followed by every constraint the node declares, in grammar §3's order. Written for a person; §9.1 makes it text a reader MUST NOT parse — the declaration itself lives in the composition. |
| `description` | string | when the field declares one | `description:`. |

`SchemaView` — one declared schema.

| field | type | presence | meaning |
|---|---|---|---|
| `source` | `"declared"` \| `"kind_default"` \| `"string_input"` | always | Where the shape comes from: the composition, the construct's kind (an inline `exec:`'s `{ exit_code, stdout }`, an inline `http:`'s `{ status, body }`, a store op's derived result — grammar §8.2, §8.3, §11.4), or an agent that declares no `input:` and so takes one unnamed string (grammar §5.3). |
| `fields` | array of [fields](#4-schemas-and-bindings) | always, possibly empty | The fields, in declaration order. Empty on `"string_input"`, which has no named field, and on a schema declared `{}`. |

`BindingView` — one name bound to one expression.

| field | type | presence | meaning |
|---|---|---|---|
| `name` | string | always | The bound name: an input field, a header, a query parameter, an environment variable, or a dotted path into a store op's parameters (`value.last_report`). |
| `expression` | string | always | What is bound to it, as written — CEL where the surface is CEL (grammar §4.1), interpolated text where it is class 2 (grammar §4.3). |

`WriteView` — one `writes:` remap (grammar §10.1, Decision D16).

| field | type | presence | meaning |
|---|---|---|---|
| `field` | string | always | An output field of the node. |
| `channel` | string | always | The state channel it is written to. |
| `reduce` | `"append"` \| `"merge"` \| `"last_wins"` | when the channel declares one | The channel's reduce policy — which is what makes a write from inside a `map` legal at all (grammar §10.2, PRD 5.6). |

---

## 5. Nodes

`GraphNode`. One record per drawn node, and there are three sorts:

* **a node of the flow**, one per `nodes:` entry, in declaration order;
* **a pseudo-node**, `start` and `end`, which a flow's edges name (grammar
  §7.2). Both are always present;
* **a satellite**, one per route of a `map`. The dispatch target is where an
  item's work happens, so it is drawn: a fan-out with nothing to fan out to is
  not a picture of a fan-out. A satellite's `id` is `<map id>/<variant>` for a
  named route, and `<map id>/(default)` or `<map id>/(item)` for the two routes
  that carry no variant tag. Neither `/` nor a parenthesis is legal in an
  identifier (grammar §2.1), so a satellite collides with nothing an author
  wrote — and, because the two tagless spellings are parenthesized, with no
  sibling satellite either: a union may declare a variant tagged `default`,
  which is a legal tag and a sibling key of `routes:` rather than a member of
  it (grammar §8.6).

| field | type | presence | meaning |
|---|---|---|---|
| `id` | string | always | The node's id on this canvas: a flow-local node id (grammar §2.4), `start`, `end`, or a satellite's derived id. Unique within the flow. |
| `kind` | see below | always | Which kind this node is. |
| `binding` | string | on every node but the two pseudo-nodes, and on a `human` node only when it declares a `timeout:` | The one line the node is read by under its id: a typed address, an inline block's command or request line, a store op, or a map's `over:`. Written for a person (§9.1). |
| `description` | string | when the node or the definition it names declares one | The node's own `description:`, or the definition's where the node declares none. |
| `input` | [input](#51-input-bindings) | when the node, or on a satellite the route that dispatches it, declares `input:` | The node's bindings (grammar §8.0). |
| `writes` | array of [writes](#4-schemas-and-bindings) | when the node, or on a satellite the route that dispatches it, remaps at least one output field | The write remap, in declaration order. |
| `policy` | [policy](#52-policy) | on every node of the flow; absent on the pseudo-nodes and on satellites | Grammar §9.3's chain, resolved. A satellite carries none because the dispatch has no policy of its own: what governs it is the map's `on_item_error` and the map node's own `policy` (grammar §8.6 rules 9, 10). |
| `schemas` | [schemas](#53-schemas) | when the node has at least one of the two | What the node is handed and what it answers with. Absent on the pseudo-nodes, and on a `map`, which declares no surface of its own. |
| `agent` | [agent](#8-agents) | `agent` nodes, and satellites whose target is an `agent.*` | The resolved agent: model, tools, prompt. |
| `coder` | [coder](#58-a-harness-run) | `coder` nodes | The harness run: which harness, what contains it, and the resolved model (grammar §8.9). |
| `tool` | [tool](#81-a-tool) | `function` nodes, and satellites whose target is a `tool.*` | The tool this node invokes (grammar §6). |
| `exec` | [exec](#54-exec-and-http) | nodes bound to an `exec:` block, inline or through a `tool.*` | The subprocess step (grammar §6.1, §8.2). |
| `http` | [http](#54-exec-and-http) | nodes bound to an `http:` block, inline or through a `tool.*` | The request (grammar §6.1, §8.3). |
| `store` | [store](#55-a-store-operation) | `store` nodes | The store operation (grammar §8.8, §11.4). |
| `subflow` | [subflow](#56-a-subflow-instantiation) | `flow` nodes, and satellites whose target is a `flow.*` | The instantiation, and the canvas a jump link opens. |
| `human` | [human](#57-a-wait) | `human` nodes | The wait (grammar §8.7). |
| `map` | [map](#7-fan-out) | `map` nodes | The fan-out (grammar §8.6). |
| `dispatch` | [dispatch](#71-a-satellite) | satellites | Which map dispatched this node, and on what. |

`kind` is one of `"start"`, `"end"`, `"agent"`, `"coder"`, `"function"`,
`"exec"`, `"http"`, `"human"`, `"store"`, `"flow"`, `"map"` — grammar §7.1's
nine, plus the two pseudo-nodes. Every one of them MUST render distinguishably,
which is the content requirement PRD resolved q56 states. A satellite takes the
kind of its **target**: a `tool.*` dispatched by a map is invoked the way a
`function:` node invokes one, so it is `"function"`. No satellite is ever
`"coder"`: a `map` dispatches to a **component** (grammar §8.6, Decision D29),
and a coder node is declared in a flow rather than defined.

### 5.1 Input bindings

`InputView`, tagged by `form` (grammar §8.0, Decision D88).

| field | type | presence | meaning |
|---|---|---|---|
| `form` | `"scalar"` \| `"fields"` | always | Which of the two forms was written. |
| `value` | string | `"scalar"` | The one unnamed value's expression. |
| `bindings` | array of [bindings](#4-schemas-and-bindings) | `"fields"`, possibly empty | The per-field bindings, in declaration order. |

### 5.2 Policy

`PolicyView`. Grammar §9.3's chain, resolved at levels 2 through 4, with the
level each field came from carried beside it — because "this node times out
after ninety seconds" and "this node declares a ninety-second timeout" are
different facts and a reader of a picture needs the first without losing the
second.

| field | type | presence | meaning |
|---|---|---|---|
| `retry` | `RetryView` | when the chain resolves to one | `max`, `backoff`, and the optional `multiplier`, `max_backoff`, `jitter` a block declares — plus `level`. |
| `timeout` | `TimeoutView` | when the chain resolves to one | `value`, plus `level`. |
| `on_error` | `OnErrorView` | always | `strategy` — `"fail"`, `"skip"` or `"fallback"` — plus `target` on a fallback (a node id, or `end`), plus `level`. Always present: the built-in is `fail` (grammar §9.2). |

`level` is `"node"`, `"defaults"` or `"built_in"` — the chain's levels 2, 3
and 4. The chain has a fourth outcome, the exemption Decision D102 gives a
`human` node's `timeout:` and `retry:` at every level, and it is **not** a
`level` a reader ever meets: an exempt field resolves to no value, so this
document writes the exemption as the **absence** of the `retry` or `timeout`
key. `examples/triage-fanout`'s `approve` is the worked case — its `policy`
carries `on_error` alone — and [`HumanView`](#57-a-wait) is where the wait's own
budget is written down instead.

### 5.3 Schemas

`SchemasView`.

| field | type | presence | meaning |
|---|---|---|---|
| `input` | [schema](#4-schemas-and-bindings) | when the construct has a declared input surface — `agent`, `coder`, `function`, `flow` and `human` nodes, and satellites | What the node is handed. Absent on the inline blocks, which bind an object rather than declaring a surface. A `coder` node's carries `source: "string_input"` where its block declares no `input:`, exactly as a string-in agent's does (grammar §8.9, §5.3). |
| `output` | [schema](#4-schemas-and-bindings) | on every kind but `map` | What the node answers with. |

### 5.4 `exec` and `http`

`ExecView` — `command`, `args` (in argv order), `cwd`, `env` (as
[bindings](#4-schemas-and-bindings)), `expect_exit`. Every key but `command` is
present only where the block declares it, and the arrays are omitted rather than
empty.

`HttpView` — `method`, `url`, `headers`, `query`, `body` (all as
[bindings](#4-schemas-and-bindings)), `expect_status`. `method` and `url` are
always present; the rest follow the same rule.

### 5.5 A store operation

`StoreView`.

| field | type | presence | meaning |
|---|---|---|---|
| `address` | string | always | The store's typed address. |
| `kind` | `"kv"` \| `"vector"` \| `"blob"` | always | The store's kind (grammar §11.1). |
| `scope` | `"execution"` \| `"session"` \| `"global"` | always | Its lifetime (Decision D35). |
| `op` | string | always | The operation this node performs (grammar §11.4). |
| `backend` | string | when the store declares one | The abstract alias the active target resolves (grammar §11.3). |
| `params` | array of [bindings](#4-schemas-and-bindings) | when the operation carries at least one parameter, which grammar §11.4 makes every one of them do | The operation's parameters in §11.4's own order, with a `value:`/`filter:`/`metadata:` field map flattened to dotted names. |

### 5.6 A subflow instantiation

`SubflowView` (grammar §8.5).

| field | type | presence | meaning |
|---|---|---|---|
| `address` | string | always | The flow this node instantiates — the canvas a jump link opens. A subgraph is **never** expanded inline: the module boundary is a fact of the composition and the picture honours it (PRD 5.7, resolved q56). |
| `context` | `"isolated"` \| `"inherit"` | when the node declares it | Absent means the default, `isolated`. |
| `policy` | `InstancePolicyView` | when the node declares a `policy:` | Grammar §9.3's **level 1**, as written rather than resolved: `timeout`, `on_error` (`fail` or `skip` — level 1 takes no `fallback`, Decision D103) and `retry`. It applies to the nodes *inside* the instance (Decision D60). |

### 5.7 A wait

`HumanView` (grammar §8.7).

| field | type | presence | meaning |
|---|---|---|---|
| `timeout` | string | when the block declares one | The wait's budget. Absent means the wait is unbounded. |
| `on_timeout` | string | when the block declares one | Where control transfers on expiry: a node id, or `end`. Jointly optional and jointly required with `timeout`, and drawn as a [`timeout` edge](#6-edges). |

---

### 5.8 A harness run

`CoderView` (grammar §8.9, PRD resolved q57).

**The harness is named**, which is PRD resolved q56's content requirement read
for this kind. Two coder nodes side by side are two different agent loops with
two different containment stories, and a picture that drew them identically would
be hiding the one fact a reader most needs.

| field | type | presence | meaning |
|---|---|---|---|
| `harness` | `"cc"` \| `"codex"` | always | Which harness runs it. The two reserved names of grammar §15 are not members: `visualize` validates first, and `validate` refuses a composition that binds one. |
| `model` | [model](#8-agents) | always | The `model.*` resolved exactly as an agent's is. It is always the **direct** form here: a route is refused at this position (grammar §12.2, Decision D141). |
| `workspace` | string | always | `workspace:`, exactly as written — an `${ENV}` reference reaches this document unresolved like every other (§10). |
| `access` | `"read_only"` \| `"workspace_write"` \| `"full_access"` | always | The containment preset, **with the default materialized**: a node that omits `access:` reads `"workspace_write"` here. This document answers the question rather than leaving it (§1), and it is a containment claim, so a reader should not have to know which way the grammar's default falls. |
| `prompt` | string | always | The run's instructions, verbatim. |
| `allow_tools` | array of strings | when the node declares any | The harness tool names the run may use, in declaration order. Absent means the harness's own default set. |
| `tools_enforced` | boolean | always | Whether this harness enforces `allow_tools` **inside its own loop** rather than bounding at its sandbox alone. It is a fact about the *harness* and is in no composition, which is why it is here: two nodes with identical lists are not under identical bounds, and PRD resolved q57 ruling c makes stating that this document's job. |
| `env` | array of [bindings](#4-schemas-and-bindings) | when the node declares any | `env:`, in declaration order, values as written. Absent means a wholly scrubbed child environment. |
| `inherit_env` | boolean | always | `inherit_env:`, with the default materialized for `access`'s reason: `false` is the containment claim. |
| `settings` | array of [settings](#8-agents) | when the node declares any | The harness config **in declaration order**, each value reaching JSON as JSON — the same treatment a model's `settings:` get, in a different order: a model's reach this document sorted by key, and this block's are held as written (§9.1). |

---

## 6. Edges

`GraphEdge`. **Every routing decision an execution could take is an edge here**,
which is the promise the picture makes: what you see is every path the validator
proved (PRD resolved q56). That is more than the flow's `edges:` list — grammar
§7.8 counts two control-transfer positions alongside edges when it decides
reachability, and both of them are drawn.

| field | type | presence | meaning |
|---|---|---|---|
| `from` | string | always | The node id control leaves — a node of this flow, or `start`. |
| `to` | string | always | The node id it arrives at — a node of this flow, a satellite, or `end`. |
| `class` | see below | always | Which kind of transfer this is. |
| `label` | string | when the class has one | The chip a canvas draws: the guard's CEL on a conditional edge, `else` on a default one, the variant on a map route — `(default)` on the catch-all and `each item` on a homogeneous fan-out, both parenthesized or spaced so that no variant tag can spell them — `on_error: fallback`, `on_timeout (24h)`. An unconditional declared edge has none. |
| `when` | string | `"conditional"` | The CEL guard, as written (grammar §7.3). |
| `max_iterations` | integer | when the edge declares one | The cycle bound a guarded back-edge carries (grammar §7.4, Decision D90). |
| `join_barrier` | boolean | when true | Present only where it is `true`: this edge leaves a `map` node, and PRD 5.6 makes the downstream edge the barrier — it fires when every dispatched instance has completed, been resolved by `on_item_error`, or been detached (grammar §8.6 rule 6). A `detach: true` route is resolved *at dispatch*, so the barrier never waits on it (Decision D94). |

`class` is one of:

| class | what it is |
|---|---|
| `unconditional` | a declared edge with no `when:` and no `else:` (grammar §7.2) |
| `conditional` | a declared edge carrying a CEL `when:` guard (grammar §7.3) |
| `default` | a declared edge marked `else: true`, taken iff no guarded sibling was (grammar §7.3 rule 4, Decision D61) |
| `map_route` | a `map`'s dispatch to one of its routes (grammar §8.6) |
| `error_fallback` | `on_error: { fallback: … }` — control transfers instead of this node's own edges being evaluated (grammar §9.2, §7.8) |
| `timeout` | `human.on_timeout:` — control transfers when the wait runs out (grammar §8.7, §7.8) |

**Order.** The declared edges come first, in declaration order — which is
semantic, because grammar §7.3 evaluates a node's outgoing edges in it. The
derived edges follow, grouped by the node they leave, in that node's declaration
order: a node's map routes, then its `error_fallback`, then its `timeout`.

**Concurrency and convergence are topology, never a hint.** Two unconditional
edges leaving one node are two branches that run in one step (grammar §7.6.1),
and a node two branches arrive at is where they converge (grammar §7.6.2). This
format records neither as a flag: both fall out of the edges, which is what
makes the picture a picture of the composition rather than of somebody's reading
of it.

---

## 7. Fan-out

`MapView`, on a `map` node (grammar §8.6, PRD 5.6).

| field | type | presence | meaning |
|---|---|---|---|
| `over` | string | always | The path expression the fan-out reads (grammar §4.2, Decision D43). |
| `max_items` | integer | when the array it resolves to declares one | The cardinality half of mandatory bounding (Decision D10). A validated composition always declares it; the key is conditional because resolving the path is the compiler's own walk and a root it cannot follow leaves nothing to report. |
| `item_binding` | string | always | `as:` — the per-item binding name; `"item"` where the map declares none. |
| `max_concurrency` | integer | always | The node-wide bound (Decision D28) — the execution half of mandatory bounding. |
| `dispatch` | `"homogeneous"` \| `"routed"` | always | Which of grammar §8.6 rule 2's two forms. |
| `route_by` | string | `"routed"` | The literal discriminator field, which equals the item union's own (rule 8). |
| `on_item_error` | `ItemErrorView` | when the map declares it | `strategy` — `"fail"`, `"skip"` or `"retry"` — plus the `retry` block a `retry` carries inline (rule 10, Decision D73). Absent means the default, `fail`. |
| `join` | string | always | What the barrier waits on, in one sentence. Written for a person (§9.1); the machine-readable half is [`join_barrier`](#6-edges) on the map's outgoing edges. |
| `routes` | array of `RouteView` | always | Every destination: the named routes in declaration order, then `default:`. A homogeneous map has exactly one, whose `variant` is absent and whose `default` is `false`. |

`RouteView`:

| field | type | presence | meaning |
|---|---|---|---|
| `node` | string | always | The satellite's id on this canvas — the `to` of the `map_route` edge. |
| `variant` | string | named routes only | The discriminator variant this route serves. |
| `default` | boolean | always | Whether this is the `default:` route. |
| `covers` | array of strings | when the route narrows at least one variant | Which union variants this route's target is **narrowed** to: its own tag, or, for `default:`, every variant no named route claims (grammar §8.6 rule 4, Decision D30). Absent on a homogeneous map, whose item type is not a union, and on a `default:` that catches nothing left over. |
| `target` | string | always | The dispatch target's typed address: `agent.*`, `tool.*`, or `flow.*`. |
| `max_concurrency` | integer | when the route declares one | This route's own bound, at most the map's. |
| `input` | [input](#51-input-bindings) | when the route declares `input:` | Per-item bindings; absent means the whole item. |
| `writes` | array of [writes](#4-schemas-and-bindings) | when the route declares `writes:` | The write remap, whose targets must be reduced channels (rule 5). |
| `detach` | boolean | when the route declares it | Fire-and-forget dispatch (rule 7). A detached route is outside the join. |

### 7.1 A satellite

`DispatchView`, on the node a route dispatches to.

| field | type | presence | meaning |
|---|---|---|---|
| `map` | string | always | The map node's id. |
| `variant` | string | named routes only | The variant this satellite serves. |
| `default` | boolean | always | Whether it is the `default:` route's target. |
| `covers` | array of strings | when the route narrows at least one variant | The variants it is narrowed to — the same set `RouteView.covers` carries. |
| `max_concurrency` | integer | when the route declares one | This route's own bound. |
| `detach` | boolean | when the route declares it | Fire-and-forget dispatch. |

---

## 8. Agents

`AgentView`, on an `agent` node and on a satellite dispatching an `agent.*`.

| field | type | presence | meaning |
|---|---|---|---|
| `address` | string | always | The agent's typed address. |
| `model` | `ModelView` | always | The model, resolved — see below. |
| `tools` | array of [tools](#81-a-tool) | always, possibly empty | **Every tool on the wire**, in one list and in this order: the `tool.*` and `flow.*` entries of `tools:` in declaration order, then the `builtin.*` shorthand entries, then the tools an attached store synthesizes (grammar §5.5, §11.5, PRD 5.8). Empty where the agent attaches none. |
| `stores` | array of strings | when the agent attaches any | `stores:` — the attached stores' addresses, in declaration order. |
| `prompt` | string | always | `prompt:`, verbatim (Decision D13). |
| `max_tool_iterations` | integer | when the agent declares it | Absent means the default, `8` (Decision D51). |

`ModelView` resolves one `model.*` through to its provider (grammar §12.2):

| field | type | presence | meaning |
|---|---|---|---|
| `address` | string | always | The model's typed address. |
| `form` | `"direct"` \| `"route"` | always | Which of Decision D39's two a model is. |
| `description` | string | when the definition declares one | `description:`. |
| `id` | string | `"direct"` | The provider-native model id. |
| `provider` | `ProviderView` | `"direct"` | The connection: its `address`, its `kind`, its declared connection keys as `config` (sorted by key, every `${ENV}` reference as written), and the `server_tools` types it appends to every request (Decision D122). |
| `settings` | array of `SettingView` | `"direct"`, when the model declares any | `settings:` sorted by key, each `{ name, value }` with the value as JSON (Decision D40). |
| `route` | array of `ModelView` | `"route"` | The failover members **in failover order**, each resolved the same way. |
| `route_on` | array of strings | `"route"`, when the definition declares them | The infrastructure conditions failover happens on. Absent means the default, `[rate_limit, overloaded, timeout]`. |

### 8.1 A tool

`ToolView`. One record whether the tool is on an agent's wire or invoked as a
`function:` node — the def/use split is the composition's, and a picture of
either shows the same definition (PRD 5.5).

| field | type | presence | meaning |
|---|---|---|---|
| `name` | string | always | The name the model calls it by: a `tool.*`'s local name, a built-in's provider-dictated name (PRD resolved q54 ruling d), or a synthesized store tool's `<store>_<op>` (grammar §11.5). |
| `source` | `"tool"` \| `"flow"` \| `"builtin"` \| `"store"` | always | Where the tool came from: a `tool.*`, a `flow.*` used as a tool (PRD resolved q19), the `builtin.*` shorthand, or an attached store. |
| `address` | string | always | The typed address it was attached by. |
| `binding` | `"exec"` \| `"http"` \| `"function"` \| `"module"` \| `"builtin"` | `"tool"` and `"builtin"` sources | Which implementation a `tool.*` is bound to (grammar §6.1). |
| `op` | string | `"store"` | The store op a synthesized tool performs. |
| `description` | string | when there is one | The LLM's selection signal — a tool's own, a flow's, a built-in's, or the store's. |
| `detail` | string | when there is one | The one line the implementation is read by. Written for a person (§9.1). |

---

## 9. Stability

`graph_version` is what a reader pins. This section is what pinning it buys.

### 9.1 What a reader may rely on

At a given `graph_version`, a reader MAY rely on:

* every field this document names, under the name and with the meaning given
  here;
* the presence rules stated in each table's *presence* column, **both ways
  round**: a column that names the cases a field appears in is equally the
  statement that it does not appear in the others. Reading an absence this
  document states is using the format;
* the **, possibly empty** qualifier and its absence as the same kind of
  statement — where a cell carries it an empty array is a value this format
  produces, and where a cell does not, the key is present only with something in
  it;
* the vocabularies of the closed enumerations: `GraphNode.kind`,
  `CoderView.harness`, `CoderView.access`,
  `GraphEdge.class`, `SchemaView.source`, `ToolView.source` and `RetryView.level`
  — which `TimeoutView.level` and `OnErrorView.level` share — plus
  `InputView.form`, `OnErrorView.strategy`, `ItemErrorView.strategy`,
  `MapView.dispatch`, `ModelView.form`, `ToolView.binding`, `StoreView.kind`,
  `StoreView.scope`, `SubflowView.context`, `TriggerView.type` and
  `WriteView.reduce`;
* the orders this document fixes — `flows` by address, a flow's `nodes` in
  declaration order with each satellite after its map, `edges` as §6 states,
  `routes` with `default:` last, a model's `route` in failover order, a
  provider's `config` and a model's `settings` by key, a coder node's
  `settings`, `env` and `allow_tools` in declaration order, and every field map
  in declaration order;
* that `id` is unique within a flow, that every `from` and `to` names a node the
  same flow's `nodes` array holds, and that a `map_route` edge's `to` is the
  `node` of one of that map's `routes`.

A reader MUST NOT rely on:

* **the absence of a field this document does not name.** A later version may
  add one, and a reader that rejects unknown keys will break on a compatible
  change. Ignore what you do not recognize. This is the complement of the
  presence rule above, not a retraction of it: widening a presence rule a
  *column* states is a version bump (§9.3).
* **the text of any summarized field** — `FieldView.type`, `GraphNode.binding`,
  `TriggerView.summary`, `MapView.join`, `ToolView.detail`. These are written
  for a person (PRD G3) and are improved between releases. Where a reader needs
  the structured fact behind one, this document says where it is: a schema's
  constraints are the composition's, a map's barrier is
  [`join_barrier`](#6-edges), and a tool's implementation is `binding`.
* **the graph document being the composition.** It is a projection: it carries
  what a picture needs and drops what a picture does not (§10).

### 9.2 What is a compatible change

Compatible, and made **without** a version bump:

* adding a field to an existing record type;
* adding a new record type reachable from an existing one;
* carrying a field in a case this document's presence column **already** names
  and the compiler was not carrying — a bug in the implementation of this format
  rather than a change to it;
* improving the text of a summarized field.

### 9.2.1 What version `2` changed

PRD resolved q57's `coder:` node kind (grammar §8.9) added a member to
`GraphNode.kind`, which §9.3 makes a **bump** rather than an addition: a reader
written against `1` was entitled to the ten kinds that version named, and an
eleventh makes it wrong. Nothing else moved. [`CoderView`](#58-a-harness-run) and
the [`coder`](#5-nodes) key that reaches it are a new record type and a new
field, both compatible on their own under §9.2; they ride this bump because they
arrived with the member that forced one. A composition with no `coder:` node
produces a byte-identical document but for its `graph_version`.

### 9.3 What requires a version bump

`graph_version` MUST be incremented for any change that would make a reader
written against the previous version wrong:

* removing a field, or renaming one;
* changing a field's type, or the meaning of its value;
* changing a presence rule in **either** direction, the **, possibly empty**
  qualifier included;
* adding a member to one of §9.1's closed enumerations, or removing one — which
  a new node kind, a new edge class and a new tool source each are;
* changing one of the fixed orders, or the derivation of a satellite's id;
* drawing a construct that used to be drawn some other way: a subgraph expanded
  inline rather than linked, a map route that stops being an edge.

A bump changes the number on both surfaces at once — `--format json` and the
document embedded in the page — because they are one document.

### 9.4 How the two are held together

The version number alone is a promise; seven tests make it a checkable one:

* `crates/compose-core/tests/graph_format_inventory.rs` reads this document and
  the emitted one and fails when their top-level fields differ, when a record
  type declared in `crates/compose-core/src/graph/document.rs` is not named
  here, and when a member of one of §9.1's closed vocabularies is not written
  out. Documentation cannot rot behind the code. The vocabularies it reads are
  generated by the declarations of the enumerations themselves, so a member
  cannot be absent from the list it checks — a list kept by hand beside an
  enumeration would pass on exactly the change this exists to catch.
* `crates/compose-core/src/graph/document.rs` holds the **page's** side of the
  two vocabularies the canvas draws with: the `KIND` table's kinds and their
  badges, and the `EDGE` table's classes. Both tables fall back rather than
  fail — an unknown kind would be drawn as a `map` and an unknown class as a
  plain unconditional edge — so a new member that nothing here named would ship
  as a collapsed rendering of something else.
* `crates/agent-compose/tests/visualize_graph_document.rs` asserts the semantic
  inventory over `examples/triage-fanout`: the conditional edges with their CEL,
  the `on_error:` fallback, the `on_timeout:` transfer, all three map routes
  with their narrowing, the synthesized store tool, and the model resolution.
* `crates/compose-core/tests/graph_document_invariants.rs` checks §9.1's
  structural promises — `id` unique within a flow, every `from` and `to` a node
  that flow holds, a `map_route` edge arriving at its own route's satellite —
  over a corpus that includes the composition those promises are hardest on: a
  union with a variant tagged `default` beside a catch-all `default:` route.
* `crates/compose-core/tests/graph_artifact_goldens.rs` commits the emitted
  bytes for two example projects, so a change to the document or to the template
  is a reviewable diff rather than a discovery made by a reader (PRD §9.15).
* `crates/compose-core/tests/graph_artifact_is_self_contained.rs` holds the
  emitted page to fetching nothing — which is a promise about the artifact
  rather than about this format, and the one that would break silently.
* `crates/compose-core/tests/graph_artifact_renders.rs` **runs** the page's own
  code against a DOM stub, over every node of every flow of four projects — one
  of them written to leave every optional array out. It is what makes the
  presence rules above checkable from the renderer's side: a page that indexed
  a key this format is allowed to omit throws on the compositions that omit it,
  and every other test here would stay green. It is also where the page's
  geometry is checked: that no fan-out container encloses a node it does not
  dispatch, and that no edge's chip is drawn over a node box.

---

## 10. What is not part of this format

* **Source spans.** The IR carries the region every construct was written in
  (PRD 5.1); this document carries none. A picture is read beside the
  composition rather than instead of it, and `agent-compose validate` is what
  points at a line.
* **Layout.** Positions are the renderer's, computed from the topology by the
  embedded template — layered longest-path with barycenter ordering, then a pass
  that pushes any node a fan-out's dashed container would enclose but does not
  dispatch clear of it, then a pass that puts each edge's chip at the first place
  along its own edge, and then off it, that clears every node box — and a viewer's
  own rearrangement lives in that viewer's browser. Nothing about where a node or
  a chip sits is in this document, which is what lets the emitted page be
  golden-tested while remaining draggable.
* **Resolved environment.** No `${ENV}` reference is substituted, here or
  anywhere upstream (PRD 5.9, resolved q15).
* **Anything a run did.** This is a picture of a composition, not of an
  execution. What happened is [`docs/trace.md`](trace.md).
* **The deploy layer.** Placements, storage backends, the hub and the trace sink
  are the target's rather than the graph's; `agent-compose plan` reports them and
  the IR carries them.
