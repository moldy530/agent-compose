//! `src/graph.ts`: the composition, assembled.
//!
//! # What one node becomes
//!
//! Every flow node is one LangGraph node whose function is
//! [`runtime.runNode`](super::runtime), driven by a **descriptor** this module
//! emits: the node's resolved policy (grammar 9.3, [`super::policy`]), how its
//! input is built (grammar 8.0), the activity it runs (grammar 8), where its
//! output fields go (grammar 10.3), and its outgoing edges in declaration order
//! (grammar 7.3). The descriptor is data; the behaviour it drives is the
//! runtime's, which is byte-identical in every project.
//!
//! # Why a node routes itself
//!
//! A node answers a LangGraph `Command`, which carries a state update **and**
//! the control transfer in one write. The alternative shaping —
//! `addConditionalEdges` with a path function — cannot write state, and grammar
//! 7.4 requires the iteration counter of a bounded edge to live *in graph
//! state*: the pass that decides an edge is taken is the pass that has to spend
//! its budget, or a replay would take a different branch than the live run did.
//! Grammar 7.6's note leaves codegen this choice ("a conditional edge emitting
//! `Send`s, a deferred join node — which is M1's call") and fixes the two
//! properties instead; both hold here:
//!
//! * **P1, the barrier** — the guards are evaluated inside the node's own task,
//!   after its activity has completed, so they cannot run before it;
//! * **P2, one run per step** — a node named by two `goto`s taken in one
//!   superstep is scheduled once, which is LangGraph's own semantics.
//!
//! # What a guard sees, and why that is a reading rather than a consequence
//!
//! A guard sees the state its node ran on plus that node's own writes, **not** a
//! concurrent sibling's from the same step. That is not a detail of the shaping;
//! it is a choice between two sentences of grammar 7.6 that do not agree, and it
//! is recorded here in the ledger style [`super::cel`] and [`super::schema`] use
//! for the same reason — a difference a reader signed off on, or a bug.
//!
//! | id | where | which way | why it is left |
//! |---|---|---|---|
//! | `a-guard-does-not-see-a-concurrent-siblings-write` | an edge guard reading a channel a concurrent branch writes in the same step | the guard sees **its own node's** writes only | grammar 7.6's P1 is stated per node — "a node's outgoing edges are evaluated only after **that node** has completed" — and routing inside the node's own task is exactly that. The *Steps* paragraph above P1 reads the other way ("when every node of step *k* has completed, its writes are applied … and its outgoing edges are evaluated"), which would need a second superstep between every node and its own edges: a deferred router node per flow, doubling the step count, moving the trace's step numbers, and evaluating a guard in a task that is not the node's — which P1 is written to forbid. The per-node reading is the one implemented, and `a_guard_sees_its_own_writes_and_not_a_concurrent_siblings` pins it |
//!
//! The row is reachable — a fork whose branches write a channel the other
//! branch's guard reads — so it is a routing outcome rather than a corner, and
//! **that is why it is pinned rather than left to be re-derived**. Which of the
//! two sentences is normative is the grammar's to say; until it says, this is
//! the answer the compiler gives and the test is what makes a change to it
//! deliberate.
//!
//! # `start`
//!
//! Unconditional `start` edges are plain `addEdge(START, …)`, one per edge, so
//! step 0 is the union of their targets. A composition whose `start` edges carry
//! guards gets a synthetic entry node instead — `start` has no output, so the
//! guards read `input`/`state`/`execution` (grammar 4.1) and there is nothing
//! else for a node to do there. It costs one superstep, which shifts every step
//! number in the trace by one and changes nothing about what runs.
//!
//! # Where a fan-out and a subgraph run, and why not `Send`
//!
//! A `map` dispatches its instances **inside the map node's own task**
//! (`runtime.runMap`), and a `flow:` node instantiates its subflow as a
//! **separate run of a separate compiled graph** (`runtime.runSubflow`). That is
//! the shaping PRD 5.5 and 5.6 now describe, ratified as PRD resolved question
//! 17; grammar 7.6's codegen note is what left the choice open, stating P1 and
//! P2 as properties "so codegen keeps its choice of shaping — a conditional edge
//! emitting `Send`s, a deferred join node — which is M1's call". The row below
//! is the argument that settled it, kept here because `Send` and an in-graph
//! subgraph remain available shapings and the reasons not to take them are
//! LangGraph-mechanical rather than grammatical.
//!
//! | id | where | which way | why it is left |
//! |---|---|---|---|
//! | `a-detached-dispatch-is-keyed-and-nothing-else-is` | which calls carry the idempotency key of grammar 9.4, and what wins when a binding names the same slot | only a **detached** dispatch to a `tool.*`, and the binding's own `headers:`/`env:` sit *over* the delivery while a `tool.*`'s input fields sit *under* it | grammar 9.4 fixes the string (D104) and now the surface — `Idempotency-Key`, `IDEMPOTENCY_KEY`, `idempotency_key` — leaving two things it does not say. *Which calls*: an `agent.*` has no delivery slot on the provider wire, and keying it would key the tool calls its own loop makes, which are distinct effects meant to repeat; a `flow.*` is handed the dispatch site itself, so an effect inside the instance derives its own key from the instance path rather than reusing the boundary's; a **joined** dispatch has an observed outcome, which is what dedupe is a substitute for. *What wins*: a binding's declared `headers:`/`env:` is the author configuring their own wire, so it layers over the delivery exactly as it layers over the emitted `content-type`; an `exec:` target's input fields layer under it, because an input field spelling `idempotency_key` is precisely what grammar 9.4 says the key is never part of, and losing the key there would lose the one thing a sink dedupes on — and no *composition* reaches that layer, because `check::maps` refuses a detached dispatch to a sink declaring the slot, which is Decision D66's rule for the same environment |
//! | `a-subgraph-is-the-one-activity-a-deadline-stops` | what a node's `timeout:` does to the work it was waiting on | a `flow:` node's instance — and a joined `map` dispatch's — is handed that node's `context.signal` and stops advancing; every other activity is raced and left running | grammar 9.2 bounds **one node execution** and says nothing about what becomes of the work, and `runtime.runActivity` records why that is usually all a deadline can mean: a host function cannot be unscheduled, and a raced promise is merely abandoned. A subgraph is not in that position — it is a run of its own, and LangGraph's `RunnableConfig.signal` stops the Pregel loop scheduling supersteps — so here the choice is real rather than forced. It is left the **stopping** way: an instance that runs on issues every effect its remaining nodes were going to issue *after* the node that started it has already failed, and holds the map node's admission permit (`runtime.Admission`) for the whole of it, so a later execution of that node queues behind work its own budget was supposed to have ended. What is still not stopped is the one activity already in flight *inside* the instance, which is the abandoned-host-function case again one level down. A **detached** dispatch keeps the other reading deliberately: its signal is the one nothing aborts, because Decision D94 says the fan-out never waited for it |
//! | `a-dispatch-runs-inside-the-map-nodes-task` | `map` dispatch and `flow:` instantiation | the instances run **in the node's task**, and a subflow is a separate run of its own compiled graph (PRD 9.17) | a `Send` schedules a node of the **parent** graph, and seven of grammar 8.6's own rules are then unstateable. A Send'd task reads the packet as its whole input and writes the **parent's** channels, so a dispatched `flow.*` cannot hold its own channel values (grammar 10.1, 7.6.4 clause 3) and its instances share the caller's `messages`, which grammar 10.4 and Decision D105 make an unwaivable module boundary. `detach: true` is *resolved at dispatch* (D94) and a superstep barrier waits for every task it scheduled. A dispatch of **zero** instances must complete and fire its edges (rule 6), while `goto: []` retires the branch. `max_concurrency` is a per-node admission bound routes may tighten (D28) and LangGraph's `maxConcurrency` is a run-level config the Pregel runner applies to every task of a superstep. `on_item_error`'s parameterized retry, and the map's own `on_error:` absorbing an exhausted item (rule 10), are policies over an *item* that a node-level `retryPolicy` cannot express — the same mismatch `runtime.runActivity` records for grammar 9. The map's own outgoing edges would be evaluated once per instance, over N different local states, and not at all when N is 0. And LangGraph orders a step's writes by `task.path`, where every `__pregel_pull` sorts before every `__pregel_push`, so a map's writes would land after *every* ordinary node's rather than in the map node's own place in clause 1's node-id order. Index-tagging survives the change: what the map writes is one `runtime.OrderedWrites` per channel, in source-item order, which every emitted reducer unpacks |
//!
//! What the row does **not** trade away is grammar 7.6's two properties. P1
//! holds because the join is the node: `runMap` returns when every instance has
//! completed, been resolved by `on_item_error`, or been detached, and only then
//! is the node's routing decision made — in the same task, as for every other
//! node. P2 is LangGraph's, unchanged.
//!
//! # A `human` node
//!
//! One node like any other, plus a [`human_descriptor`] beside it: the wait's
//! own budget and the route its expiry takes, and the two readings of its
//! `output:` — the published JSON Schema a status route hands whoever is
//! answering, and the emitted Zod that decides whether their answer fits
//! (grammar 8.7, PRD 5.11). Its `input:` is built in the node's input phase
//! through grammar 8.0's chain, exactly as an agent's is, because it *is* a
//! declared input surface — and because what it evaluates to is the question a
//! resume surface renders.
//!
//! Two things about it are the compiler's rather than the runtime's, and both
//! are D102's: the descriptor's `timeoutMs` is the node's own `human: { timeout:
//! … }` and never a resolved policy field, and the node's emitted `policy:`
//! carries neither `timeout` nor `retry` at any level — `exempt: true` is the
//! half of that which level 1 needs at runtime.
//!
//! # What is emitted for a construct this release does not execute
//!
//! A store bound to a **production** backend is the only one. Grammar
//! 14.2's vocabulary reaches past this release — `redis`, `pgvector`, `s3` and
//! the rest land in M3 — so [`backend_of`] resolves the alias at compile time
//! and the emitted binding carries the provider it resolved to; `src/stores.ts`
//! is where a store bound to one says so, naming the backend, where the
//! resolution came from, and the milestone. Under `--target local` no alias and
//! no per-kind default is consulted at all (PRD 5.8, Decision D87), which is
//! what makes a project with production infrastructure in `deploy/staging.yml`
//! still runnable with none.
//!
//! A `flow.*` in an agent's `tools:` was the other entry under this heading and
//! is not one any more. Flow-as-tool **runs**: the emitted `invoke` calls
//! `runtime.callSubflowTool`, which instantiates the module the model named and
//! joins its trace to the caller's (grammar 5.4, PRD resolved q19 and q20) — see
//! [`flow_tool`]. Nothing here refuses it.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::{Address, ControlTarget, EdgeSource, EdgeTarget, Interpolated};
use crate::ast::definition::{ProviderKind, StoreKind};
use crate::ast::flow::FlowContext;
// The SCC decomposition grammar 7.4 is checked over, reused rather than
// reimplemented: the ceiling below is sized from the same clause-1 reading the
// validator applies, and two readings of one rule is how they come to disagree.
use crate::check::cycles::counted;
use crate::check::graph::Graph as CheckedGraph;
use crate::ir::Ir;
use crate::ir::binding::{Bindings, Exec, Http, NodeInput, Writes};
use crate::ir::definition::{Agent, DefinitionBody, Model, Tool};
use crate::ir::flow::{
    Edge, Flow, ItemError, Map, MapDispatch, Node, NodeKind, ToolImplementation,
};
use crate::ir::policy::Policy;
use crate::ir::schema::{Field, FieldMap, TypeForm, TypeNode};

use super::names::{self, Names};
use super::policy::{self, Strategy};
use super::{cel, schema};

/// The canonical path of the shape every declared state channel is read
/// through.
///
/// Not `state.shape`: that is the path a channel **named** `shape` already owns
/// (`src/schemas.ts` exports its Zod under it), and one key for two declarations
/// is one *name* for two declarations — the collision this registry exists to
/// make impossible rather than unlikely. The `$` marks the path as the emitter's
/// own; grammar 2.1's identifier cannot open with one, and [`names::camel`]
/// drops it, so the emitted name is `stateShape` exactly as before unless a
/// composition really does declare a channel called `shape`.
const STATE_SHAPE: &str = "$state.shape";

/// Every name `src/graph.ts` declares, added to the shared registry.
///
/// Declared in the IR's own canonical order — definitions by address, then each
/// flow's nodes in declaration order — so two builds of one composition assign
/// the same names (PRD 5.12).
pub fn declare(names: &mut Names, ir: &Ir) {
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Provider(_)
            | DefinitionBody::Model(_)
            | DefinitionBody::Agent(_)
            | DefinitionBody::Tool(_) => {
                names.declare(address);
            }
            DefinitionBody::Flow(flow) => {
                names.declare(address);
                names.declare(&format!("{address}.shape"));
                names.declare(&format!("{address}.graph"));
                names.declare(&format!("{address}.binding"));
                if needs_start_router(flow) {
                    names.declare(&format!("{address}.start"));
                }
                for node in &flow.nodes {
                    let id = node.id.value.as_str();
                    names.declare(&format!("{address}.node.{id}"));
                    if matches!(node.kind, NodeKind::Map { .. }) {
                        names.declare(&format!("{address}.node.{id}.map"));
                    }
                    if matches!(node.kind, NodeKind::Human { .. }) {
                        names.declare(&format!("{address}.node.{id}.human"));
                    }
                }
            }
            // A store's binding is a `const` of its own, and it is the value a
            // store-op node and a synthesized tool both reach (grammar 11).
            DefinitionBody::Store(_) => {
                names.declare(address);
            }
        }
    }
    // A key of the emitter's own rather than `state.shape`, which a channel
    // called `shape` already owns — see [`STATE_SHAPE`].
    names.declare(STATE_SHAPE);
    for (address, definition) in &ir.definitions {
        if let DefinitionBody::Flow(flow) = &definition.body {
            for node in &flow.nodes {
                if output_path(ir, address, node).is_some() {
                    names.declare(&format!("{address}.node.{}.shape", node.id.value.as_str()));
                }
            }
        }
    }
}

/// `src/graph.ts`.
#[must_use]
pub fn module(ir: &Ir, names: &Names) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);

    let surfaces = schema::surfaces(ir);
    let mut imported: Vec<String> = Vec::new();
    let mut body = String::new();

    body.push_str(&shapes(ir, names, &surfaces));
    body.push_str(&providers(ir, names));
    body.push_str(&models(ir, names));
    body.push_str(&stores(ir, names));
    body.push_str(&tools(ir, names, &surfaces, &mut imported));
    body.push_str(&agents(ir, names, &surfaces, &mut imported));

    let mut registry: Vec<(String, String)> = Vec::new();
    for (address, definition) in &ir.definitions {
        if let DefinitionBody::Flow(flow) = &definition.body {
            body.push_str(&self::flow(
                ir,
                names,
                address,
                flow,
                &surfaces,
                &mut imported,
            ));
            registry.push((address.clone(), names.value(address).to_string()));
        }
    }
    body.push_str(&registry_source(ir, names, &registry, &mut imported));

    contents.push_str("\nimport { END, START, StateGraph } from \"@langchain/langgraph\";\n");
    contents.push_str("\nimport * as runtime from \"./runtime.ts\";\n");
    contents.push_str("import * as stores from \"./stores.ts\";\n");
    imported.sort();
    imported.dedup();
    if !imported.is_empty() {
        contents.push_str("import {\n");
        for name in &imported {
            contents.push_str(&format!("  {name},\n"));
        }
        contents.push_str("} from \"./schemas.ts\";\n");
    }
    contents.push_str("import { State } from \"./state.ts\";\n");
    contents.push_str("import type { GraphState } from \"./state.ts\";\n");
    contents.push_str(&body);

    super::GeneratedFile {
        path: "src/graph.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// The compiled graph (PRD 5.3, 5.4, grammar 7, 8, 9).
//
// One LangGraph node per flow node, driven by the descriptor beside it: how its
// input is built (grammar 8.0), the activity it runs (grammar 8), where its
// output goes (grammar 10.3), and its outgoing edges in declaration order
// (grammar 7.3). Every node answers a `Command`, so the branch it takes and the
// state it writes — including the counter a bounded edge spends (grammar 7.4) —
// land in one write. `./runtime.ts` is what the descriptors drive.
";

// ---------------------------------------------------------------------------
// Shapes: how a value's declared type reaches the evaluator (grammar 4.1)
// ---------------------------------------------------------------------------

fn shapes(ir: &Ir, names: &Names, surfaces: &[schema::Surface<'_>]) -> String {
    let mut text = String::from(
        "\n// --- Shapes: the declared type an expression reads a value through ---\n",
    );

    text.push_str(&names::doc(
        "",
        &[
            "Every declared state channel, so `state.count` is an `int` where the".to_string(),
            "channel says `type: integer` and a `double` where it says `type: number`.".to_string(),
        ],
    ));
    text.push_str(&format!(
        "const {}: runtime.Shape = {{\n  properties: {{\n",
        names.value(STATE_SHAPE)
    ));
    for (path, channel) in schema::channels(ir) {
        let _ = path;
        text.push_str(&format!(
            "    {}: {},\n",
            names::string(channel.name.value.as_str()),
            cel::shape_of_type(&channel.ty, "    ")
        ));
    }
    text.push_str("  },\n};\n");

    for (address, definition) in &ir.definitions {
        let DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        let empty = FieldMap {
            surface: crate::ast::schema::Surface::Input,
            fields: Vec::new(),
            span: definition.span.clone(),
        };
        let inputs = flow.inputs.as_ref().unwrap_or(&empty);
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — the `input` root inside it (grammar 7.5)."
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.Shape = {};\n",
            names.value(&format!("{address}.shape")),
            cel::shape_of_field_map(inputs, "")
        ));

        for node in &flow.nodes {
            let Some(path) = output_path(ir, address, node) else {
                continue;
            };
            let fields = surface_fields(surfaces, &path);
            let id = node.id.value.as_str();
            text.push('\n');
            text.push_str(&names::doc(
                "",
                &[format!(
                    "`{address}` node `{id}` — the `{id}.output` root its guards read."
                )],
            ));
            text.push_str(&format!(
                "const {}: runtime.Shape = {};\n",
                names.value(&format!("{address}.node.{id}.shape")),
                cel::shape_of_field_map(fields.as_ref(), "")
            ));
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Providers and models (grammar 12)
// ---------------------------------------------------------------------------

fn providers(ir: &Ir, names: &Names) -> String {
    let mut text = String::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Provider(provider) = &definition.body else {
            continue;
        };
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — {} `{}` connection (grammar 12.1). Every value is read \
                 when a node calls it, never at import: a build carries no credential and \
                 `./index.ts` is where their presence is checked (PRD 5.9).",
                article(provider.kind.as_str()),
                provider.kind.as_str()
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.ProviderBinding = {{\n",
            names.value(address)
        ));
        text.push_str(&format!("  address: {},\n", names::string(address)));
        text.push_str(&format!(
            "  kind: {},\n",
            names::string(provider_kind(provider.kind))
        ));
        let config = &provider.config;
        for (key, reference) in [
            ("apiKey", config.api_key.as_ref().map(|r| ("api_key", r))),
            ("baseUrl", config.base_url.as_ref().map(|r| ("base_url", r))),
        ] {
            let Some((spelling, reference)) = reference else {
                continue;
            };
            text.push_str(&format!(
                "  get {key}(): string {{\n    return runtime.environmentValue({}, {});\n  }},\n",
                names::string(&reference.value.name),
                names::string(&format!("{address}.{spelling}"))
            ));
        }
        for (key, spelling, value) in [
            (
                "apiVersion",
                "api_version",
                config.api_version.as_ref().map(|value| &value.value),
            ),
            (
                "organization",
                "organization",
                config.organization.as_ref().map(|value| &value.value),
            ),
        ] {
            let Some(value) = value else { continue };
            text.push_str(&format!(
                "  get {key}(): string {{\n    return runtime.interpolate({});\n  }},\n",
                interpolation(value, &format!("{address}.{spelling}"))
            ));
        }
        if !config.headers.is_empty() {
            text.push_str("  get headers(): Record<string, string> {\n    return {\n");
            for header in &config.headers {
                text.push_str(&format!(
                    "      {}: runtime.interpolate({}),\n",
                    names::string(&header.name.value),
                    interpolation(
                        &header.value.value,
                        &format!("{address}.headers.{}", header.name.value)
                    )
                ));
            }
            text.push_str("    };\n  },\n");
        }
        text.push_str("};\n");
    }
    text
}

/// `a` or `an`, so a generated sentence reads like one.
fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

const fn provider_kind(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::OpenAi => "openai",
        ProviderKind::OpenAiCompatible => "openai_compatible",
        ProviderKind::AzureOpenAi => "azure_openai",
        // Grammar 12.1's two SDK-reached kinds have no HTTP endpoint the spec
        // points at, so the runtime has no surface to render them onto. The
        // binding is still emitted — `runtime.ProviderKind` carries both, so a
        // project declaring one type-checks like any other — and `callModel` is
        // where a run that reaches one is told what it reached.
        ProviderKind::Bedrock => "bedrock",
        ProviderKind::Vertex => "vertex",
    }
}

/// Every `model.*`, direct bindings first and routes after them.
///
/// The order is load-bearing rather than cosmetic: a route's members are the
/// `const`s it names, and `model.default` sorts before `model.fast` in the IR's
/// address order — so emitting in that order would produce a module that
/// references a binding before its declaration and throws at import. Two passes
/// is the whole of the fix, and grammar 12.2 makes it sufficient: a route's
/// members are direct models, never other routes (Decision D39).
fn models(ir: &Ir, names: &Names) -> String {
    let mut text = String::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Model(Model::Direct(direct)) = &definition.body else {
            continue;
        };
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — `{}` on `{}` (grammar 12.2).",
                direct.id.value, direct.provider.value
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.ModelBinding = {{\n",
            names.value(address)
        ));
        text.push_str(&format!("  address: {},\n", names::string(address)));
        text.push_str(&format!("  id: {},\n", names::string(&direct.id.value)));
        text.push_str(&format!(
            "  provider: {},\n",
            names.value(&direct.provider.value.to_string())
        ));
        if direct.settings.is_empty() {
            text.push_str("  settings: {},\n");
        } else {
            text.push_str("  settings: {\n");
            for (key, value) in &direct.settings {
                text.push_str(&format!(
                    "    {}: {},\n",
                    names::string(key),
                    names::literal(&value.value)
                ));
            }
            text.push_str("  },\n");
        }
        text.push_str("};\n");
    }

    for (address, definition) in &ir.definitions {
        let DefinitionBody::Model(Model::Route(route)) = &definition.body else {
            continue;
        };
        let conditions: Vec<String> = route
            .route_on
            .as_ref()
            .map(|declared| {
                declared
                    .iter()
                    .map(|condition| condition.value.as_str().to_string())
                    .collect()
            })
            // Grammar 12.2's default, written out rather than left to the
            // runtime: `route_on:` decides which failures fail over, and a
            // default a reader cannot see in the emitted binding is one they
            // would have to look up.
            .unwrap_or_else(|| {
                DEFAULT_ROUTE_ON
                    .iter()
                    .map(|condition| (*condition).to_string())
                    .collect()
            });
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — an ordered failover route over {} (grammar 12.2, PRD 5.9). \
                 A member that refuses with one of the conditions below moves the call to \
                 the next; anything else fails the node, and which member served a call is \
                 recorded in the trace.",
                crate::parse::reader::list(
                    route
                        .route
                        .iter()
                        .map(|member| member.value.to_string())
                        .collect::<Vec<_>>()
                )
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.ModelRoute = {{\n",
            names.value(address)
        ));
        text.push_str(&format!("  address: {},\n", names::string(address)));
        text.push_str(&format!(
            "  route: [{}],\n",
            route
                .route
                .iter()
                .map(|member| names.value(&member.value.to_string()).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        text.push_str(&format!(
            "  routeOn: [{}],\n",
            conditions
                .iter()
                .map(|condition| names::string(condition))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        text.push_str("};\n");
    }
    text
}

/// Grammar 12.2's `route_on:` default.
const DEFAULT_ROUTE_ON: &[&str] = &["rate_limit", "overloaded", "timeout"];

// ---------------------------------------------------------------------------
// Stores (grammar 11)
// ---------------------------------------------------------------------------

/// Every `store.*`, as the binding `src/stores.ts` runs ops against.
///
/// One `const` per store rather than one per usage, because PRD 5.8's whole
/// point is that a store is **one definition with two consumption surfaces**: a
/// `store:` node and a synthesized tool address the same binding, so a
/// `scope: execution` store an agent wrote through is the same store the next
/// node reads.
fn stores(ir: &Ir, names: &Names) -> String {
    let mut text = String::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Store(store) = &definition.body else {
            continue;
        };
        let local = address
            .split_once('.')
            .map_or(address.as_str(), |(_, rest)| rest);
        let backend = backend_of(ir, store);
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — a `{}` store with `scope: {}`, on the `{}` backend ({}) \
                 (grammar 11.1, 11.3).",
                store.kind.as_str(),
                store.scope.as_str(),
                backend.provider,
                backend.from
            )],
        ));
        text.push_str(&format!(
            "const {}: stores.StoreBinding = {{\n",
            names.value(address)
        ));
        text.push_str(&format!("  address: {},\n", names::string(address)));
        text.push_str(&format!("  name: {},\n", names::string(local)));
        text.push_str(&format!(
            "  kind: {},\n",
            names::string(store.kind.as_str())
        ));
        text.push_str(&format!(
            "  scope: {},\n",
            names::string(store.scope.as_str())
        ));
        if let Some(description) = &store.description {
            text.push_str(&format!(
                "  description: {},\n",
                names::string(&description.value)
            ));
        }
        // Absent and `{}` are different declarations, and this is the field the
        // difference reaches the runtime through: a store with no
        // `metadata_schema:` derives matches with no `metadata` at all
        // (Decision D114).
        text.push_str(&format!(
            "  metadata: {},\n",
            store.metadata_schema.is_some()
        ));
        if let Some(embed) = &store.embed {
            text.push_str("  embed: {\n");
            text.push_str(&format!("    store: {},\n", names::string(address)));
            text.push_str(&format!(
                "    model: {},\n",
                names::string(&embed.model.value)
            ));
            text.push_str(&format!(
                "    provider: {},\n",
                names.value(&embed.provider.value.to_string())
            ));
            if let Some(dimensions) = embed.dimensions {
                text.push_str(&format!("    dimensions: {dimensions},\n"));
            }
            text.push_str("  },\n");
        }
        text.push_str("  backend: {\n");
        text.push_str(&format!(
            "    provider: {},\n",
            names::string(backend.provider)
        ));
        text.push_str(&format!("    from: {},\n", names::string(&backend.from)));
        text.push_str("  },\n");
        text.push_str("};\n");
    }
    text
}

/// Which backend a store resolved to under the active target, and why.
struct Backend {
    provider: &'static str,
    from: String,
}

/// Grammar 11.3's resolution order, run at compile time.
///
/// `--target local` substitutes local storage for **every** store
/// unconditionally, so under it no alias and no per-kind default is consulted at
/// all (PRD 5.8, Decision D87) — which is what makes a project with production
/// infrastructure in `deploy/staging.yml` still buildable and runnable with none.
/// Under any other target the order is the grammar's: explicit alias, then the
/// per-kind `defaults:`, then the target built-in, which is the same local
/// storage because it is the only backend this compiler release implements.
fn backend_of(ir: &Ir, store: &crate::ir::definition::Store) -> Backend {
    let built_in = match store.kind {
        StoreKind::Kv => "sqlite",
        StoreKind::Vector => "sqlite_vec",
        StoreKind::Blob => "local_fs",
    };
    if ir.target == crate::DEFAULT_TARGET {
        return Backend {
            provider: built_in,
            from: "the `local` target substitutes local storage for every store unconditionally"
                .to_string(),
        };
    }
    let backends = ir.deploy.storage_backends.as_ref();
    if let Some(alias) = &store.backend
        && let Some(config) =
            backends.and_then(|backends| backends.aliases.get(alias.value.as_str()))
    {
        return Backend {
            provider: config.provider.as_str(),
            from: format!(
                "the alias `{}`, defined by the `{}` target",
                alias.value, ir.target
            ),
        };
    }
    if let Some(config) = backends.and_then(|backends| backends.defaults.get(store.kind.as_str())) {
        return Backend {
            provider: config.provider.as_str(),
            from: format!(
                "the `{}` default of the `{}` target",
                store.kind.as_str(),
                ir.target
            ),
        };
    }
    Backend {
        provider: built_in,
        from: format!(
            "the built-in for `kind: {}`, which the `{}` target does not override",
            store.kind.as_str(),
            ir.target
        ),
    }
}

// ---------------------------------------------------------------------------
// Tools (grammar 6)
// ---------------------------------------------------------------------------

fn tools(
    ir: &Ir,
    names: &Names,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
) -> String {
    let mut text = String::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Tool(tool) = &definition.body else {
            continue;
        };
        let input_schema = names.value(&format!("{address}.input"));
        let output_schema = names.value(&format!("{address}.output"));
        imported.push(input_schema.to_string());
        imported.push(output_schema.to_string());
        let input_fields = surface_fields(surfaces, &format!("{address}.input"));

        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — {} (grammar 6.1). Its arguments are parsed with its own \
                 declared `input:` before the implementation sees them, which is the \
                 checked signature grammar 8.4 asks for. This is the parse a \
                 `function:` node's binding faces, and it is a `ResultMismatch` that \
                 fails the node: the arguments are the composition's, checked \
                 field-by-field at compile time, so a value constraint they miss at \
                 runtime is the graph's own failure and there is nobody to hand it to. \
                 A **model's** arguments are refused one level out, at the entry in the \
                 agent's `tools:`, where `runtime.parseToolArguments` raises the \
                 `ToolCallRefused` the loop hands back (Decision D119). The tool's \
                 **result** is parsed with `runtime.parseResult` on both surfaces: a \
                 tool answering off-contract is not a call anybody can rephrase.",
                match &tool.implementation {
                    ToolImplementation::Exec { .. } => "a subprocess",
                    ToolImplementation::Http { .. } => "an HTTP request",
                    ToolImplementation::Function { .. } => "a host-registered function",
                }
            )],
        ));
        text.push_str(&format!(
            "async function {}(args: unknown, context: runtime.RunContext): Promise<unknown> {{\n",
            names.value(address)
        ));
        text.push_str(&format!(
            "  const input = runtime.parseResult({input_schema}, args, {});\n",
            names::string(&format!("the arguments `{address}` was called with"))
        ));
        match &tool.implementation {
            ToolImplementation::Exec { exec } => {
                text.push_str(&format!(
                    "  return runtime.parseResult(\n    {output_schema},\n    await runtime.runExec({}, input, context),\n    {},\n  );\n",
                    exec_binding(exec, &tool.output, address, "    ", true),
                    names::string(&format!("the result of `{address}`"))
                ));
            }
            ToolImplementation::Http { http } => {
                // `roots` is what a declared `query:`/`body:` expression is
                // evaluated against — the tool's own `input` and nothing else
                // (grammar 6.1, D65). A binding that declares neither sends the
                // input object whole and evaluates nothing, so it gets no
                // `roots` rather than an unused one.
                if http.query.is_some() || http.body.is_some() {
                    text.push_str(&format!(
                        "  const roots = {{ input: runtime.bind(input, {}) }};\n",
                        cel::shape_of_field_map(input_fields.as_ref(), "  ")
                    ));
                }
                text.push_str(&format!(
                    "  return runtime.parseResult(\n    {output_schema},\n    await runtime.runHttp({}, {}, context),\n    {},\n  );\n",
                    http_binding(http, &tool.output, address, "    ", true),
                    http_request(http, Bound::ToolInput, "    "),
                    names::string(&format!("the result of `{address}`"))
                ));
            }
            ToolImplementation::Function { function } => {
                text.push_str(&format!(
                    "  return runtime.parseResult(\n    {output_schema},\n    await runtime.callFunction({}, input, context),\n    {},\n  );\n",
                    names::string(function.name.value.as_str()),
                    names::string(&format!("the result of `{address}`"))
                ));
            }
        }
        text.push_str("}\n");
    }
    text
}

// ---------------------------------------------------------------------------
// Agents (grammar 5)
// ---------------------------------------------------------------------------

fn agents(
    ir: &Ir,
    names: &Names,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
) -> String {
    let mut text = String::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Agent(agent) = &definition.body else {
            continue;
        };
        let output = surface_fields(surfaces, &format!("{address}.output"));
        let local = address
            .split_once('.')
            .map_or(address.as_str(), |(_, rest)| rest);

        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — one LLM call with structured output (PRD 5.2, grammar 5). \
                 The schema below is the **published** JSON Schema of grammar 3.8's table, \
                 which is the column the conformance corpus proves equal to the parse its \
                 answer then faces."
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.AgentBinding = {{\n",
            names.value(address)
        ));
        text.push_str(&format!("  address: {},\n", names::string(address)));
        text.push_str(&format!(
            "  prompt: {},\n",
            names::string(&agent.prompt.value)
        ));
        text.push_str(&format!(
            "  model: {},\n",
            names.value(&agent.model.value.to_string())
        ));
        text.push_str("  output: {\n");
        text.push_str(&format!(
            "    name: {},\n",
            names::string(&output_tool_name(ir, agent, local))
        ));
        text.push_str(&format!(
            "    description: {},\n",
            names::string(&format!("The structured output `{address}` must produce."))
        ));
        text.push_str(&format!(
            "    schema: {},\n",
            json_literal(&schema::json_field_map(output.as_ref()), "    ")
        ));
        text.push_str("  },\n");
        if agent.tools.is_empty() && agent.stores.is_empty() {
            text.push_str("  tools: [],\n");
        } else {
            text.push_str("  tools: [\n");
            for reference in &agent.tools {
                let tool_address = reference.value.to_string();
                let Some(attached) = ir.definitions.get(&tool_address) else {
                    continue;
                };
                let DefinitionBody::Tool(tool) = &attached.body else {
                    // A `flow.*` here is flow-as-tool (grammar 5.4): a real tool
                    // on the wire, and a call to it instantiates the module.
                    if let DefinitionBody::Flow(attached) = &attached.body {
                        text.push_str(&flow_tool(
                            ir,
                            names,
                            surfaces,
                            imported,
                            &tool_address,
                            attached,
                        ));
                    }
                    continue;
                };
                let local_name = tool_address
                    .split_once('.')
                    .map_or(tool_address.as_str(), |(_, rest)| rest);
                let input = surface_fields(surfaces, &format!("{tool_address}.input"));
                text.push_str("    {\n");
                text.push_str(&format!("      name: {},\n", names::string(local_name)));
                text.push_str(&format!(
                    "      address: {},\n",
                    names::string(&tool_address)
                ));
                text.push_str(&format!(
                    "      description: {},\n",
                    names::string(&tool.description.value)
                ));
                text.push_str(&format!(
                    "      schema: {},\n",
                    json_literal(&schema::json_field_map(input.as_ref()), "      ")
                ));
                // The model's arguments are parsed **here**, at the attachment,
                // rather than by the tool function the `function:` node shares
                // with it: only this call site has a model to hand a refusal
                // back to (Decision D119), and only here is the tool's name the
                // local one the wire offered — a refusal naming `{tool_address}`
                // would name an identifier the model cannot call. The tool then
                // parses what it is given a second time, which is what keeps its
                // own signature checked for every caller (grammar 6.1, 8.4). The
                // value it re-parses has already satisfied the schema and the
                // second parse cannot move it: nothing `codegen::schema` emits
                // coerces or strips — every object is `.strict()`, no `format:`
                // normalizes, no field transforms — and its one exception,
                // `.default(…)`, has already fired, so the property it fills is
                // present and it does not fire again (see that module's doc,
                // which states the premise and the exception together). A
                // coercing or non-idempotent form emitted there would make this
                // second parse observable, and would have to be answered here.
                let input_schema = names.value(&format!("{tool_address}.input"));
                imported.push(input_schema.to_string());
                text.push_str(&format!(
                    "      invoke: (args, context) =>\n        {}(\n          \
                     runtime.parseToolArguments({input_schema}, args, {}),\n          \
                     context,\n        ),\n",
                    names.value(&tool_address),
                    names::string(&format!("the arguments `{local_name}` was called with"))
                ));
                text.push_str("    },\n");
            }
            text.push_str(&store_tools(ir, names, surfaces, agent, imported));
            text.push_str("  ],\n");
        }
        text.push_str(&format!(
            "  maxToolIterations: {},\n",
            agent.max_tool_iterations.unwrap_or(DEFAULT_TOOL_ITERATIONS)
        ));
        text.push_str("};\n");
    }
    text
}

/// Grammar 5's default for `max_tool_iterations:` (Decision D51, PRD §9.14).
const DEFAULT_TOOL_ITERATIONS: i64 = 8;

/// One `flow.*` in an agent's `tools:` — flow-as-tool (grammar 5.4, PRD 5.1).
///
/// The tool is emitted with the contract grammar 5.4 gives it — its local name
/// is the name the model calls, its `description:`, which the validator requires
/// exactly here, is the selection signal, and its `inputs:` is the parameter
/// schema — and a call to it **instantiates the module**, which is PRD 5.1's
/// flow-as-tool equivalence executed rather than asserted.
///
/// Everything the instantiation needs that a module-level binding cannot know is
/// handed to `runtime.callSubflowTool` at the call: the invoking agent
/// execution's instance path and the ordinal of this call, which together are
/// the frame grammar 9.4 gives it (PRD resolved q19), and the level-1 policy
/// that reached the calling instance. What is emitted here is the half that *is*
/// a compile-time constant — which module, under which name, held to which
/// parameter schema.
///
/// The parameter schema is emitted twice, and the two columns are the same
/// document: the JSON below is what constrains the model, and the emitted Zod
/// beside it is what the arguments are parsed with, which is the constrain ==
/// parse equality PRD §9.16 makes structured output load-bearing for. A flow
/// with no `inputs:` is a no-argument tool: no field map is written anywhere, so
/// there is no schema to name and the instance starts on an empty object.
///
/// What an attachment becomes is a **tool**, unconditionally — never a source
/// comment, and never an entry the emitter skips. This is the attachment where
/// that posture was once broken: the entry was emitted as a comment and no tool,
/// so a composition the validator had analysed the attachment of — grammar 7.7
/// clause 4 carries session coherence, sync interrupt-freedom and recursion
/// through it — reached the provider with the tool missing and nothing anywhere
/// saying so. The tool-name-collision rule in [`crate::check::bindings`] leans
/// on the same posture from the other side: two attachments of one local name
/// are both emitted and the provider refuses the request, because dropping
/// either would be that bug again.
fn flow_tool(
    ir: &Ir,
    names: &Names,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
    address: &str,
    flow: &Flow,
) -> String {
    let local = address.split_once('.').map_or(address, |(_, rest)| rest);
    let description = flow.description.as_ref().map_or_else(
        // Unreachable over a composition `validate` accepted: grammar 5.4 makes
        // `description:` REQUIRED of a flow in a `tools:` list, and
        // `check::bindings` refuses one without. Said rather than unwrapped,
        // because a panic here would be a `build` crash over a rule with a
        // diagnostic of its own.
        || format!("The flow `{address}`, attached as a tool."),
        |description| description.value.clone(),
    );
    let parameters = flow_parameters(ir, surfaces, address);
    let inputs = if flow.inputs.is_some() {
        let schema = names.value(&format!("{address}.inputs")).to_string();
        imported.push(schema.clone());
        format!(", inputs: {schema}")
    } else {
        String::new()
    };
    let mut text = String::from("    {\n");
    text.push_str(&format!("      name: {},\n", names::string(local)));
    text.push_str(&format!("      address: {},\n", names::string(address)));
    text.push_str(&format!(
        "      description: {},\n",
        names::string(&description)
    ));
    text.push_str(&format!(
        "      schema: {},\n",
        json_literal(&schema::json_fields(parameters.as_ref()), "      ")
    ));
    text.push_str(&format!(
        "      invoke: (args, context, call) =>\n        runtime.callSubflowTool(\n          \
         {{ name: {}, binding: {}{inputs} }},\n          args,\n          context,\n          \
         call,\n        ),\n",
        names::string(local),
        names.value(&format!("{address}.binding")),
    ));
    text.push_str("    },\n");
    text
}

/// The tools an agent's attached stores synthesize (grammar 11.5, PRD 5.8).
///
/// One `AgentTool` per row of grammar 11.5's table, read through the store's
/// `agent_access:` — so a store declared `read` offers its reading ops and not
/// its writing one, which is the least-privilege knob Decision D37 adds and PRD
/// §9.13 accepts. The arguments are parsed against the emitted Zod for the tool's
/// own surface before the store sees them, which is the same schema the JSON
/// column below constrains the model with: constrain == parse, exactly as for an
/// agent's own output (PRD §9.16). Through `runtime.parseToolArguments`, so a
/// refusal returns to the model like every other tool surface's (Decision D119);
/// a *backend* that then fails is the store's own failure and fails the node.
///
/// They are **appended** to the declared tools rather than merged into them, so
/// a transcript reads in the order the composition declares: `tools:` first,
/// then `stores:` in their own declaration order.
fn store_tools(
    ir: &Ir,
    names: &Names,
    surfaces: &[schema::Surface<'_>],
    agent: &Agent,
    imported: &mut Vec<String>,
) -> String {
    let mut text = String::new();
    for reference in &agent.stores {
        let address = reference.value.to_string();
        let Some(definition) = ir.definitions.get(&address) else {
            continue;
        };
        let DefinitionBody::Store(store) = &definition.body else {
            continue;
        };
        let local = address
            .split_once('.')
            .map_or(address.as_str(), |(_, rest)| rest);
        let access = store
            .agent_access
            .unwrap_or(crate::ast::definition::AgentAccess::ReadWrite);
        for op in crate::check::model::store_tools(store.kind, access) {
            let name = crate::check::model::store_tool_name(local, *op);
            let path = format!("{address}.tool.{}.input", op.as_str());
            let schema_name = names.value(&path).to_string();
            imported.push(schema_name.clone());
            let arguments = surface_fields(surfaces, &path);
            text.push_str("    {\n");
            text.push_str(&format!("      name: {},\n", names::string(&name)));
            text.push_str(&format!("      address: {},\n", names::string(&address)));
            text.push_str(&format!(
                "      description: {},\n",
                names::string(&store_tool_description(store, &address, *op))
            ));
            text.push_str(&format!(
                "      schema: {},\n",
                json_literal(&schema::json_field_map(arguments.as_ref()), "      ")
            ));
            text.push_str(&format!(
                "      invoke: async (args, context) =>\n        stores.runStoreTool(\n          \
                 {},\n          {},\n          runtime.parseToolArguments({schema_name}, args, {}) as Record<string, unknown>,\n          \
                 context,\n        ),\n",
                names.value(&address),
                names::string(op.as_str()),
                names::string(&format!("the arguments `{name}` was called with"))
            ));
            text.push_str("    },\n");
        }
    }
    text
}

/// What a synthesized store tool tells the model it does.
///
/// The store's own `description:` is the LLM-facing half grammar 11.1 asks for,
/// and the op supplies the verb: a model choosing between `docs_search` and
/// `prefs_get` is choosing between two stores, and a description that named only
/// the op would leave it guessing which.
fn store_tool_description(
    store: &crate::ir::definition::Store,
    address: &str,
    op: crate::ast::flow::StoreOp,
) -> String {
    let what = match op {
        crate::ast::flow::StoreOp::Get => "Read one stored value by key from",
        crate::ast::flow::StoreOp::Set => "Store a value under a key in",
        crate::ast::flow::StoreOp::Delete => "Delete the value at a key in",
        crate::ast::flow::StoreOp::List => "List the keys of",
        crate::ast::flow::StoreOp::Search => "Search by meaning in",
        crate::ast::flow::StoreOp::Upsert => "Add or replace a document in",
        crate::ast::flow::StoreOp::Put => "Store an object under a key in",
    };
    match &store.description {
        Some(description) => format!("{what} `{address}`. {}", description.value.trim()),
        None => format!("{what} `{address}`."),
    }
}

/// The name the agent's output schema is offered to the model under.
///
/// `<local name>_output`, which is what makes a transcript readable — except
/// where the agent already attaches a tool of that name, in which case the two
/// would be one tool on the wire and the pinned choice would be ambiguous.
fn output_tool_name(ir: &Ir, agent: &Agent, local: &str) -> String {
    let mut name = format!("{local}_output");
    let mut attached: Vec<String> = agent
        .tools
        .iter()
        .filter_map(|reference| {
            let address = reference.value.to_string();
            ir.definitions.get(&address).map(|_| {
                address
                    .split_once('.')
                    .map_or(address.clone(), |(_, rest)| rest.to_string())
            })
        })
        .collect();
    // The synthesized store tools are on the wire beside the declared ones
    // (grammar 11.5), so they are names the pinned output tool has to avoid too:
    // two tools of one name would make the pinned choice ambiguous.
    for reference in &agent.stores {
        let address = reference.value.to_string();
        let Some(definition) = ir.definitions.get(&address) else {
            continue;
        };
        let DefinitionBody::Store(store) = &definition.body else {
            continue;
        };
        let store_local = address
            .split_once('.')
            .map_or(address.as_str(), |(_, rest)| rest);
        let access = store
            .agent_access
            .unwrap_or(crate::ast::definition::AgentAccess::ReadWrite);
        for op in crate::check::model::store_tools(store.kind, access) {
            attached.push(crate::check::model::store_tool_name(store_local, *op));
        }
    }
    let mut ordinal = 2;
    while attached.iter().any(|tool| tool == &name) {
        name = format!("{local}_output_{ordinal}");
        ordinal += 1;
    }
    name
}

// ---------------------------------------------------------------------------
// Flows (grammar 7)
// ---------------------------------------------------------------------------

fn flow(
    ir: &Ir,
    names: &Names,
    address: &str,
    flow: &Flow,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
) -> String {
    let mut text = format!("\n// --- {address} ---\n");
    let state_shape = names.value(STATE_SHAPE);
    let input_shape = names.value(&format!("{address}.shape"));

    if needs_start_router(flow) {
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}`'s entry: a `start` edge here carries a guard, and `start` is \
                 not a node, so the guards are evaluated by a synthetic node that runs \
                 nothing. It reads `input`, `state` and `execution` — the roots grammar \
                 4.1 gives an edge leaving `start` — and costs the run one superstep."
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.NodeDescriptor = {{\n",
            names.value(&format!("{address}.start"))
        ));
        text.push_str(&format!("  flow: {},\n", names::string(address)));
        text.push_str(&format!("  node: {},\n", names::string(START_ROUTER)));
        text.push_str("  policy: { onError: \"fail\" },\n");
        text.push_str(&format!(
            "  shapes: {{ input: {input_shape}, state: {state_shape}, output: \"any\" }},\n"
        ));
        text.push_str("  input: () => null,\n");
        text.push_str("  run: () => Promise.resolve({ output: {} }),\n");
        text.push_str("  writes: [],\n");
        text.push_str("  edges: [\n");
        for (index, edge) in flow.edges.iter().enumerate() {
            if starts(edge) {
                text.push_str(&edge_descriptor(address, edge, index, None));
            }
        }
        text.push_str("  ],\n");
        text.push_str("};\n");
    }

    let retained = retained_nodes(flow);

    for node in &flow.nodes {
        let id = node.id.value.as_str();
        let resolved = policy::resolve(ir, node);
        let output = output_path(ir, address, node);
        let output_shape = output
            .as_ref()
            .map(|_| {
                names
                    .value(&format!("{address}.node.{id}.shape"))
                    .to_string()
            })
            .unwrap_or_else(|| "\"any\"".to_string());

        if let NodeKind::Map { map } = &node.kind {
            text.push_str(&map_descriptor(
                ir, names, address, flow, node, map, surfaces, imported,
            ));
        }

        if let NodeKind::Human { human } = &node.kind {
            text.push_str(&human_descriptor(
                names, address, node, human, surfaces, imported,
            ));
        }

        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` node `{id}` — {}.",
                describe_kind(&node.kind)
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.NodeDescriptor = {{\n",
            names.value(&format!("{address}.node.{id}"))
        ));
        text.push_str(&format!("  flow: {},\n", names::string(address)));
        text.push_str(&format!("  node: {},\n", names::string(id)));
        text.push_str(&emitted_policy(&resolved));
        if matches!(node.kind, NodeKind::Human { .. }) {
            // Grammar 9.3 level 1 reaches this node's `on_error` and neither of
            // the other two fields (Decision D102).
            text.push_str("  exempt: true,\n");
        }
        if retained.contains(id) {
            // A `map.over` in this flow reads this node's result, and the map
            // runs in a later step (grammar 8.6 rule 11).
            text.push_str("  retains: true,\n");
        }
        text.push_str(&format!(
            "  shapes: {{ input: {input_shape}, state: {state_shape}, output: {output_shape} }},\n"
        ));
        text.push_str(&input_builder(ir, names, address, node, surfaces));
        text.push_str(&activity(ir, names, address, node, surfaces, imported));
        text.push_str(&writes(ir, address, node, surfaces));
        text.push_str(&edges(ir, address, flow, node));
        text.push_str("};\n");
    }

    // The builder.
    text.push('\n');
    text.push_str(&names::doc(
        "",
        &[format!(
            "`{address}` — its nodes, its `start` edges, and the compiled graph."
        )],
    ));
    text.push_str(&format!(
        "function {}() {{\n  return new StateGraph(State)\n",
        names.value(address)
    ));
    for node in &flow.nodes {
        let id = node.id.value.as_str();
        let descriptor = names.value(&format!("{address}.node.{id}"));
        let mut ends: Vec<String> = Vec::new();
        for edge in flow.edges.iter().filter(|edge| leaves(edge, id)) {
            let target = target_name(&edge.to.value);
            if !ends.contains(&target) {
                ends.push(target);
            }
        }
        // The two control-transfer positions are scheduling edges too
        // (grammar 7.8 clauses 2 and 3): a node reached only by one of them is
        // live code, and a `ends` list that left it out would be a graph
        // LangGraph refuses to compile for a composition the validator accepts.
        if let (Strategy::Fallback(target), _) = policy::resolve(ir, node).on_error {
            let target = control_name(target);
            if !ends.contains(&target) {
                ends.push(target);
            }
        }
        if let NodeKind::Human { human } = &node.kind
            && let Some(on_timeout) = &human.on_timeout
        {
            let target = control_name(&on_timeout.value);
            if !ends.contains(&target) {
                ends.push(target);
            }
        }
        text.push_str(&format!(
            "    .addNode({}, (state: GraphState) => runtime.runNode({descriptor}, state), {{\n      ends: [{}],\n    }})\n",
            names::string(id),
            ends.join(", ")
        ));
    }
    if let Some(entry) = start_router(names, address, flow) {
        text.push_str(&entry);
    } else {
        for edge in flow.edges.iter().filter(|edge| starts(edge)) {
            text.push_str(&format!(
                "    .addEdge(START, {})\n",
                names::string(node_of(&edge.to.value))
            ));
        }
    }
    text.push_str("    .compile();\n}\n");
    text.push('\n');
    text.push_str(&names::doc(
        "",
        &[format!(
            "`{address}`, compiled once. Building it at import is also what checks it: a \
             state model LangGraph refuses, or an edge to a node that is not registered, \
             fails here rather than at the first invocation."
        )],
    ));
    text.push_str(&format!(
        "const {} = {}();\n",
        names.value(&format!("{address}.graph")),
        names.value(address)
    ));

    // The same graph, as a `flow:` node and a `map` dispatch reach it
    // (grammar 8.5, 8.6). An instance is a **separate run** of it, started from
    // a state built out of the instantiation's bindings alone: grammar 10.1
    // makes the channel set composition-global in shape and per-instance in
    // value, so a subgraph sharing the caller's state object would be sharing
    // the values D68 says nothing falls through.
    let compiled = names.value(&format!("{address}.graph"));
    text.push('\n');
    text.push_str(&names::doc(
        "",
        &[format!(
            "`{address}` as a module: what a `flow:` node instantiates and a `map` \
             dispatches to (grammar 7.5, 8.5)."
        )],
    ));
    text.push_str(&format!(
        "const {}: runtime.SubflowBinding = {{\n",
        names.value(&format!("{address}.binding"))
    ));
    text.push_str(&format!("  address: {},\n", names::string(address)));
    text.push_str(&format!(
        "  outputs: [{}],\n",
        flow.outputs
            .fields
            .iter()
            .map(|field| names::string(field.name.value.as_str()))
            .collect::<Vec<_>>()
            .join(", ")
    ));
    text.push_str(&format!("  recursionLimit: {},\n", recursion_limit(flow)));
    text.push_str(&format!(
        "  stream: (initial, options) =>\n    \
         {compiled}.stream(initial, {{\n      \
         ...options,\n      \
         streamMode: \"values\",\n      \
         outputKeys: {compiled}.outputChannels,\n    \
         }}) as unknown as Promise<AsyncIterable<runtime.GraphStateLike>>,\n"
    ));
    text.push_str("};\n");
    text
}

/// The nodes of this flow whose result a `map.over` reads (grammar 8.6 rule 11).
///
/// `over` is the one surface that reads a node's output from another node's task
/// (Decision D42), and the producer ran in an earlier step — so its answer is
/// kept in `$run.outputs`. Only these nodes keep one: a node result is
/// arbitrarily large and that channel is carried through every superstep.
fn retained_nodes(flow: &Flow) -> BTreeSet<&str> {
    let mut retained = BTreeSet::new();
    for node in &flow.nodes {
        let NodeKind::Map { map } = &node.kind else {
            continue;
        };
        let root = map.over.value.root.as_str();
        if flow
            .nodes
            .iter()
            .any(|candidate| candidate.id.value.as_str() == root)
        {
            retained.insert(root);
        }
    }
    retained
}

/// Whether any `start` edge carries a guard, so the flow needs a synthetic
/// entry node to evaluate them (grammar 7.2, 7.6.3 rule 2).
fn needs_start_router(flow: &Flow) -> bool {
    flow.edges
        .iter()
        .filter(|edge| starts(edge))
        .any(|edge| edge.when.is_some() || edge.else_edge.is_some())
}

/// The synthetic entry node's wiring, if the flow needs one.
fn start_router(names: &Names, address: &str, flow: &Flow) -> Option<String> {
    if !needs_start_router(flow) {
        return None;
    }
    let mut ends: Vec<String> = Vec::new();
    for edge in flow.edges.iter().filter(|edge| starts(edge)) {
        let target = target_name(&edge.to.value);
        if !ends.contains(&target) {
            ends.push(target);
        }
    }
    let mut text = String::new();
    text.push_str(&format!(
        "    .addNode({}, (state: GraphState) => runtime.runNode({}, state), {{\n      ends: [{}],\n    }})\n",
        names::string(START_ROUTER),
        names.value(&format!("{address}.start")),
        ends.join(", ")
    ));
    text.push_str(&format!(
        "    .addEdge(START, {})\n",
        names::string(START_ROUTER)
    ));
    Some(text)
}

/// The node id the synthetic entry takes. Grammar 2.1's identifier cannot spell
/// it, so no composition can collide with it.
const START_ROUTER: &str = "$start";

fn describe_kind(kind: &NodeKind) -> String {
    match kind {
        NodeKind::Agent { agent } => format!("`{}` (grammar 8.1)", agent.value),
        NodeKind::Exec { .. } => "an inline subprocess (grammar 8.2)".to_string(),
        NodeKind::Http { .. } => "an inline request (grammar 8.3)".to_string(),
        NodeKind::Function { function } => format!("`{}` (grammar 8.4)", function.value),
        NodeKind::Flow { flow, .. } => format!("`{}` (grammar 8.5)", flow.value),
        NodeKind::Map { .. } => "a fan-out (grammar 8.6)".to_string(),
        NodeKind::Human { .. } => "a human-in-the-loop pause (grammar 8.7)".to_string(),
        NodeKind::Store { store, op, .. } => {
            format!("a `{}` on `{}` (grammar 8.8)", op.as_str(), store.value)
        }
    }
}

fn emitted_policy(resolved: &policy::Resolved<'_>) -> String {
    let mut text = String::from("  // Grammar 9.3, resolved: ");
    text.push_str(&format!(
        "`retry` from {}, `timeout` from {}, `on_error` from {}.\n",
        resolved.retry.1.as_str(),
        resolved.timeout.1.as_str(),
        resolved.on_error.1.as_str()
    ));
    text.push_str("  policy: {\n");
    if let (Some(retry), _) = resolved.retry {
        text.push_str(&format!("    retry: {},\n", retry_object(retry, "    ")));
    }
    if let (Some(timeout), _) = resolved.timeout {
        text.push_str(&format!(
            "    timeoutMs: {},\n",
            policy::milliseconds(timeout)
        ));
    }
    text.push_str(&format!(
        "    onError: {},\n",
        match resolved.on_error.0 {
            Strategy::Fail => "\"fail\"".to_string(),
            Strategy::Skip => "\"skip\"".to_string(),
            Strategy::Fallback(target) => format!(
                "{{ fallback: {} }}",
                names::string(&control_name_raw(target))
            ),
        }
    ));
    text.push_str("  },\n");
    text
}

/// A `retry:` block as the runtime's `RetryPolicy` (grammar 9.1).
///
/// Three sites render one: a node's resolved policy, a `flow:` node's `policy:`
/// override, and a `map`'s `on_item_error: { retry: … }` — which grammar 8.6
/// rule 10 defines as §9.1's block verbatim, so it is this same rendering.
fn retry_object(retry: &crate::ir::policy::Retry, indent: &str) -> String {
    let inner = format!("{indent}  ");
    let mut text = String::from("{\n");
    text.push_str(&format!("{inner}max: {},\n", retry.max));
    text.push_str(&format!(
        "{inner}backoffMs: {},\n",
        policy::milliseconds(&retry.backoff.value)
    ));
    text.push_str(&format!(
        "{inner}multiplier: {},\n",
        names::number(&crate::ast::schema::Number::Float(
            retry.multiplier.unwrap_or(2.0)
        ))
    ));
    if let Some(max_backoff) = &retry.max_backoff {
        text.push_str(&format!(
            "{inner}maxBackoffMs: {},\n",
            policy::milliseconds(&max_backoff.value)
        ));
    }
    text.push_str(&format!(
        "{inner}jitter: {},\n",
        retry.jitter.unwrap_or(true)
    ));
    text.push_str(&format!("{indent}}}"));
    text
}

/// A `flow:` node's `policy:` as the runtime's level-1 override (grammar 9.3).
///
/// `on_error` takes only `fail` and `skip` here: the `fallback:` form names a
/// node id of the flow the declaring node sits in, and this level names no flow
/// (§9.2, Decision D103), so the validator has already refused one.
fn instance_policy(policy: &Policy) -> String {
    let mut text = String::from("{\n");
    if let Some(retry) = &policy.retry {
        text.push_str(&format!(
            "        retry: {},\n",
            retry_object(retry, "        ")
        ));
    }
    if let Some(timeout) = &policy.timeout {
        text.push_str(&format!(
            "        timeoutMs: {},\n",
            policy::milliseconds(&timeout.value)
        ));
    }
    match &policy.on_error {
        Some(crate::ir::policy::OnError::Fail { .. }) => {
            text.push_str("        onError: \"fail\",\n");
        }
        Some(crate::ir::policy::OnError::Skip { .. }) => {
            text.push_str("        onError: \"skip\",\n");
        }
        Some(crate::ir::policy::OnError::Fallback { .. }) | None => {}
    }
    text.push_str("      }");
    text
}

// ---------------------------------------------------------------------------
// Inputs (grammar 8.0)
// ---------------------------------------------------------------------------

fn input_builder(
    ir: &Ir,
    names: &Names,
    address: &str,
    node: &Node,
    surfaces: &[schema::Surface<'_>],
) -> String {
    let id = node.id.value.as_str();
    let reader = |field: &str| format!("`{address}` node `{id}`'s input field `{field}`");

    match &node.kind {
        NodeKind::Agent { agent } => {
            let declared = ir
                .definitions
                .get(&agent.value.to_string())
                .and_then(|definition| match &definition.body {
                    DefinitionBody::Agent(agent) => agent.input.as_ref(),
                    _ => None,
                });
            match declared {
                None => match &node.input {
                    // A string-in agent binds one unnamed value (Decision D14).
                    Some(NodeInput::Scalar { value }) => format!(
                        "  input: (roots) => runtime.toJson(runtime.evaluate({}, roots)),\n",
                        names::string(value.value.as_str())
                    ),
                    _ => "  input: () => \"\",\n".to_string(),
                },
                Some(fields) => field_map_input(ir, address, node, fields, &reader),
            }
        }
        NodeKind::Function { function } => {
            let declared = surface_fields(surfaces, &format!("{}.input", function.value));
            field_map_input(ir, address, node, declared.as_ref(), &reader)
        }
        NodeKind::Exec { .. } => match &node.input {
            Some(NodeInput::Scalar { value }) => format!(
                "  input: (roots) => runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(value.value.as_str())
            ),
            Some(NodeInput::Fields { bindings }) => {
                let mut text = String::from("  input: (roots) => ({\n");
                for binding in &bindings.entries {
                    text.push_str(&format!(
                        "    {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                        names::string(&binding.name.value),
                        names::string(binding.value.value.as_str())
                    ));
                }
                text.push_str("  }),\n");
                text
            }
            None => "  input: () => ({}),\n".to_string(),
        },
        NodeKind::Http { http } => {
            let mut text = String::from("  input: (roots) => ({\n");
            text.push_str(&http_request_fields(
                http,
                Bound::of_node(node.input.as_ref()),
                "    ",
            ));
            text.push_str("  }),\n");
            text
        }
        // A subgraph's parameter surface, bound **totally** at the instantiating
        // node: every field the subflow declares without a `default:` is bound
        // here, and nothing falls through by name (grammar 7.5, 8.5, D68).
        NodeKind::Flow { flow, .. } => {
            let declared = flow_parameters(ir, surfaces, &flow.value.to_string());
            format!(
                "  input: (roots) => {},\n",
                bound_object(node.input.as_ref(), declared.as_ref(), "  ")
            )
        }
        // A `map` node's "input" is its whole dispatch, decided before any
        // instance runs: which items, which route each takes, and what each is
        // passed (grammar 8.6). It is built here, outside the node's error
        // policy, because a per-item binding that reads an absent value fails
        // the execution rather than one item (grammar 4.1, D110).
        NodeKind::Map { .. } => format!(
            "  input: (_roots, view) => runtime.mapPlan({}, view),\n",
            names.value(&format!("{address}.node.{id}.map"))
        ),
        // A store op's parameters are its own row in grammar 11.4, and six of
        // the nine are CEL over `input`/`state`/`execution` (grammar 8.8). They
        // are evaluated **here**, in the node's input phase, for the reason
        // every node's input is built here: an expression that cannot be
        // evaluated fails the execution rather than the activity, and neither
        // `skip` nor a `fallback:` may absorb that (Decisions D78, D110).
        NodeKind::Store { params, .. } => store_params(params),
        // What the human is **shown** (grammar 8.7): a declared input surface
        // like an agent's or a tool's, bound through grammar 8.0's own chain, so
        // `input: { answer: "state.answer" }` and a field that resolves by name
        // from a channel read the same way here as anywhere else. It is built in
        // the node's input phase for that reason and for one more: a resume
        // surface has to be able to render the question, and this is the value
        // it renders (`runtime.HumanWait.shown`).
        NodeKind::Human { human } => field_map_input(ir, address, node, &human.input, &reader),
    }
    .to_string()
}

/// One store op's parameters, as the object `stores.runStoreOp` takes.
///
/// The three literals of grammar 8.8 — `top_k`, `limit`, `content_type` — are
/// written out as literals rather than evaluated: a store op's bound is a
/// written-down number, readable without running the graph, exactly as a
/// fan-out's is a schema bound.
fn store_params(params: &crate::ir::flow::StoreParams) -> String {
    let mut text = String::from("  input: (roots) => ({\n");
    for (key, expression) in [
        ("key", params.key.as_ref()),
        ("query", params.query.as_ref()),
        ("prefix", params.prefix.as_ref()),
    ] {
        let Some(expression) = expression else {
            continue;
        };
        text.push_str(&format!(
            "    {key}: String(runtime.toJson(runtime.evaluate({}, roots))),\n",
            names::string(expression.value.as_str())
        ));
    }
    match &params.value {
        // A `vector upsert`'s and a `blob put`'s value is one expression: the
        // text, or the content.
        Some(crate::ir::flow::StoreValue::Expression { value }) => {
            text.push_str(&format!(
                "    value: String(runtime.toJson(runtime.evaluate({}, roots))),\n",
                names::string(value.value.as_str())
            ));
        }
        // A `kv set`'s is a field map checked against the store's `value_schema`.
        Some(crate::ir::flow::StoreValue::Fields { bindings }) => {
            text.push_str("    value: {\n");
            for binding in &bindings.entries {
                text.push_str(&format!(
                    "      {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                    names::string(&binding.name.value),
                    names::string(binding.value.value.as_str())
                ));
            }
            text.push_str("    },\n");
        }
        None => {}
    }
    for (key, map) in [
        ("filter", params.filter.as_ref()),
        ("metadata", params.metadata.as_ref()),
    ] {
        let Some(map) = map else { continue };
        text.push_str(&format!("    {key}: {{\n"));
        for binding in &map.entries {
            text.push_str(&format!(
                "      {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(&binding.name.value),
                names::string(binding.value.value.as_str())
            ));
        }
        text.push_str("    },\n");
    }
    if let Some(top_k) = params.top_k {
        text.push_str(&format!("    topK: {top_k},\n"));
    }
    if let Some(limit) = params.limit {
        text.push_str(&format!("    limit: {limit},\n"));
    }
    if let Some(content_type) = &params.content_type {
        text.push_str(&format!(
            "    contentType: {},\n",
            names::string(&content_type.value)
        ));
    }
    text.push_str("  }),\n");
    text
}

/// An object built field by field from a binding map, over a declared surface.
///
/// The two module boundaries share it — a `flow:` node's `input:` (grammar 8.5)
/// and a `map` dispatch's field-map form (grammar 8.6 rule 12) — because both
/// are total by the same rule: a field the site bound is that expression, a
/// field it left out carries its own `default:`, and there is no third case the
/// validator lets through (D68). A surface with no fields is `{}`: the object a
/// flow declaring no `inputs:` is started with.
fn bound_object(input: Option<&NodeInput>, declared: &[Field], indent: &str) -> String {
    let bindings = match input {
        Some(NodeInput::Fields { bindings }) => Some(bindings),
        _ => None,
    };
    let mut text = String::from("({\n");
    for field in declared {
        let name = field.name.value.as_str();
        let bound = bindings.and_then(|bindings| {
            bindings
                .entries
                .iter()
                .find(|binding| binding.name.value == name)
        });
        match bound {
            Some(binding) => text.push_str(&format!(
                "{indent}  {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(name),
                names::string(binding.value.value.as_str())
            )),
            None => match schema::effective_default(&field.ty) {
                Some(default) => text.push_str(&format!(
                    "{indent}  {}: {},\n",
                    names::string(name),
                    names::literal(&default)
                )),
                // Unreachable over an artifact `build` accepted: an unbound
                // field with no `default:` is a compile error (D68).
                None => text.push_str(&format!(
                    "{indent}  // `{name}` is unbound and has no `default:`, which the validator refuses (Decision D68).\n"
                )),
            },
        }
    }
    text.push_str(&format!("{indent}}})"));
    text
}

/// The four steps of grammar 8.0's in-flow chain, resolved per field.
fn field_map_input(
    ir: &Ir,
    address: &str,
    node: &Node,
    declared: &FieldMap,
    reader: &dyn Fn(&str) -> String,
) -> String {
    let bindings = match &node.input {
        Some(NodeInput::Fields { bindings }) => Some(bindings),
        _ => None,
    };
    let channels = ir.state.as_ref();
    let flow_inputs = ir
        .definitions
        .get(address)
        .and_then(|definition| match &definition.body {
            DefinitionBody::Flow(flow) => flow.inputs.as_ref(),
            _ => None,
        });

    let mut text = String::from("  input: (roots, view) => ({\n");
    for field in &declared.fields {
        let name = field.name.value.as_str();
        let bound = bindings.and_then(|bindings| {
            bindings
                .entries
                .iter()
                .find(|binding| binding.name.value == name)
        });
        if let Some(binding) = bound {
            text.push_str(&format!(
                "    {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(name),
                names::string(binding.value.value.as_str())
            ));
            continue;
        }
        if channels.is_some_and(|state| state.entries.contains_key(name)) {
            text.push_str(&format!(
                "    {}: runtime.channelValue(view.state, {}, {}),\n",
                names::string(name),
                names::string(name),
                names::string(&reader(name))
            ));
            continue;
        }
        if flow_inputs.is_some_and(|inputs| inputs.field(name).is_some()) {
            text.push_str(&format!(
                "    {}: runtime.flowInput(view.run, {}, {}),\n",
                names::string(name),
                names::string(name),
                names::string(&reader(name))
            ));
            continue;
        }
        if let Some(default) = schema::effective_default(&field.ty) {
            text.push_str(&format!(
                "    {}: {},\n",
                names::string(name),
                names::literal(&default)
            ));
            continue;
        }
        // Grammar 8.0 step 5 is a compile error, which the validator has
        // already raised — so this is unreachable over an artifact `build`
        // accepted, and it says so rather than inventing a value.
        text.push_str(&format!(
            "    // `{name}` is unbound, which the validator refuses (grammar 8.0 step 5).\n"
        ));
    }
    text.push_str("  }),\n");
    text
}

// ---------------------------------------------------------------------------
// Activities (grammar 8)
// ---------------------------------------------------------------------------

fn activity(
    ir: &Ir,
    names: &Names,
    address: &str,
    node: &Node,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
) -> String {
    let id = node.id.value.as_str();
    // A store-op node parses nothing — see its arm below — so its derived row is
    // not imported: an import a module never reads would be noise in every
    // golden that has a store in it.
    let output_schema = output_path(ir, address, node)
        .filter(|_| !matches!(node.kind, NodeKind::Store { .. }))
        .map(|path| {
            let name = names.value(&path).to_string();
            imported.push(name.clone());
            name
        });

    match &node.kind {
        NodeKind::Agent { agent } => {
            let binding = names.value(&agent.value.to_string());
            let schema = output_schema.expect("an agent node has an output surface");
            // Where this agent execution sits, for the one tool that needs it:
            // a `flow.*` in its `tools:` instantiates a module beneath this
            // node's own frame (grammar 5.4, 9.4, PRD resolved q19), and the
            // level-1 policy that reached this instance crosses with it
            // (grammar 9.3, Decision D79). Emitted on every agent node rather
            // than only the ones with a `flow.*` attached: what a node knows
            // about its own site is not a property of its tool list, and a
            // conditional would make the emitter the thing to audit when a
            // frame comes out wrong.
            format!(
                "  run: async (input, context, view) => {{\n    \
                 const answer = await runtime.callAgent(\n      {binding},\n      input,\n      \
                 runtime.historyTurns(view.state[\"messages\"] as unknown[]),\n      context,\n      \
                 {{ path: runtime.instancePath(view, {node}), policy: view.run.policy }},\n    );\n    \
                 return {{\n      \
                 output: runtime.parseResult({schema}, answer.output, {subject}),\n      \
                 history: answer.history,\n      \
                 models: answer.models,\n      \
                 toolDispatches: answer.toolDispatches,\n    \
                 }};\n  }},\n",
                node = names::string(id),
                subject = names::string(&format!("the answer of `{}`", agent.value))
            )
        }
        NodeKind::Function { function } => {
            let tool = names.value(&function.value.to_string());
            format!(
                "  run: async (input, context) => ({{ output: await {tool}(input, context) }}),\n"
            )
        }
        NodeKind::Exec { exec } => {
            let output = surface_fields(surfaces, &format!("{address}.node.{id}.output"));
            let schema = output_schema.expect("an exec node has an output surface");
            format!(
                "  run: async (input, context) => ({{\n    output: runtime.parseResult(\n      \
                 {schema},\n      await runtime.runExec({}, input, context),\n      {subject},\n    ),\n  }}),\n",
                exec_binding(
                    exec,
                    output.as_ref(),
                    &format!("{address}.node.{id}"),
                    "      ",
                    false
                ),
                subject = names::string(&format!("the result of `{address}` node `{id}`"))
            )
        }
        NodeKind::Http { http } => {
            let output = surface_fields(surfaces, &format!("{address}.node.{id}.output"));
            let schema = output_schema.expect("an http node has an output surface");
            format!(
                "  run: async (input, context) => ({{\n    output: runtime.parseResult(\n      \
                 {schema},\n      await runtime.runHttp(\n{},\n        input as {{ query?: Record<string, unknown>; body?: unknown }},\n        context,\n      ),\n      {subject},\n    ),\n  }}),\n",
                indent_block(
                    &http_binding(
                        http,
                        output.as_ref(),
                        &format!("{address}.node.{id}"),
                        "        ",
                        false
                    ),
                    "        "
                ),
                subject = names::string(&format!("the result of `{address}` node `{id}`"))
            )
        }
        NodeKind::Flow {
            flow,
            context,
            policy,
        } => {
            let binding = names.value(&format!("{}.binding", flow.value));
            let mut text = String::from("  run: async (input, context, view) =>\n");
            text.push_str(&format!("    runtime.runSubflow({binding}, {{\n"));
            text.push_str("      inputs: input as Record<string, unknown>,\n");
            text.push_str("      execution: view.run.execution,\n");
            text.push_str(&format!(
                "      path: runtime.instancePath(view, {}),\n",
                names::string(id)
            ));
            // This node's own deadline, inside the boundary: a subgraph is the
            // one activity a `timeout:` can stop, and an instance nothing
            // aborted runs to quiescence after the node has already failed
            // (grammar 9.2, and see `runtime.runSubflow`).
            text.push_str("      signal: context.signal,\n");
            // Grammar 9.3 level 1, with D79's outermost-wins: what already
            // reached this instance beats what this site declares, per field.
            text.push_str(&format!(
                "      policy: runtime.instancePolicy(view.run.policy, {}),\n",
                policy
                    .as_ref()
                    .filter(|policy| !policy.is_empty())
                    .map_or_else(|| "undefined".to_string(), instance_policy)
            ));
            if matches!(context, Some(FlowContext::Inherit)) {
                // `context: inherit` shares the caller's history with the
                // instance; what the instance adds comes back as this node's
                // contribution to the channel (grammar 8.5, 10.4).
                text.push_str(
                    "      history: (view.state[\"messages\"] ?? []) as readonly unknown[],\n",
                );
            }
            text.push_str("    }),\n");
            text
        }
        NodeKind::Map { .. } => format!(
            "  run: async (input, context) =>\n    runtime.runMap({}, input as runtime.MapPlan, context),\n",
            names.value(&format!("{address}.node.{id}.map"))
        ),
        NodeKind::Human { .. } => format!(
            "  run: async (input, context, view) =>\n    runtime.runHuman({}, input, context, view),\n",
            names.value(&format!("{address}.node.{id}.human"))
        ),
        // The result is **not** parsed against the emitted Zod for the derived
        // row. That schema describes a shape this compiler's own runtime builds
        // rather than a contract with something outside the process, and the one
        // field grammar 11.4 marks optional — `value` on a `get` that missed —
        // is required in it, so a parse would refuse exactly the answer
        // Decision D110 says a miss gives.
        NodeKind::Store { store, op, .. } => format!(
            "  run: async (input, context, view) => ({{\n    \
             output: await stores.runStoreOp(\n      {},\n      {},\n      \
             input as stores.StoreParams,\n      context,\n      \
             {{ via: \"node\", idempotencyKey: [view.run.execution.id, ...runtime.instancePath(view, {})].join(\"/\") }},\n    \
             ),\n  }}),\n",
            names.value(&store.value.to_string()),
            names::string(op.as_str()),
            names::string(id)
        ),
    }
}

/// One `human` node's wait, as a `runtime.HumanDescriptor` (grammar 8.7).
///
/// Everything a pause needs that is not the node's own input: its budget and the
/// route its expiry takes, and the two readings of its `output:` — the published
/// JSON Schema a status route hands whoever is answering, and the emitted Zod
/// that decides whether their answer fits (PRD 5.11). Both are lowerings of one
/// field map and `schema_conformance` is what holds them equal, so a UI told the
/// contract by one is refused by the other only when it really did not fit.
///
/// The budget is **not** the node's `policy.timeoutMs` and cannot become one: a
/// `human` node resolves no `timeout:` at any of grammar 9.3's levels (D102), so
/// this is the one place a wait's duration is written.
fn human_descriptor(
    names: &Names,
    address: &str,
    node: &Node,
    human: &crate::ir::flow::Human,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
) -> String {
    let id = node.id.value.as_str();
    let output = surface_fields(surfaces, &format!("{address}.node.{id}.output"));
    let parse = names
        .value(&format!("{address}.node.{id}.output"))
        .to_string();
    imported.push(parse.clone());

    let mut text = String::from("\n");
    text.push_str(&names::doc(
        "",
        &[format!(
            "`{address}` node `{id}` — the pause it holds: what the human is shown, and \
             what an answer has to fit (grammar 8.7, PRD 5.11)."
        )],
    ));
    text.push_str(&format!(
        "const {}: runtime.HumanDescriptor = {{\n",
        names.value(&format!("{address}.node.{id}.human"))
    ));
    text.push_str(&format!("  flow: {},\n", names::string(address)));
    text.push_str(&format!("  node: {},\n", names::string(id)));
    if let Some(timeout) = &human.timeout {
        text.push_str(&format!(
            "  timeoutMs: {},\n",
            policy::milliseconds(&timeout.value)
        ));
    }
    if let Some(on_timeout) = &human.on_timeout {
        text.push_str(&format!(
            "  onTimeout: {},\n",
            names::string(&control_name_raw(&on_timeout.value))
        ));
    }
    text.push_str(&format!(
        "  schema: {},\n",
        json_literal(&schema::json_field_map(output.as_ref()), "  ")
    ));
    // The subject names the *node*, never the surface the answer arrived on.
    // One parser serves both (grammar 8.7, PRD §9.21) and its message is what
    // each surface's refusal ends in — so "the resume payload" here would tell
    // a person answering at a terminal that what they typed was a request body
    // for a route they did not use.
    text.push_str(&format!(
        "  parse: (payload) => runtime.parseResult({parse}, payload, {}),\n",
        names::string(&format!("the answer to `{address}` node `{id}`"))
    ));
    text.push_str("};\n");
    text
}

// ---------------------------------------------------------------------------
// Fan-out (grammar 8.6)
// ---------------------------------------------------------------------------

/// One `map` node's whole dispatch, as a `runtime.MapDescriptor`.
///
/// Everything a fan-out decides is data here: the array it reads, how many
/// instances may run at once, what each item is called, which route each tag
/// takes, and what each route writes. The behaviour those describe is
/// `runtime.runMap`'s, byte-identical in every project.
#[allow(clippy::too_many_arguments)]
fn map_descriptor(
    ir: &Ir,
    names: &Names,
    address: &str,
    flow: &Flow,
    node: &Node,
    map: &Map,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
) -> String {
    let id = node.id.value.as_str();
    let binding = map
        .item_binding
        .as_ref()
        .map_or("item", |name| name.value.as_str());
    let item = map_item_type(ir, address, flow, surfaces, map);

    let mut text = String::from("\n");
    text.push_str(&names::doc(
        "",
        &[format!(
            "`{address}` node `{id}` — the fan-out it dispatches (grammar 8.6). \
             `over` resolves against `{}`, and `{}` is what an instance's own \
             bindings call the item.",
            map.over.value.as_str(),
            binding
        )],
    ));
    text.push_str(&format!(
        "const {}: runtime.MapDescriptor = {{\n",
        names.value(&format!("{address}.node.{id}.map"))
    ));
    text.push_str(&format!("  node: {},\n", names::string(id)));
    text.push_str(&format!("  as: {},\n", names::string(binding)));

    // The producer half of `over` (grammar 8.6 rule 11): the node whose result
    // the path reads, if it reads one. `input` and `state` are roots the node
    // already has, so those paths name no producer.
    let root = map.over.value.root.as_str();
    let producer = flow
        .nodes
        .iter()
        .find(|candidate| candidate.id.value.as_str() == root);
    text.push_str("  source: {\n");
    text.push_str(&format!(
        "    path: {},\n",
        names::string(map.over.value.as_str())
    ));
    match producer {
        Some(producer) => {
            let path = output_path(ir, address, producer)
                .expect("`over` reads a node whose result the validator resolved");
            let fields = surface_fields(surfaces, &path);
            text.push_str(&format!("    producer: {},\n", names::string(root)));
            text.push_str(&format!(
                "    shape: {},\n",
                cel::shape_of_field_map(fields.as_ref(), "    ")
            ));
        }
        None => text.push_str("    shape: \"any\",\n"),
    }
    text.push_str("  },\n");

    text.push_str(&format!("  maxConcurrency: {},\n", map.max_concurrency));
    text.push_str(&format!(
        "  onItemError: {},\n",
        match &map.on_item_error {
            None | Some(ItemError::Fail { .. }) => "\"fail\"".to_string(),
            Some(ItemError::Skip { .. }) => "\"skip\"".to_string(),
            Some(ItemError::Retry { retry, .. }) =>
                format!("{{ retry: {} }}", retry_object(retry, "  ")),
        }
    ));

    match &map.dispatch {
        MapDispatch::Homogeneous {
            node: target,
            input,
            writes,
            detach,
        } => {
            text.push_str("  routes: [\n");
            text.push_str(&dispatch_route(
                ir,
                names,
                surfaces,
                imported,
                &DispatchSite {
                    target: &target.value,
                    tag: None,
                    max_concurrency: map.max_concurrency,
                    input: input.as_ref(),
                    writes: writes.as_ref(),
                    detach: detach.as_ref().is_some_and(|detach| detach.value),
                    binding,
                    item: item.as_ref(),
                },
                "    ",
            ));
            text.push_str("  ],\n");
        }
        MapDispatch::Routed {
            route_by,
            routes,
            default,
        } => {
            text.push_str(&format!(
                "  routeBy: {},\n",
                names::string(route_by.value.as_str())
            ));
            let union = item.as_ref().and_then(|item| match &item.form {
                TypeForm::Union(union) => Some(union),
                _ => None,
            });
            text.push_str("  routes: [\n");
            for route in routes {
                let tag = route
                    .tag
                    .as_ref()
                    .expect("a named route carries its variant tag");
                // The narrowing of grammar 8.6 rule 4: the item this route sees
                // is its **own variant's payload**, which is the shape it was
                // type-checked against and so the shape it is bound through.
                let narrowed = union.and_then(|union| {
                    union
                        .variants
                        .iter()
                        .find(|variant| variant.tag.value == tag.value)
                        .map(|variant| {
                            crate::check::maps::narrowed(
                                &union.discriminator.value,
                                &[tag.value.as_str()],
                                &variant.fields,
                                &route.span,
                            )
                        })
                });
                text.push_str(&dispatch_route(
                    ir,
                    names,
                    surfaces,
                    imported,
                    &DispatchSite {
                        target: &route.node.value,
                        tag: Some(tag.value.as_str()),
                        max_concurrency: route.max_concurrency.unwrap_or(map.max_concurrency),
                        input: route.input.as_ref(),
                        writes: route.writes.as_ref(),
                        detach: route.detach.as_ref().is_some_and(|detach| detach.value),
                        binding,
                        item: narrowed.as_ref(),
                    },
                    "    ",
                ));
            }
            text.push_str("  ],\n");

            if let Some(default) = default {
                // The catch-all sees the **unrouted** variants: the
                // discriminator narrowed to their tags, plus the fields every
                // one of them declares identically (grammar 8.6 rule 4, D30).
                let served: Vec<&str> = routes
                    .iter()
                    .filter_map(|route| route.tag.as_ref())
                    .map(|tag| tag.value.as_str())
                    .collect();
                let narrowed = union.map(|union| {
                    let unrouted: Vec<&crate::ir::schema::UnionVariant> = union
                        .variants
                        .iter()
                        .filter(|variant| !served.contains(&variant.tag.value.as_str()))
                        .collect();
                    let tags: Vec<&str> = unrouted
                        .iter()
                        .map(|variant| variant.tag.value.as_str())
                        .collect();
                    crate::check::maps::narrowed(
                        &union.discriminator.value,
                        &tags,
                        &crate::check::maps::common_fields(&unrouted, &default.span),
                        &default.span,
                    )
                });
                let route = dispatch_route(
                    ir,
                    names,
                    surfaces,
                    imported,
                    &DispatchSite {
                        target: &default.node.value,
                        // Not `default`: a route's tag is a **variant tag**, and
                        // a union may declare a variant called `default` beside
                        // a `default:` catch-all — two different routes, legal
                        // together, that `selectRoute` tells apart (named routes
                        // are searched first) and a trace could not. The sigil
                        // is the settlement: grammar §2.1's identifier is
                        // `lower , { lower | digit | "_" }`, so no variant tag
                        // an author can spell reaches this spelling.
                        tag: Some(CATCH_ALL_TAG),
                        max_concurrency: default.max_concurrency.unwrap_or(map.max_concurrency),
                        input: default.input.as_ref(),
                        writes: default.writes.as_ref(),
                        detach: default.detach.as_ref().is_some_and(|detach| detach.value),
                        binding,
                        item: narrowed.as_ref(),
                    },
                    "  ",
                );
                // The shared renderer writes a list element; the catch-all is a
                // single value with a key of its own.
                text.push_str(&format!("  fallback: {}", route.trim_start()));
            }
        }
    }

    text.push_str("};\n");
    text
}

/// What a `map`'s `default:` catch-all is called in a dispatch record.
///
/// A reserved spelling rather than `default`, because a route's tag is otherwise
/// a variant tag the author chose and one of them may *be* `default` — see the
/// note at the emission site. `$` is outside grammar §2.1's identifier, so this
/// name collides with nothing a composition can declare.
const CATCH_ALL_TAG: &str = "$default";

/// One dispatch target of a `map`, and everything it decides (grammar 8.6).
struct DispatchSite<'ir> {
    target: &'ir Address,
    tag: Option<&'ir str>,
    max_concurrency: i64,
    input: Option<&'ir NodeInput>,
    writes: Option<&'ir Writes>,
    detach: bool,
    /// The map's `as:` name, which is what the whole-item form reads.
    binding: &'ir str,
    /// The item type this target sees — narrowed to its variant on a route.
    item: Option<&'ir TypeNode>,
}

/// One `runtime.MapRoute`, at this indentation.
fn dispatch_route(
    ir: &Ir,
    names: &Names,
    surfaces: &[schema::Surface<'_>],
    imported: &mut Vec<String>,
    site: &DispatchSite<'_>,
    indent: &str,
) -> String {
    let target = site.target.to_string();
    let inner = format!("{indent}  ");
    let mut text = format!("{indent}{{\n");
    if let Some(tag) = site.tag {
        text.push_str(&format!("{inner}tag: {},\n", names::string(tag)));
    }
    text.push_str(&format!("{inner}target: {},\n", names::string(&target)));
    text.push_str(&format!(
        "{inner}maxConcurrency: {},\n",
        site.max_concurrency
    ));
    text.push_str(&format!("{inner}detach: {},\n", site.detach));
    text.push_str(&format!(
        "{inner}itemShape: {},\n",
        site.item.map_or_else(
            || "\"any\"".to_string(),
            |item| cel::shape_of_type(item, &inner)
        )
    ));

    // The per-item binding (grammar 8.6 rule 12, Decision D75). Both `input:`
    // forms are legal here, and which one fits is the target's input contract:
    // the scalar form binds a string-in agent's single unnamed value, the field
    // map binds a declared object field by field, and no `input:` at all passes
    // the whole item.
    let contract = target_contract(ir, surfaces, &target);
    text.push_str(&match (site.input, &contract) {
        (Some(NodeInput::Scalar { value }), _) => format!(
            "{inner}input: (roots) => runtime.toJson(runtime.evaluate({}, roots)),\n",
            names::string(value.value.as_str())
        ),
        (Some(NodeInput::Fields { .. }), Contract::Fields(fields)) => format!(
            "{inner}input: (roots) => {},\n",
            bound_object(site.input, fields.as_ref(), &inner)
        ),
        // No `input:`: the whole item is the instance's input, which the
        // validator has already held to the target's contract.
        (None, _) | (Some(NodeInput::Fields { .. }), Contract::StringIn) => format!(
            "{inner}input: (roots) => runtime.toJson(runtime.evaluate({}, roots)),\n",
            names::string(site.binding)
        ),
    });

    text.push_str(&dispatch_run(
        ir,
        names,
        imported,
        &target,
        site.detach,
        &inner,
    ));
    text.push_str(&if site.detach {
        // A detached dispatch MUST NOT write reduced state and MUST NOT declare
        // `writes:` (grammar 8.6 rule 7, Decisions D31, D94) — including the
        // name-based half, which the validator refuses too. The list is empty
        // rather than computed, because the join is over before the delivery is,
        // so a write it made could only land after everything that reads it.
        format!("{inner}writes: [],\n")
    } else {
        target_output_path(ir, &target).map_or_else(
            || format!("{inner}writes: [],\n"),
            |path| {
                let fields = surface_fields(surfaces, &path);
                write_list(ir, fields.as_ref(), site.writes, &inner)
            },
        )
    });
    text.push_str(&format!("{indent}}},\n"));
    text
}

/// How one dispatched instance is run: an agent call, a tool invocation, or a
/// subflow instantiation (grammar 8.6, and the target's own section).
///
/// `detach` decides one thing here: whether the call is handed the idempotency
/// key of grammar 9.4. It goes to a **`tool.*`** and to nothing else, because
/// that is the construct PRD 5.6's sentence is about — "an `idempotency_key` …
/// is passed to the sink automatically, and sinks are documented to dedupe on
/// it" — and grammar 9.4's delivery surface is written per binding kind, which
/// only a `tool.*` has. The other two targets are left alone deliberately, and
/// the `a-detached-dispatch-is-keyed-and-nothing-else-is` row in this module's
/// ledger is why. Which slot the key travels in is the runtime's: see
/// `runtime.delivering`, `runtime.runHttp` and `runtime.runExec`.
fn dispatch_run(
    ir: &Ir,
    names: &Names,
    imported: &mut Vec<String>,
    target: &str,
    detach: bool,
    indent: &str,
) -> String {
    let Some(definition) = ir.definitions.get(target) else {
        return format!("{indent}run: () => Promise.resolve({{ output: {{}} }}),\n");
    };
    match &definition.body {
        DefinitionBody::Agent(_) => {
            let binding = names.value(target);
            let schema = names.value(&format!("{target}.output")).to_string();
            imported.push(schema.clone());
            // An empty history, and nothing written back to the caller's: a
            // dispatched instance runs on a fresh conversation that is discarded
            // when it completes, and there is no key to say otherwise
            // (grammar 10.4, Decision D105).
            // The dispatch's own instance path is this agent's site, so a
            // `flow.*` in its `tools:` instantiates beneath the item rather
            // than beneath the map node (grammar 9.4, PRD resolved q19). No
            // policy travels with it: grammar 8.6 rule 10 resolves a dispatched
            // instance's nodes with level 1 absent, and what the agent's own
            // tool loop starts is inside that instance.
            format!(
                "{indent}run: async (input, context, site) => {{\n{indent}  \
                 const answer = await runtime.callAgent({binding}, input, [], context, {{ path: site.path }});\n{indent}  \
                 return {{\n{indent}    \
                 output: runtime.parseResult({schema}, answer.output, {subject}),\n{indent}    \
                 models: answer.models,\n{indent}    \
                 toolDispatches: answer.toolDispatches,\n{indent}  \
                 }};\n{indent}\
                 }},\n",
                subject = names::string(&format!("the answer of `{target}`"))
            )
        }
        // The sink of a detached dispatch, handed the key it delivers.
        DefinitionBody::Tool(_) if detach => format!(
            "{indent}run: async (input, context, site) => ({{\n{indent}  \
             output: await {}(input, runtime.delivering(context, site)),\n{indent}\
             }}),\n",
            names.value(target)
        ),
        DefinitionBody::Tool(_) => format!(
            "{indent}run: async (input, context) => ({{ output: await {}(input, context) }}),\n",
            names.value(target)
        ),
        DefinitionBody::Flow(_) => {
            let binding = names.value(&format!("{target}.binding"));
            // Grammar 8.6 rule 10: a dispatched subflow's nodes resolve their
            // policy with **level 1 absent** — a `map` has no `policy:` key — so
            // no override crosses this boundary, and its history is fresh and
            // discarded (D105), so no `history:` does either.
            //
            // The dispatch's clock does cross: `context.signal` is the map
            // node's on a joined dispatch, so its `timeout:` ends the instance
            // rather than only the wait for it, and the delivery's own — which
            // nothing aborts — on a detached one (D94, `runtime.runSubflow`).
            format!(
                "{indent}run: async (input, context, site) =>\n{indent}  \
                 runtime.runSubflow({binding}, {{\n{indent}    \
                 inputs: input as Record<string, unknown>,\n{indent}    \
                 execution: site.execution,\n{indent}    \
                 path: site.path,\n{indent}    \
                 signal: context.signal,\n{indent}  \
                 }}),\n"
            )
        }
        DefinitionBody::Provider(_) | DefinitionBody::Model(_) | DefinitionBody::Store(_) => {
            format!("{indent}run: () => Promise.resolve({{ output: {{}} }}),\n")
        }
    }
}

/// What a dispatch target accepts (grammar 8.0, 8.6 rule 12).
enum Contract<'ir> {
    /// A string-in agent: one unnamed value (Decision D14).
    StringIn,
    /// A declared input object, as the fields bound field by field. A flow with
    /// no `inputs:` is this with no entries, not a missing contract.
    Fields(Cow<'ir, [Field]>),
}

/// The input contract of one `agent.*`, `tool.*` or `flow.*` target.
fn target_contract<'ir>(ir: &Ir, surfaces: &[schema::Surface<'ir>], target: &str) -> Contract<'ir> {
    let Some(definition) = ir.definitions.get(target) else {
        return Contract::StringIn;
    };
    match &definition.body {
        DefinitionBody::Agent(agent) if agent.input.is_none() => Contract::StringIn,
        DefinitionBody::Agent(_) | DefinitionBody::Tool(_) => Contract::Fields(fields_of(
            surface_fields(surfaces, &format!("{target}.input")),
        )),
        DefinitionBody::Flow(_) => Contract::Fields(flow_parameters(ir, surfaces, target)),
        DefinitionBody::Provider(_) | DefinitionBody::Model(_) | DefinitionBody::Store(_) => {
            Contract::StringIn
        }
    }
}

/// The parameter surface of one `flow.*`: what an instantiating site binds.
///
/// A flow with no `inputs:` has the surface `inputs: {}` declares — a closed
/// object with no properties (grammar 3.9) — and it is *written nowhere*, so
/// [`schema::surfaces`] emits no `<flow>.inputs` for it and there is no field
/// map to look up. That is not an absence for each use site to discover: it is
/// the empty parameter list, and an instance is started with an empty object.
///
/// Both module boundaries resolve their target's parameters here — a `flow:`
/// node (grammar 8.5) and a `map` dispatch onto a flow (grammar 8.6 rule 12) —
/// so an inputs-less subflow reads the same from either position. Answering
/// only at the `map` boundary is how the `flow:` node position came to reach
/// [`surface_fields`]'s panic on a composition `validate` accepts, which
/// `a_flow_node_instantiates_a_subflow_that_declares_no_inputs` pins.
fn flow_parameters<'ir>(
    ir: &Ir,
    surfaces: &[schema::Surface<'ir>],
    address: &str,
) -> Cow<'ir, [Field]> {
    let declares_inputs =
        ir.definitions
            .get(address)
            .is_some_and(|definition| match &definition.body {
                DefinitionBody::Flow(flow) => flow.inputs.is_some(),
                _ => false,
            });
    if declares_inputs {
        fields_of(surface_fields(surfaces, &format!("{address}.inputs")))
    } else {
        Cow::Borrowed(&[])
    }
}

/// A field map's entries, keeping whatever the map itself was borrowed as.
fn fields_of(map: Cow<'_, FieldMap>) -> Cow<'_, [Field]> {
    match map {
        Cow::Borrowed(map) => Cow::Borrowed(&map.fields),
        Cow::Owned(map) => Cow::Owned(map.fields),
    }
}

/// The canonical path of a dispatch target's result surface.
fn target_output_path(ir: &Ir, target: &str) -> Option<String> {
    match ir
        .definitions
        .get(target)
        .map(|definition| &definition.body)
    {
        Some(DefinitionBody::Agent(_) | DefinitionBody::Tool(_)) => {
            Some(format!("{target}.output"))
        }
        Some(DefinitionBody::Flow(_)) => Some(format!("{target}.outputs")),
        _ => None,
    }
}

/// The item type a `map` fans out over: what `over` resolves to, one step past
/// the array (grammar 4.2, 8.6 rule 1).
///
/// The walk is [`crate::check::maps::walk`]'s, and the roots are read through
/// the same surface enumeration the emitted schemas come from — so the type the
/// item is *bound* through at run time is the type it was *checked* against.
fn map_item_type(
    ir: &Ir,
    address: &str,
    flow: &Flow,
    surfaces: &[schema::Surface<'_>],
    map: &Map,
) -> Option<TypeNode> {
    let path = &map.over.value;
    let steps = &path.steps;
    let (root, consumed) = match path.root.as_str() {
        "state" => {
            let crate::ast::common::PathStep::Field(name) = steps.first()? else {
                return None;
            };
            let channel = ir.state.as_ref()?.entries.get(name.as_str())?;
            (channel.ty.clone(), 1)
        }
        "input" => {
            let crate::ast::common::PathStep::Field(name) = steps.first()? else {
                return None;
            };
            (flow.inputs.as_ref()?.field(name.as_str())?.ty.clone(), 1)
        }
        id => {
            let producer = flow
                .nodes
                .iter()
                .find(|node| node.id.value.as_str() == id)?;
            let output = surface_fields(surfaces, &output_path(ir, address, producer)?);
            let (
                crate::ast::common::PathStep::Field(selector),
                crate::ast::common::PathStep::Field(name),
            ) = (steps.first()?, steps.get(1)?)
            else {
                return None;
            };
            if selector.as_str() != "output" {
                return None;
            }
            (output.field(name.as_str())?.ty.clone(), 2)
        }
    };
    let resolved = crate::check::maps::walk(root, &steps[consumed.min(steps.len())..])?;
    match resolved.form {
        TypeForm::Array(array) => Some(*array.items),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// `exec` and `http` blocks
// ---------------------------------------------------------------------------

fn exec_binding(
    exec: &Exec,
    output: &FieldMap,
    site: &str,
    indent: &str,
    tool_surface: bool,
) -> String {
    let inner = format!("{indent}  ");
    let mut text = String::from("{\n");
    text.push_str(&format!(
        "{inner}command: {},\n",
        interpolation(&exec.command.value, &format!("{site}.exec.command"))
    ));
    if exec.args.is_empty() {
        text.push_str(&format!("{inner}args: [],\n"));
    } else {
        text.push_str(&format!("{inner}args: [\n"));
        for (index, argument) in exec.args.iter().enumerate() {
            text.push_str(&format!(
                "{inner}  {},\n",
                interpolation(&argument.value, &format!("{site}.exec.args[{index}]"))
            ));
        }
        text.push_str(&format!("{inner}],\n"));
    }
    if let Some(cwd) = &exec.cwd {
        text.push_str(&format!(
            "{inner}cwd: {},\n",
            interpolation(&cwd.value, &format!("{site}.exec.cwd"))
        ));
    }
    if exec.env.is_empty() {
        text.push_str(&format!("{inner}env: [],\n"));
    } else {
        text.push_str(&format!("{inner}env: [\n"));
        for entry in &exec.env {
            text.push_str(&format!(
                "{inner}  {{ name: {}, value: {} }},\n",
                names::string(&entry.name.value),
                interpolation(
                    &entry.value.value,
                    &format!("{site}.exec.env.{}", entry.name.value)
                )
            ));
        }
        text.push_str(&format!("{inner}],\n"));
    }
    let expected = if exec.expect_exit.is_empty() {
        vec![0]
    } else {
        exec.expect_exit.clone()
    };
    text.push_str(&format!(
        "{inner}expectExit: [{}],\n",
        expected
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    ));
    text.push_str(&format!(
        "{inner}decoding: {},\n",
        decoding(output, EXEC_ENVELOPE, tool_surface)
    ));
    text.push_str(&format!("{indent}}}"));
    text
}

fn http_binding(
    http: &Http,
    output: &FieldMap,
    site: &str,
    indent: &str,
    tool_surface: bool,
) -> String {
    let inner = format!("{indent}  ");
    let mut text = String::from("{\n");
    text.push_str(&format!(
        "{inner}method: {},\n",
        names::string(http.method.value.as_str())
    ));
    text.push_str(&format!(
        "{inner}url: {},\n",
        interpolation(&http.url.value, &format!("{site}.http.url"))
    ));
    if http.headers.is_empty() {
        text.push_str(&format!("{inner}headers: [],\n"));
    } else {
        text.push_str(&format!("{inner}headers: [\n"));
        for entry in &http.headers {
            text.push_str(&format!(
                "{inner}  {{ name: {}, value: {} }},\n",
                names::string(&entry.name.value),
                interpolation(
                    &entry.value.value,
                    &format!("{site}.http.headers.{}", entry.name.value)
                )
            ));
        }
        text.push_str(&format!("{inner}],\n"));
    }
    text.push_str(&format!(
        "{inner}expectStatus: {},\n",
        if http.expect_status.is_empty() {
            "\"2xx\"".to_string()
        } else {
            format!(
                "[{}]",
                http.expect_status
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    ));
    text.push_str(&format!(
        "{inner}decoding: {},\n",
        decoding(output, HTTP_ENVELOPE, tool_surface)
    ));
    text.push_str(&format!("{indent}}}"));
    text
}

/// The envelope fields an inline `exec:` node's result binds (grammar 8.2).
const EXEC_ENVELOPE: &[&str] = &["exit_code", "stdout", "stderr"];
/// The envelope fields an inline `http:` node's result binds (grammar 8.3).
const HTTP_ENVELOPE: &[&str] = &["status", "body"];

/// How a result is read out (grammar 6.1's exception, 8.2 and 8.3's envelopes).
fn decoding(output: &FieldMap, envelope: &[&str], tool_surface: bool) -> String {
    if output.is_empty() {
        return "{ envelope: [], decoded: [], empty: true }".to_string();
    }
    // Grammar 6.1: a `tool.*` whose `output` declares exactly one property and
    // that property is string-typed takes the raw stream whole. The count is
    // over the whole property set (Decision D109), and the exception is a
    // `tool.*`-surface rule (Decision D91).
    //
    // *How* whole is the implementation's, and the two surfaces differ: grammar
    // 6.1 says "trimmed raw stdout" for `exec` and "the raw response text" for
    // `http`, so the descriptor names the property and `runtime.decode` — where
    // the stream is — takes each sentence at its word. See its doc comment, and
    // `a_raw_binding_trims_stdout_and_takes_a_response_body_verbatim`.
    if tool_surface
        && output.fields.len() == 1
        && matches!(
            &output.fields[0].ty.form,
            crate::ir::schema::TypeForm::Scalar(scalar)
                if scalar.kind == crate::ast::schema::ScalarKind::String
        )
    {
        return format!(
            "{{ envelope: [], decoded: [], raw: {}, empty: false }}",
            names::string(output.fields[0].name.value.as_str())
        );
    }
    let mut bound: Vec<String> = Vec::new();
    let mut decoded: Vec<String> = Vec::new();
    for field in &output.fields {
        let name = field.name.value.as_str();
        if !tool_surface && envelope.contains(&name) {
            bound.push(names::string(name));
        } else {
            decoded.push(names::string(name));
        }
    }
    format!(
        "{{ envelope: [{}], decoded: [{}], empty: false }}",
        bound.join(", "),
        decoded.join(", ")
    )
}

/// What fills the half of a request the `http:` block did not write out
/// (grammar 6.1's *bound input object*, 8.3's *Request payload*, D66).
///
/// The two surfaces bind different things, which is why this is an enum rather
/// than an `Option<&Bindings>`:
///
/// * a **`tool.*`** has a declared `input:` schema, and the object the caller's
///   arguments parsed into *is* the bound object — it goes on the wire whole;
/// * an **inline node** has no input schema of its own, so its `input:`
///   bindings build an ad-hoc object field by field, each one a flow-scoped CEL
///   expression evaluated against `roots` (grammar 8.3). The field-map form is
///   the only legal one there (D88), so a scalar `input:` reaches this as
///   [`Bound::Nothing`] — the validator has already refused it.
enum Bound<'a> {
    /// A `tool.*`'s own parsed `input`.
    ToolInput,
    /// An inline node's `input:` bindings.
    NodeFields(&'a Bindings),
    /// Nothing to fall back on: the block wrote the request out in full, or
    /// there is no input at all.
    Nothing,
}

impl<'a> Bound<'a> {
    /// What an inline node's `input:` binds, if anything.
    fn of_node(input: Option<&'a NodeInput>) -> Self {
        match input {
            Some(NodeInput::Fields { bindings }) => Self::NodeFields(bindings),
            _ => Self::Nothing,
        }
    }
}

/// A tool binding's request: its `query:`/`body:` values over the tool's own
/// `input` (grammar 6.1, Decision D65), and the input object itself in whichever
/// of the two the binding left unwritten.
fn http_request(http: &Http, bound: Bound<'_>, indent: &str) -> String {
    let mut text = String::from("{\n");
    text.push_str(&http_request_fields(http, bound, &format!("{indent}  ")));
    text.push_str(&format!("{indent}}}"));
    text
}

/// One `query`/`body` slot written out from explicit bindings.
fn request_slot(slot: &str, bindings: &Bindings, indent: &str) -> String {
    let mut text = format!("{indent}{slot}: {{\n");
    for binding in &bindings.entries {
        text.push_str(&format!(
            "{indent}  {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
            names::string(&binding.name.value),
            names::string(binding.value.value.as_str())
        ));
    }
    text.push_str(&format!("{indent}}},\n"));
    text
}

/// The `query` and `body` an `http:` surface sends (grammar 6.1, 8.3).
///
/// Each slot is the block's own key where it declared one, and otherwise the
/// bound input object — as the body on a body-bearing method and as the query
/// string on `GET`/`HEAD`, which is the convention grammar 6.1 states and 8.3
/// reuses. The two never compete for one slot: D66 makes declaring both a
/// compile error, so the `else` here is reached only where the block is silent.
fn http_request_fields(http: &Http, bound: Bound<'_>, indent: &str) -> String {
    let mut text = String::new();
    let body_bearing = !matches!(
        http.method.value,
        crate::ast::binding::HttpMethod::Get | crate::ast::binding::HttpMethod::Head
    );

    // `query` before `body`, whichever of them the input object filled: the
    // emitted object's key order is the request's rather than the method's.
    for (slot, declared) in [("query", http.query.as_ref()), ("body", http.body.as_ref())] {
        if let Some(bindings) = declared {
            text.push_str(&request_slot(slot, bindings, indent));
            continue;
        }
        // The input object goes to the body on a body-bearing method and to the
        // query string on `GET`/`HEAD`; the other slot stays unsent.
        if (slot == "body") != body_bearing {
            continue;
        }
        match &bound {
            Bound::ToolInput => text.push_str(&format!("{indent}{slot}: input,\n")),
            Bound::NodeFields(bindings) => text.push_str(&request_slot(slot, bindings, indent)),
            Bound::Nothing => {}
        }
    }
    text
}

// ---------------------------------------------------------------------------
// Writes (grammar 8.0, 10.3) and edges (grammar 7.2, 7.3, 7.4)
// ---------------------------------------------------------------------------

fn writes(ir: &Ir, address: &str, node: &Node, surfaces: &[schema::Surface<'_>]) -> String {
    let Some(path) = output_path(ir, address, node) else {
        return "  writes: [],\n".to_string();
    };
    let fields = surface_fields(surfaces, &path);
    write_list(ir, fields.as_ref(), node.writes.as_ref(), "  ")
}

/// One effective write map (grammar 8.0, 10.3), as the runtime's descriptors.
///
/// Shared by the three sites that have one — a node, a `map`'s homogeneous
/// dispatch, and each route of a routed one — because "the remap, then the
/// channel of the same name" is one rule and a second spelling of it would be a
/// second answer about which channel a field lands in.
fn write_list(ir: &Ir, fields: &FieldMap, remap: Option<&Writes>, indent: &str) -> String {
    let remap: BTreeMap<&str, &str> = remap
        .map(|writes| {
            writes
                .entries
                .iter()
                .map(|entry| (entry.field.value.as_str(), entry.channel.value.as_str()))
                .collect()
        })
        .unwrap_or_default();

    let mut text = String::new();
    for field in &fields.fields {
        let name = field.name.value.as_str();
        let channel = match remap.get(name) {
            Some(channel) => Some(*channel),
            // Name-based wiring: a field with no remap writes the channel of
            // the same name, if one is declared (grammar 10.3).
            None => ir
                .state
                .as_ref()
                .and_then(|state| state.entries.contains_key(name).then_some(name)),
        };
        let Some(channel) = channel else { continue };
        let reduce = ir
            .state
            .as_ref()
            .and_then(|state| state.entries.get(channel))
            .and_then(|channel| channel.reduce)
            .map_or("set", |reduce| match reduce {
                crate::ast::document::Reduce::Append => "append",
                crate::ast::document::Reduce::Merge => "merge",
                crate::ast::document::Reduce::LastWins => "set",
            });
        text.push_str(&format!(
            "{indent}  {{ field: {}, channel: {}, reduce: {} }},\n",
            names::string(name),
            names::string(channel),
            names::string(reduce)
        ));
    }
    if text.is_empty() {
        format!("{indent}writes: [],\n")
    } else {
        format!("{indent}writes: [\n{text}{indent}],\n")
    }
}

fn edges(ir: &Ir, address: &str, flow: &Flow, node: &Node) -> String {
    let id = node.id.value.as_str();
    let _ = ir;
    let mut text = String::from("  edges: [\n");
    for (index, edge) in flow.edges.iter().enumerate() {
        if !leaves(edge, id) {
            continue;
        }
        text.push_str(&edge_descriptor(address, edge, index, Some(id)));
    }
    text.push_str("  ],\n");
    text
}

fn edge_descriptor(address: &str, edge: &Edge, index: usize, source: Option<&str>) -> String {
    let mut text = format!("    {{ to: {}", target_name(&edge.to.value));
    if let Some(when) = &edge.when {
        text.push_str(&format!(", when: {}", names::string(when.value.as_str())));
        if let Some(source) = source
            && cel::reads_output_of(when.value.as_str(), source)
        {
            text.push_str(", readsOutput: true");
        }
    }
    if edge.else_edge.is_some() {
        text.push_str(", otherwise: true");
    }
    if let Some(max) = edge.max_iterations {
        // One counter per bounded **edge** (grammar 7.4): an SCC carrying two
        // budgeted edges carries two counters, one budget each. The key is the
        // edge's own place in the flow's `edges:` list, which is stable and
        // unique however the edge is spelled.
        text.push_str(&format!(
            ", budget: {{ key: {}, max: {max} }}",
            names::string(&format!("{address}#{index}"))
        ));
    }
    text.push_str(" },\n");
    text
}

// ---------------------------------------------------------------------------
// The registry, and how a flow is invoked
// ---------------------------------------------------------------------------

fn registry_source(
    ir: &Ir,
    names: &Names,
    registry: &[(String, String)],
    imported: &mut Vec<String>,
) -> String {
    let mut text = String::from(REGISTRY_DOC);
    text.push_str("export const flows: Readonly<Record<string, CompiledFlow>> = {\n");
    for (address, _) in registry {
        let Some(definition) = ir.definitions.get(address) else {
            continue;
        };
        let DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        let compiled = names.value(&format!("{address}.graph")).to_string();
        text.push_str(&format!("  {}: {{\n", names::string(address)));
        text.push_str(&format!("    address: {},\n", names::string(address)));
        text.push_str(&format!(
            "    inputs: [{}],\n",
            flow.inputs
                .as_ref()
                .map(|inputs| inputs
                    .fields
                    .iter()
                    .map(|field| names::string(field.name.value.as_str()))
                    .collect::<Vec<_>>()
                    .join(", "))
                .unwrap_or_default()
        ));
        text.push_str("    inputKinds: {");
        let kinds: Vec<String> = flow
            .inputs
            .as_ref()
            .map(|inputs| {
                inputs
                    .fields
                    .iter()
                    .map(|field| {
                        format!(
                            " {}: {},",
                            names::string(field.name.value.as_str()),
                            names::string(input_kind(&field.ty))
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        text.push_str(&kinds.join(""));
        text.push_str(if kinds.is_empty() { "},\n" } else { " },\n" });
        text.push_str(&format!(
            "    outputs: [{}],\n",
            flow.outputs
                .fields
                .iter()
                .map(|field| names::string(field.name.value.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        text.push_str(&format!(
            "    sessionStores: [{}],\n",
            session_stores(ir, address)
                .iter()
                .map(|store| names::string(store))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        text.push_str(&format!("    recursionLimit: {},\n", recursion_limit(flow)));
        text.push_str(&match &flow.inputs {
            Some(_) => {
                let schema = names.value(&format!("{address}.inputs")).to_string();
                imported.push(schema.clone());
                // Through `runtime.parseResult` rather than the schema's own
                // `.parse`, for the reason every other result is: the message is
                // what a caller acts on. This is the surface an invocation
                // arrives at — `--input k=v` from the CLI, one decoded payload
                // from the app — and grammar 13.2 asks it to fail "naming the
                // field", which a schema library's own multi-line dump does only
                // in the sense that the name is somewhere inside it (PRD G3).
                format!(
                    "    parse: (inputs: unknown) =>\n      runtime.parseResult({schema}, inputs, {}) as Record<string, unknown>,\n",
                    names::string(&format!("the `inputs:` of `{address}`"))
                )
            }
            // A flow with no `inputs:` takes none, so an invocation supplies
            // nothing and there is no schema to parse against (grammar 7.5).
            None => "    parse: () => ({}),\n".to_string(),
        });
        // `outputKeys` is what `invoke` passes on this graph's behalf, so the
        // values streamed here are the ones an invocation would have answered
        // with rather than a wider view of the state.
        text.push_str(&format!(
            "    stream: (initial, options) =>\n      \
             {compiled}.stream(initial, {{\n        \
             ...options,\n        \
             streamMode: \"values\",\n        \
             outputKeys: {compiled}.outputChannels,\n      \
             }}) as unknown as Promise<AsyncIterable<GraphState>>,\n"
        ));
        text.push_str("  },\n");
    }
    text.push_str("};\n");
    text.push_str(RUN_FLOW);
    text
}

/// How a `--input` value is read for one declared field (grammar 13.2).
fn input_kind(ty: &TypeNode) -> &'static str {
    match &ty.form {
        TypeForm::Scalar(scalar) => match scalar.kind {
            crate::ast::schema::ScalarKind::String => "string",
            crate::ast::schema::ScalarKind::Integer => "integer",
            crate::ast::schema::ScalarKind::Number => "number",
            crate::ast::schema::ScalarKind::Boolean => "boolean",
        },
        // An enum is a closed set of strings (Decision D9), so its argument is
        // one of them written out — text, not a JSON document.
        TypeForm::Enum(_) => "string",
        TypeForm::Object(_) | TypeForm::Array(_) | TypeForm::Union(_) => "json",
    }
}

/// The `scope: session` stores a flow reaches (grammar 7.7, 11.3).
///
/// The same relation the validator's session-coherence check quantifies over,
/// read here rather than restated: two spellings of one traversal is how they
/// come to disagree about which flows need a session key (Decision D86).
fn session_stores(ir: &Ir, address: &str) -> Vec<String> {
    crate::check::reach::stores_of(ir, address)
        .into_iter()
        .filter(|store| {
            matches!(
                ir.definitions.get(store).map(|definition| &definition.body),
                Some(DefinitionBody::Store(store))
                    if store.scope == crate::ast::definition::StoreScope::Session
            )
        })
        .collect()
}

/// The passes a cycle carrying no counting bound is given, per node in it.
///
/// Grammar 7.4's two clauses are not the same kind of statement: clause 1's
/// `max_iterations` is a number the composition declares, so a ceiling can be
/// *derived* from it, while clause 2's CEL exit condition declares nothing —
/// grammar 7.4 and PRD 5.4 both say outright that it is not a static termination
/// proof. So this is a policy number, and the only honest way to pick one is to
/// say what it is between: an order of magnitude above the revision counts PRD
/// 5.4's own loops describe (evaluator-optimizer, plan-revise, ReAct), and an
/// order of magnitude below the 1000 a counting bound may declare — generous
/// enough that no loop a reader would call reasonable meets it, small enough
/// that a guard which never goes false stops in bounded time rather than
/// running forever. [`RUN_FLOW`]'s `recursionLimit` option raises it for one
/// run, and `runtime.SuperstepCeiling` is what says so when the net catches a
/// run.
const CEL_BOUNDED_PASSES: i64 = 100;

/// How many supersteps a flow instance may take before LangGraph refuses.
///
/// A safety net rather than a semantic bound: the bounds that decide a run are
/// the composition's own (grammar 7.4). Sized from both of that section's
/// clauses, because they are proofs of different strength:
///
/// * every **counting** bound can be spent in full — one pass per budget through
///   every node — with room for the acyclic part on either side;
/// * every cycle bounded **only** by a CEL exit condition gets
///   [`CEL_BOUNDED_PASSES`] passes through each of its own nodes, since nothing
///   in the composition says how many it takes.
///
/// The second is what keeps the net off the loops PRD 5.4 exists for: a review
/// cycle carrying no `max_iterations` is bounded by grammar 7.4 clause 2 and by
/// nothing this function can count, and sizing it as though it ran once would
/// stop a legitimate run at `25 + |nodes|` supersteps.
fn recursion_limit(flow: &Flow) -> i64 {
    let budgets: i64 = flow
        .edges
        .iter()
        .filter_map(|edge| edge.max_iterations)
        .sum();
    let graph = CheckedGraph::new(flow);
    let unproven: i64 = graph
        .components()
        .iter()
        .filter(|members| {
            members
                .first()
                .is_some_and(|first| graph.cyclic(*first) && !counted(&graph, members))
        })
        .map(|members| members.len() as i64)
        .sum();
    25 + (flow.nodes.len() as i64) * (1 + budgets) + unproven * CEL_BOUNDED_PASSES
}

const REGISTRY_DOC: &str = r#"
/**
 * How a declared flow input reads a command-line value (grammar 13.2).
 *
 * `json` is every structured shape — an object, an array, a tagged union — for
 * which the one honest reading of a shell argument is the document it spells.
 */
export type InputKind = "string" | "integer" | "number" | "boolean" | "json";

/** One compiled flow: what it takes, what it answers, and how to run it. */
export interface CompiledFlow {
  /** Its typed address (grammar 2.2). */
  readonly address: string;
  /** The fields its `inputs:` declares (grammar 7.5). */
  readonly inputs: readonly string[];
  /**
   * What each declared input *is*, so a `--input k=v` argument can be read as
   * the type the field declares rather than reaching Zod as text (grammar 13.2).
   */
  readonly inputKinds: Readonly<Record<string, InputKind>>;
  /** The fields its `outputs:` declares, each read from the channel of that name. */
  readonly outputs: readonly string[];
  /**
   * The `scope: session` stores this flow **reaches**, under the relation
   * grammar 7.7 fixes — its own nodes, its maps' dispatch targets, the flows it
   * instantiates, and the stores of every agent it reaches.
   *
   * A run of this flow needs a session identity exactly when this list is not
   * empty (grammar 11.3): a declared trigger supplies it through `session_key:`,
   * and the CLI through `--session`. Checked at run start rather than at
   * validate, for the reason env-ref presence is: the value does not exist until
   * the invocation does.
   */
  readonly sessionStores: readonly string[];
  /** The superstep ceiling a run of it takes by default. */
  readonly recursionLimit: number;
  /** Parse an invocation's inputs against the flow's own schema (grammar 13.2). */
  parse(inputs: unknown): Record<string, unknown>;
  /**
   * Invoke the compiled graph, answering with the state after each superstep.
   *
   * A stream rather than a plain invocation because of what a **failure** must
   * leave behind. LangGraph's `invoke` is this stream with the last value kept,
   * and an error thrown out of it discards the state it was keeping — the
   * routing trace of PRD 5.3 with it, since nothing here is checkpointed. Taking
   * the supersteps one at a time keeps every one that did complete, which is
   * what `runFlow` reports the failure with.
   */
  stream(
    initial: Record<string, unknown>,
    options: { recursionLimit: number },
  ): Promise<AsyncIterable<GraphState>>;
}

/**
 * Every flow this composition declares, by address.
 *
 * PRD 5.11 makes manual invocation universal — every flow is runnable whether or
 * not a `manual` trigger names it (Decision D64) — so the registry is every
 * flow rather than every triggered one.
 */
"#;

const RUN_FLOW: &str = r#"
/** What one run produced. */
export interface FlowRun {
  /** The flow's `outputs:`, materialized from state at quiescence (grammar 7.6.3). */
  readonly outputs: Record<string, unknown>;
  /** Every routing decision the run made, in step order (PRD 5.3). */
  readonly trace: readonly runtime.TraceEntry[];
  /** The whole state at quiescence. */
  readonly state: GraphState;
}

/**
 * Why a flow that reaches a `scope: session` store refuses a run that arrived
 * with no session identity (grammar 11.3, 13.2).
 *
 * One sentence in one place, because it is raised from two and the two must not
 * drift: `src/cli.ts` raises it as a **usage** error, before a `run` starts
 * anything, because a missing `--session` is an argument the caller has to add —
 * the same class as an unknown `--input` name or an absent `${ENV}`, which
 * grammar 11.3 says outright by likening this check to env-ref presence (§4.3);
 * `runFlow` raises it for every other caller, where the key arrives per
 * invocation.
 *
 * Named by the store rather than by the flow, because the store is what the
 * author has to look at. And three ways a run arrives with none are named,
 * because two of them are advice a reader has already taken: a declared
 * `session_key:` that *evaluated* to the empty string — an absent header, a
 * payload member that was not sent — is an identity-less run whose trigger does
 * declare one, and a message offering only the two remedies would send that
 * reader to look at a line that is already there (PRD G3).
 */
export function sessionRefusal(address: string, stores: readonly string[]): string {
  return `\`${address}\` reaches ${stores.map((store) => `\`${store}\``).join(", ")}, which ${stores.length === 1 ? "is" : "are"} \`scope: session\`, so this run needs a session identity and arrived with none: pass \`--session <key>\` to \`agent-compose run\`, or declare \`session_key:\` on the trigger that starts it — and where one is declared, it answered the empty string for this invocation (grammar 11.3, 13.2)`;
}

/**
 * Run one flow to quiescence and materialize its outputs.
 *
 * This is the invocation surface `agent-compose run` and the generated `serve`
 * app are built on (PRD 5.11's `start`), and what an ejected project calls
 * directly. The inputs are parsed against the flow's own `inputs:` schema before
 * anything runs, which is where an invocation that the flow cannot accept is
 * refused by field name (grammar 13.2).
 *
 * A run that produces no answer raises `runtime.FlowFailure`, which carries the
 * trace it did make and the original error as its `cause` — see
 * `CompiledFlow.stream` for why the run is streamed to keep it. Both of the ways
 * that happens raise it: a run that never reached quiescence, and a run that
 * reached quiescence holding no value for one of its `outputs:` fields
 * (grammar 10.1, Decision D78). The second is the one whose trace is complete —
 * every step landed — so dropping it there would lose the whole routing record
 * of a run that made one.
 *
 * A third way is a `human` node (grammar 8.7): a run that reaches one and was
 * started with **no answer surface** stops there, and the `FlowFailure` carries
 * a `runtime.HumanInterrupt` on its `cause` chain — `runtime.interruptOf` is how
 * a caller tells that outcome from a failure. `resumable: true` is what says an
 * answer surface is attached, and there are two callers that pass it, one per
 * surface (PRD §9.21): `src/serve.ts`, whose `POST /executions/:id/resume`
 * delivers the answer, and `src/cli.ts`, when the `run` it is serving may ask
 * at the terminal it was launched from.
 *
 * # Every invocation is journaled
 *
 * PRD resolved q26–q29: the run's effects are written to this project's journal
 * as they happen, and `resume: true` re-runs the graph consuming that record
 * read-only up to the frontier. Journaling is unconditional — every target this
 * compiler builds is process-local and binds the SQLite journal beside the
 * project (resolved q27) — so a caller that says nothing about durability still
 * gets it. `docs/durability.md` is normative.
 */
export async function runFlow(
  address: string,
  inputs: unknown = {},
  options: {
    readonly executionId?: string;
    readonly sessionKey?: string;
    readonly recursionLimit?: number;
    /**
     * Whether something is standing by to answer a `human` pause this run
     * reaches (grammar 8.7, PRD 5.11).
     *
     * `false` — the default, and what an `agent-compose run` with nobody to ask
     * leaves it at — makes a pause the end of the run rather than a wait
     * nothing can settle. `src/cli.ts` passes `true` where the run may ask at
     * its own standard input, and `src/serve.ts` passes it for the resume
     * route.
     */
    readonly resumable?: boolean;
    /**
     * What started this execution, for the journal's lifecycle row.
     *
     * `manual` where nothing says otherwise, which is every `agent-compose run`
     * and every `manual` trigger; `src/serve.ts` passes the `http` trigger's own
     * name. Recorded and never dispatched on — recovery replays executions, it
     * does not re-fire triggers (PRD resolved q28).
     */
    readonly trigger?: string;
    /**
     * The completion webhook of an `async` `http` trigger, resolved against the
     * request that started this execution (grammar 13.3).
     *
     * `src/serve.ts` passes it and nothing else does. It goes on the journal's
     * lifecycle row because the process that finishes an execution need not be
     * the one that started it: a `serve` that restarts mid-run recovers the
     * execution and has to be able to call the caller back
     * (`docs/durability.md` §6.1).
     */
    readonly callback?: string;
    /**
     * Whether this is a **resumed** generation of an execution the journal
     * already holds (PRD resolved q29).
     *
     * `true` makes every effect site consult the journal before it calls the
     * world: a recorded answer is returned byte for byte and nothing is
     * re-issued, until the frontier — the first effect the journal does not
     * hold — where the execution goes live again. `executionId` names which
     * execution, and `inputs`/`sessionKey` must be the ones the journal
     * recorded, which is why the two callers that resume read them back off the
     * lifecycle row rather than composing them again.
     */
    readonly resume?: boolean;
  } = {},
): Promise<FlowRun> {
  const flow = flows[address];
  if (flow === undefined) {
    throw new Error(
      `\`${address}\` is not a flow of this composition: ${Object.keys(flows).join(", ")}`,
    );
  }
  const parsed = flow.parse(inputs);
  const ceiling = options.recursionLimit ?? flow.recursionLimit;
  const sessionKey = options.sessionKey ?? "";
  // Grammar 11.3, checked where the value first exists: a flow that reaches a
  // `scope: session` store keys off the identity its trigger supplies, and a run
  // started without one would silently address a partition named by the empty
  // string. `src/cli.ts` decides the same thing one step earlier for a `run`,
  // where the identity is a command-line argument; this is the guard for every
  // caller it cannot stand in for — a `serve` request, an ejected invocation —
  // where the key arrives per invocation and its absence really is a failure of
  // that run rather than of the command.
  if (sessionKey === "" && flow.sessionStores.length > 0) {
    throw new Error(sessionRefusal(address, flow.sessionStores));
  }
  const executionId = options.executionId ?? `exec_${globalThis.crypto.randomUUID()}`;
  // Before the graph is streamed, so an execution the process dies in the middle
  // of already has a row saying it was open (PRD resolved q28).
  await runtime.openExecution({
    execution: executionId,
    flow: address,
    trigger: options.trigger ?? "manual",
    inputs: parsed,
    sessionKey,
    ...(options.callback === undefined ? {} : { callback: options.callback }),
    ...(options.resume === true ? { resuming: true } : {}),
  });
  try {
    const produced = await quiesceFlow(address, flow, parsed, ceiling, sessionKey, executionId, options);
    // A run that reached quiescence may still be holding a divergence raised
    // where nothing could throw it — a detached `map` delivery, which grammar 8.6
    // rule 7 says the flow instance does not wait for. PRD resolved q29 makes a
    // divergence un-absorbable by any policy at any nesting depth, and `detach:`
    // is one, so it fails the resume here rather than being reported as a
    // completion the record does not support.
    const diverged = runtime.latchedDivergence(executionId);
    if (diverged !== undefined) throw diverged;
    runtime.settleExecution(executionId);
    return produced;
  } catch (error) {
    // Which of the three closing rows this is — `failed`, or none at all
    // because the run is parked at a `human` pause and is exactly what a resume
    // exists for — is `runtime.settleExecution`'s to decide, off the same
    // `runtime.interruptOf` this function's own callers read.
    runtime.settleExecution(executionId, error);
    throw error;
  } finally {
    runtime.closeExecution(executionId);
  }
}

/** [`runFlow`]'s body, with the journal's lifecycle row already open. */
async function quiesceFlow(
  address: string,
  flow: CompiledFlow,
  parsed: Record<string, unknown>,
  ceiling: number,
  sessionKey: string,
  executionId: string,
  options: { readonly resumable?: boolean },
): Promise<FlowRun> {
  // Opened before the graph is streamed, so a status route asked the instant
  // after `start` answered already has somewhere to read this run's pauses from
  // (grammar 8.7, PRD 5.11). Every instance nested inside the run registers
  // against the same execution id and is told apart by its instance path.
  runtime.openHumanWaits(executionId, options.resumable === true);
  // `scope: execution` means what it says: whatever this run's own stores held
  // is released when the run ends, however it ended (PRD 5.8, grammar 11.1).
  // A `serve` process runs many executions, so a store that stayed open would
  // be both a leak and a lifetime the composition did not declare. A pause the
  // run was holding goes the same way and for the same reason: a wait that
  // outlived its run would be one a resume could still be delivered to, with
  // no graph left to receive it (grammar 8.7).
  //
  // **Unless the run has not ended.** An execution whose journal row stays open
  // — parked at a `human` pause with nobody to answer it, or stopped by a
  // divergence — is one a resume replays, and `runtime.staysOpen` is the very
  // predicate `runtime.settleExecution` decides that by. What such a run owns
  // *on disk* has to still be there when the resumed generation reads past the
  // frontier: a replayed write is never applied a second time, so a partition
  // this generation deleted would answer an empty `get` about something the
  // execution wrote, with nothing comparing unequal to catch it
  // (`docs/durability.md` §5).
  const release = async (outcome: unknown): Promise<void> => {
    const parked = runtime.staysOpen(executionId, outcome);
    runtime.releaseHumanWaits(executionId);
    // And the deliveries nothing joined. A detached `map` delivery is journaled
    // when it answers, so a generation that walks out from under one in flight
    // leaves an effect with no record — which the generation that resumes this
    // execution issues a second time (`docs/durability.md` §3.2). Only where
    // there *is* going to be one: a run that ended waits for nothing, which is
    // grammar 8.6 rule 7 read where it applies.
    if (parked) await runtime.settleDetached(executionId);
    stores.releaseExecution(executionId, parked);
  };
  // `runtime.quiesce` keeps the last state each superstep produced, which is
  // what makes a failure's trace survive; the one failure it restates on the way
  // out is LangGraph stopping the run at the ceiling. Its answer carries the
  // run's own failure rather than throwing it, so the release reads that error
  // on the settled path and the thrown one on the other — the same outcome
  // either way, which is what a `finally` could not have been told.
  const { state, error } = await runtime
    .quiesce(
      flow,
      {
        $run: {
          ...runtime.emptyRun(),
          input: parsed,
          execution: { id: executionId, session_key: sessionKey },
        },
      },
      ceiling,
    )
    .then(
      async (reached) => {
        await release(reached.error);
        return reached;
      },
      async (thrown: unknown) => {
        await release(thrown);
        throw thrown;
      },
    );
  if (error !== undefined) {
    throw new runtime.FlowFailure(
      address,
      "did not run to quiescence",
      runtime.failedTrace(state, error),
      error,
    );
  }
  if (state === undefined) {
    // Unreachable: `streamMode: "values"` emits the state the run started from
    // before any node has run. A run with no state at all is still not one this
    // function can answer for, and saying so beats reading `undefined` as empty.
    throw new Error(`\`${address}\` produced no state`);
  }
  const quiesced = state;

  const outputs: Record<string, unknown> = {};
  try {
    for (const field of flow.outputs) {
      outputs[field] = runtime.channelValue(
        quiesced as unknown as Record<string, unknown>,
        field,
        `\`${address}\`'s output field \`${field}\``,
      );
    }
  } catch (error) {
    // Inside the same record as every other way a run fails to answer: the run
    // got all the way here, so `$run.trace` holds every step it took, and that
    // is the routing record a reader wants most when the flow cannot say what it
    // produced (PRD 5.3).
    throw new runtime.FlowFailure(
      address,
      "reached quiescence without an output",
      quiesced.$run.trace,
      error,
    );
  }
  return { outputs, trace: quiesced.$run.trace, state: quiesced };
}

/**
 * A new builder over this composition's state model.
 *
 * Every flow's topology is added to one of these. Constructing it is also what
 * proves the state model is a shape LangGraph accepts — a channel spec it
 * refuses fails here rather than at the first invocation.
 */
export function createBuilder() {
  return new StateGraph(State);
}
"#;

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Whether this edge leaves the node with this id.
fn leaves(edge: &Edge, id: &str) -> bool {
    matches!(&edge.from.value, EdgeSource::Node(node) if node.as_str() == id)
}

/// Whether this edge leaves `start`.
fn starts(edge: &Edge) -> bool {
    matches!(edge.from.value, EdgeSource::Start)
}

/// The node id an edge target names, for an `addEdge`.
fn node_of(target: &EdgeTarget) -> &str {
    match target {
        EdgeTarget::Node(node) => node.as_str(),
        EdgeTarget::End => "__end__",
    }
}

/// An edge target as TypeScript source: a quoted node id, or `END`.
fn target_name(target: &EdgeTarget) -> String {
    match target {
        EdgeTarget::Node(node) => names::string(node.as_str()),
        EdgeTarget::End => "END".to_string(),
    }
}

/// A control-transfer target as TypeScript source (grammar 9.2, 8.7).
fn control_name(target: &ControlTarget) -> String {
    match target {
        ControlTarget::Node(node) => names::string(node.as_str()),
        ControlTarget::End => "END".to_string(),
    }
}

/// The same target as a bare string, for a descriptor's `fallback` field.
fn control_name_raw(target: &ControlTarget) -> String {
    match target {
        ControlTarget::Node(node) => node.as_str().to_string(),
        ControlTarget::End => "__end__".to_string(),
    }
}

/// The canonical path of a node's result surface, if it has one.
fn output_path(ir: &Ir, address: &str, node: &Node) -> Option<String> {
    let id = node.id.value.as_str();
    match &node.kind {
        NodeKind::Agent { agent } => Some(format!("{}.output", agent.value)),
        NodeKind::Function { function } => Some(format!("{}.output", function.value)),
        NodeKind::Flow { flow, .. } => Some(format!("{}.outputs", flow.value)),
        NodeKind::Exec { .. } | NodeKind::Http { .. } | NodeKind::Human { .. } => {
            Some(format!("{address}.node.{id}.output"))
        }
        NodeKind::Store { store, .. } => ir
            .definitions
            .contains_key(&store.value.to_string())
            .then(|| format!("{address}.node.{id}.output")),
        // A `map` node has no output of its own (grammar 8.6 rule 9).
        NodeKind::Map { .. } => None,
    }
}

/// The field map of one surface, by canonical path.
///
/// [`schema::surfaces`] is the single enumeration of every schema there is —
/// including the three that appear nowhere in the source text (an inline node's
/// kind default, a store op's derived row) — so reading it here is what keeps
/// this module and the emitted Zod naming one shape.
fn surface_fields<'ir>(surfaces: &[schema::Surface<'ir>], path: &str) -> Cow<'ir, FieldMap> {
    surfaces
        .iter()
        .find(|surface| surface.path == path)
        .and_then(|surface| match &surface.body {
            schema::Body::Fields(fields) => Some(fields.clone()),
            schema::Body::Type(_) => None,
        })
        .unwrap_or_else(|| {
            panic!("no schema surface is emitted for `{path}`");
        })
}

/// An interpolable string as the runtime's parts (grammar 4.3 class 2).
fn interpolation(text: &Interpolated, site: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut literal = String::new();
    let raw = text.as_str();
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$' {
            // `$${` is the escape producing a literal `${` (grammar 4.3).
            if bytes.get(index + 1) == Some(&b'$') {
                if bytes.get(index + 2) == Some(&b'{') {
                    literal.push_str("${");
                    index += 3;
                } else {
                    literal.push_str("$$");
                    index += 2;
                }
                continue;
            }
            if bytes.get(index + 1) == Some(&b'{')
                && let Some(offset) = raw[index..].find('}')
            {
                let name = &raw[index + 2..index + offset];
                if crate::parse::lexical::is_env_name(name) {
                    if !literal.is_empty() {
                        parts.push(names::string(&literal));
                        literal.clear();
                    }
                    parts.push(format!(
                        "{{ env: {}, site: {} }}",
                        names::string(name),
                        names::string(site)
                    ));
                    index += offset + 1;
                    continue;
                }
            }
        }
        let character = raw[index..].chars().next().expect("a character boundary");
        literal.push(character);
        index += character.len_utf8();
    }
    if !literal.is_empty() {
        parts.push(names::string(&literal));
    }
    format!("[{}]", parts.join(", "))
}

/// A JSON value as a TypeScript literal, indented to sit inside an object.
///
/// JSON is a subset of TypeScript's object-literal syntax, so a schema and a
/// shape are both emitted this way — quoted keys and all, which is what makes
/// them legible as the data they are.
pub(super) fn json_literal(value: &serde_json::Value, indent: &str) -> String {
    let rendered = serde_json::to_string_pretty(value).expect("a schema serializes");
    indent_block(&rendered, indent)
}

/// Re-indent a multi-line block so every line after the first sits at `indent`.
fn indent_block(text: &str, indent: &str) -> String {
    let mut lines = text.lines();
    let mut out = String::from(lines.next().unwrap_or_default());
    for line in lines {
        out.push('\n');
        out.push_str(indent);
        out.push_str(line);
    }
    out
}

/// Every host-registered function a composition names, for the emitted README.
///
/// Grammar 6.1's `function:` binding is the escape hatch that breaks spec
/// portability: the implementation lives outside the composition, and a project
/// that uses one does not run until the host has registered it. The README lists
/// them by name so that is a sentence a reader finds rather than a runtime
/// error they hit.
#[must_use]
pub fn host_functions(ir: &Ir) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (address, definition) in &ir.definitions {
        if let DefinitionBody::Tool(Tool {
            implementation: ToolImplementation::Function { function },
            ..
        }) = &definition.body
        {
            found.push((function.name.value.as_str().to_string(), address.clone()));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    /// The emitted module for one composition, through the real name registry.
    fn emit(source: &str) -> String {
        let ir = ir_of(source);
        let mut names = Names::of(&ir);
        declare(&mut names, &ir);
        module(&ir, &names).contents
    }

    const PREAMBLE: &str = r#"version: "0.1"

state:
  draft: { type: string, default: "" }
  feedback: { type: string, default: "" }
  notes:
    type: array
    max_items: 4
    items: { type: string }
    reduce: append

provider.p:
  kind: anthropic
  api_key: ${MODEL_KEY}

model.m:
  provider: provider.p
  id: some-model

agent.reviewer:
  model: model.m
  prompt: Review it.
  input:
    goal: { type: string }
    draft: { type: string }
  output:
    verdict: { enum: [approve, revise] }
    feedback: { type: string }
"#;

    /// Grammar 7.3 evaluates a node's outgoing edges in **declaration order**,
    /// so that is the order they are emitted in — with the guard, the `else:`
    /// marker and the budget each carried as data rather than as control flow.
    #[test]
    fn a_nodes_edges_are_emitted_in_declaration_order_with_their_guards() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    write: {{ agent: agent.reviewer }}
    review: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: write }}
    - {{ from: write, to: review }}
    - {{ from: review, to: write, when: "review.output.verdict == 'revise'", max_iterations: 3 }}
    - {{ from: review, to: end, else: true }}
"#
        ));
        let edges = emitted
            .split("const flowFNodeReview: runtime.NodeDescriptor")
            .nth(1)
            .expect("the node is emitted")
            .split("edges: [")
            .nth(1)
            .expect("its edges are emitted")
            .split("],")
            .next()
            .expect("the list closes");
        let lines: Vec<&str> = edges
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect();
        assert_eq!(
            lines,
            [
                "{ to: \"write\", when: \"review.output.verdict == 'revise'\", readsOutput: true, \
                 budget: { key: \"flow.f#2\", max: 3 } },",
                "{ to: END, otherwise: true },",
            ]
        );
    }

    /// One counter per bounded **edge** (grammar 7.4): an SCC carrying two
    /// budgeted edges carries two counters, one budget each.
    #[test]
    fn each_bounded_edge_gets_its_own_counter() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    write: {{ agent: agent.reviewer }}
    review: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: write }}
    - {{ from: write, to: review }}
    - {{ from: review, to: write, when: "review.output.verdict == 'revise'", max_iterations: 3 }}
    - {{ from: review, to: write, when: "review.output.feedback != ''", max_iterations: 5 }}
    - {{ from: review, to: end, else: true }}
"#
        ));
        assert!(
            emitted.contains("budget: { key: \"flow.f#2\", max: 3 }"),
            "{emitted}"
        );
        assert!(
            emitted.contains("budget: { key: \"flow.f#3\", max: 5 }"),
            "{emitted}"
        );
    }

    /// Grammar 8.0's chain, resolved per field: an explicit binding, a channel
    /// of the same name, the enclosing flow input, then the field's own default.
    #[test]
    fn each_input_field_is_resolved_through_grammar_eight_zeros_chain() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
agent.mixed:
  model: model.m
  prompt: Mix it.
  input:
    goal: {{ type: string }}
    draft: {{ type: string }}
    feedback: {{ type: string }}
    tone: {{ type: string, default: calm }}
  output:
    verdict: {{ enum: [approve, revise] }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    mix:
      agent: agent.mixed
      input:
        feedback: "'explicit'"
  edges:
    - {{ from: start, to: mix }}
    - {{ from: mix, to: end }}
"#
        ));
        let builder = emitted
            .split("const flowFNodeMix: runtime.NodeDescriptor")
            .nth(1)
            .expect("the node is emitted")
            .split("run:")
            .next()
            .expect("the input builder closes");
        // The flow input of the same name (step 3)…
        assert!(
            builder.contains("\"goal\": runtime.flowInput(view.run, \"goal\","),
            "{builder}"
        );
        // …the channel of the same name (step 2)…
        assert!(
            builder.contains("\"draft\": runtime.channelValue(view.state, \"draft\","),
            "{builder}"
        );
        // …the explicit binding (step 1), which beats the channel of that name…
        assert!(
            builder
                .contains("\"feedback\": runtime.toJson(runtime.evaluate(\"'explicit'\", roots))"),
            "{builder}"
        );
        // …and the field's own `default:` (step 4).
        assert!(builder.contains("\"tone\": \"calm\""), "{builder}");
    }

    /// A string-in agent binds one unnamed value (Decision D14), which is a
    /// scalar rather than an object.
    #[test]
    fn a_string_in_agent_is_bound_with_the_scalar_form() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
agent.plain:
  model: model.m
  prompt: Answer.
  output: {{ draft: {{ type: string }} }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    say: {{ agent: agent.plain, input: "input.goal" }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: end }}
"#
        ));
        assert!(
            emitted.contains(
                "input: (roots) => runtime.toJson(runtime.evaluate(\"input.goal\", roots)),"
            ),
            "{emitted}"
        );
    }

    /// Grammar 9.3's chain is applied here, and the emitted policy says which
    /// level each field came from — the artifact keeps them unresolved, so the
    /// comment is where a reader learns what won.
    #[test]
    fn the_resolved_policy_names_the_level_each_field_came_from() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
defaults:
  timeout: 30s
  on_error: fail

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    review:
      agent: agent.reviewer
      retry: {{ max: 2, backoff: 1s, jitter: false }}
      on_error: {{ fallback: end }}
  edges:
    - {{ from: start, to: review }}
    - {{ from: review, to: end }}
"#
        ));
        assert!(
            emitted.contains(
                "// Grammar 9.3, resolved: `retry` from the node, `timeout` from `defaults:`, \
                 `on_error` from the node."
            ),
            "{emitted}"
        );
        assert!(emitted.contains("backoffMs: 1000,"), "{emitted}");
        assert!(emitted.contains("jitter: false,"), "{emitted}");
        assert!(emitted.contains("timeoutMs: 30000,"), "{emitted}");
        assert!(
            emitted.contains("onError: { fallback: \"__end__\" },"),
            "{emitted}"
        );
    }

    /// A `human` node's whole wait is written down: its budget, the route its
    /// expiry takes, and what an answer is held to (grammar 8.7, PRD 5.11).
    ///
    /// The budget is the one worth pinning by value. Grammar 9.3 keeps a `human`
    /// node's `timeout` out of *every* level (D102), so `timeoutMs` on the
    /// descriptor and no `timeoutMs` in the node's `policy:` is the whole of that
    /// decision: a wait's duration is written in one place, and a
    /// composition-wide budget can never reach it.
    #[test]
    fn a_human_nodes_descriptor_carries_its_wait_its_route_and_its_answer_schema() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
defaults:
  timeout: 30s

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    ask:
      human:
        input: {{ question: {{ type: string }} }}
        output: {{ decision: {{ enum: [approve, reject] }} }}
        timeout: 24h
        on_timeout: rescue
      input: {{ question: "input.goal" }}
    rescue: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: ask }}
    - {{ from: ask, to: end }}
    - {{ from: rescue, to: end }}
"#
        ));
        assert!(
            emitted.contains("const flowFNodeAskHuman: runtime.HumanDescriptor = {"),
            "{emitted}"
        );
        assert!(emitted.contains("  timeoutMs: 86400000,\n"), "{emitted}");
        assert!(emitted.contains("  onTimeout: \"rescue\",\n"), "{emitted}");
        assert!(
            emitted.contains(
                "  parse: (payload) => runtime.parseResult(flowFNodeAskOutput, payload, \
                 \"the answer to `flow.f` node `ask`\"),"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains("runtime.runHuman(flowFNodeAskHuman, input, context, view)"),
            "{emitted}"
        );
        // What the human is shown is the node's own `input:`, bound through
        // grammar 8.0's chain like any other declared input surface.
        assert!(
            emitted
                .contains("\"question\": runtime.toJson(runtime.evaluate(\"input.goal\", roots)),"),
            "{emitted}"
        );
        // Decision D102: `defaults: { timeout: 30s }` above reaches every other
        // node and not this one, so the node's resolved policy carries no
        // budget at all — the wait's is the descriptor's.
        assert!(
            emitted.contains(
                "// Grammar 9.3, resolved: `retry` from exempt (Decision D102), \
                 `timeout` from exempt (Decision D102), `on_error` from the built-in."
            ),
            "{emitted}"
        );
        // Grammar 7.8 clause 3: a node reached only by `on_timeout` is live
        // code, and the graph has to be told so or it will not compile.
        assert!(
            emitted.contains("ends: [END, \"rescue\"],"),
            "the control-transfer target is an end of the node that transfers to it:\n{emitted}"
        );
    }

    /// A flow attached as a tool is **on the wire** with the contract grammar
    /// 5.4 gives it, and a call to it instantiates the module.
    ///
    /// The two halves are one rule. A tool the compiler drops is a model that
    /// cannot call what the composition attached and a run that says nothing
    /// about it — no diagnostic at `validate`, none at `build`, and an agent the
    /// validator analysed the attachment of (grammar 7.7 clause 4) reaching the
    /// provider without it. So the name, the description grammar 5.4 requires
    /// here, and the `inputs:` schema are all emitted; the `invoke` is where the
    /// call is served, by handing `runtime.callSubflowTool` the three things a
    /// module-level binding can know — which flow, under which name, and the
    /// emitted Zod its arguments are parsed with (PRD 5.1, resolved q19/q20).
    #[test]
    fn a_flow_attached_as_a_tool_is_offered_to_the_model_and_instantiates_on_a_call() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
agent.caller:
  model: model.m
  prompt: Answer.
  output: {{ draft: {{ type: string }} }}
  tools: [flow.helper, flow.bare]

flow.helper:
  description: Summarise one passage into a line.
  inputs: {{ passage: {{ type: string, min_length: 1 }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    write: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: write }}
    - {{ from: write, to: end }}

flow.bare:
  description: Answer with no arguments at all.
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    write: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: write }}
    - {{ from: write, to: end }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    say: {{ agent: agent.caller }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: end }}
"#
        ));
        // The local name is the name on the wire, and the address travels with
        // it so a transcript says which flow was attached.
        assert!(emitted.contains("      name: \"helper\","), "{emitted}");
        assert!(
            emitted.contains("      address: \"flow.helper\","),
            "{emitted}"
        );
        // Grammar 5.4: the flow's own `description:` is the selection signal.
        assert!(
            emitted.contains("      description: \"Summarise one passage into a line.\","),
            "{emitted}"
        );
        // …and its `inputs:` is the parameter schema, `min_length:` included.
        assert!(emitted.contains("\"minLength\": 1"), "{emitted}");
        // The call is served: the module the tool instantiates, and the emitted
        // Zod for the same `inputs:` the JSON column above constrained the model
        // with — constrain == parse, at the one surface a model supplies the
        // arguments (PRD §9.16).
        assert!(
            emitted.contains(
                "{ name: \"helper\", binding: flowHelperBinding, inputs: flowHelperInputs }"
            ),
            "{emitted}"
        );
        // A flow with no `inputs:` is a no-argument tool rather than a panic:
        // the empty parameter list is written nowhere, so there is no field map
        // to look up and no schema to name (see `flow_parameters`).
        assert!(emitted.contains("      name: \"bare\","), "{emitted}");
        assert!(
            emitted.contains("{ name: \"bare\", binding: flowBareBinding }"),
            "{emitted}"
        );
        // The construct is emitted rather than refused: the wiring above is what
        // the tool does, and no refusal path is left beside it.
        assert!(!emitted.contains("Unimplemented"), "{emitted}");
    }

    /// A `tool.*`'s two usage surfaces parse its `input:` differently, and the
    /// difference is who hears about a mismatch (grammar 6, Decision D119).
    ///
    /// One definition, two call sites: the emitted function is shared, so the
    /// parse that can *bounce* has to sit where only a model reaches it. At the
    /// attachment it is `parseToolArguments`, which raises the `ToolCallRefused`
    /// the loop hands back, under the **local** name — the only spelling the
    /// model can call again. Inside the function, which is also what a
    /// `function:` node calls, it stays `parseResult`: grammar 8.4 has already
    /// checked that binding field-by-field, nothing there proposes a call, and a
    /// class whose whole meaning is "the model was handed this" would be a lie
    /// on a node that just ended.
    #[test]
    fn a_tools_arguments_are_refused_to_a_model_and_mismatched_for_a_graph() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
tool.lookup:
  description: Look one thing up.
  input: {{ query: {{ type: string, min_length: 1 }} }}
  output: {{ snippet: {{ type: string }} }}
  exec: {{ command: printf, args: ["a snippet"] }}

agent.caller:
  model: model.m
  prompt: Answer.
  output: {{ draft: {{ type: string }} }}
  tools: [tool.lookup]

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    say: {{ agent: agent.caller }}
    check:
      function: tool.lookup
      input: {{ query: "state.draft" }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: check }}
    - {{ from: check, to: end }}
"#
        ));
        // The graph's surface: the tool's own parse, naming the component the
        // way a person reading a node failure needs it named.
        assert!(
            emitted.contains(
                "  const input = runtime.parseResult(toolLookupInput, args, \"the arguments `tool.lookup` was called with\");"
            ),
            "{emitted}"
        );
        // The model's surface: one level out, refusable, and named `lookup` —
        // the name the wire offered, which `name:` above is the same string as.
        assert!(emitted.contains("      name: \"lookup\","), "{emitted}");
        assert!(
            emitted.contains(
                "      invoke: (args, context) =>\n        toolLookup(\n          runtime.parseToolArguments(toolLookupInput, args, \"the arguments `lookup` was called with\"),\n          context,\n        ),"
            ),
            "{emitted}"
        );
        // …and nothing on the graph's side reaches the refusable parse.
        assert!(
            emitted.contains(
                "  run: async (input, context) => ({ output: await toolLookup(input, context) }),"
            ),
            "{emitted}"
        );
    }

    /// An `agent:` node hands `callAgent` the site its tool loop instantiates
    /// beneath (grammar 9.4, PRD resolved q19).
    ///
    /// Emitted on **every** agent node rather than only the ones with a `flow.*`
    /// attached, which is the half worth pinning: a conditional would make the
    /// emitter the thing to audit when an instance path comes out wrong, and the
    /// site a node execution sits at is not a property of its tool list. The two
    /// call sites differ in exactly one thing, and it is grammar 8.6 rule 10's:
    /// a dispatched instance resolves its nodes with level 1 absent, so a
    /// `map`-dispatched agent passes its dispatch path and no policy.
    #[test]
    fn every_agent_call_site_hands_the_loop_the_instance_it_would_start_under() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
agent.caller:
  model: model.m
  prompt: Answer.
  input: {{ goal: {{ type: string }} }}
  output:
    drafts:
      type: array
      max_items: 4
      items: {{ type: string }}

agent.each:
  model: model.m
  prompt: Answer.
  input: {{ goal: {{ type: string }} }}
  output: {{ draft: {{ type: string }} }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ notes: {{ type: array, max_items: 4, items: {{ type: string }} }} }}
  nodes:
    say:
      agent: agent.caller
      input: {{ goal: "input.goal" }}
    fan:
      map:
        over: "say.output.drafts"
        as: one
        node: agent.each
        max_concurrency: 2
        input: {{ goal: "one" }}
        writes: {{ draft: notes }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: fan }}
    - {{ from: fan, to: end }}
"#
        ));
        assert!(
            emitted.contains(
                "{ path: runtime.instancePath(view, \"say\"), policy: view.run.policy },"
            ),
            "an `agent:` node's own frame and the level-1 policy that reached its \
             instance (grammar 9.3, D79):\n{emitted}"
        );
        assert!(
            emitted
                .contains("runtime.callAgent(agentEach, input, [], context, { path: site.path })"),
            "a dispatched agent's site is the dispatch's, and no policy crosses \
             (grammar 8.6 rule 10):\n{emitted}"
        );
        // What the loop instantiated reaches the node's trace entry from both.
        assert_eq!(
            emitted
                .matches("toolDispatches: answer.toolDispatches,")
                .count(),
            2,
            "{emitted}"
        );
    }

    /// A `model.*` route is an ordered ladder over its members, and it is
    /// emitted **after** every direct binding it names.
    ///
    /// The order is the load-bearing half. `model.default` sorts before
    /// `model.fast` and `model.smart` in the IR's address order, so a single
    /// pass would emit a `const` that references two declared later — a module
    /// that type-checks and throws at import, past every gate a build has.
    #[test]
    fn a_route_is_an_ordered_ladder_emitted_after_its_members() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
model.fast:
  provider: provider.p
  id: another-model

model.default:
  route: [model.m, model.fast]

agent.routed:
  model: model.default
  prompt: Answer.
  output: {{ draft: {{ type: string }} }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    say: {{ agent: agent.routed, input: "input.goal" }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: end }}
"#
        ));
        let routed = emitted
            .split("const modelDefault: runtime.ModelRoute = {")
            .nth(1)
            .expect("the route is emitted");
        assert!(
            routed.contains("route: [modelM, modelFast],"),
            "the members are the route's own order: {routed}"
        );
        assert!(
            routed.contains("routeOn: [\"rate_limit\", \"overloaded\", \"timeout\"],"),
            "grammar 12.2's default is written out: {routed}"
        );
        assert!(
            emitted.find("const modelFast: runtime.ModelBinding")
                < emitted.find("const modelDefault: runtime.ModelRoute"),
            "a route references its members, so it is declared after them:\n{emitted}"
        );
        assert!(emitted.contains("  model: modelDefault,\n"), "{emitted}");
    }

    /// A composition with stores: the fixture the store tests share.
    const STORES: &str = r#"version: "0.1"

state:
  found: { type: boolean, default: false }

provider.p:
  kind: anthropic
  api_key: ${MODEL_KEY}

provider.embeds:
  kind: openai_compatible
  base_url: ${EMBED_URL}

model.m:
  provider: provider.p
  id: some-model

store.prefs:
  kind: kv
  scope: execution
  description: What this run was told.
  value_schema:
    theme: { type: string }

store.docs:
  kind: vector
  scope: global
  description: The documentation.
  embed: { model: text-embedding-3-small, provider: provider.embeds, dimensions: 4 }
  metadata_schema:
    source: { type: string }
  agent_access: read

agent.grounded:
  model: model.m
  prompt: Answer.
  stores: [store.docs, store.prefs]
  input:
    question: { type: string }
  output:
    answer: { type: string }

flow.f:
  inputs:
    question: { type: string }
  outputs: {}
  nodes:
    load:
      store: store.prefs
      op: get
      key: "'k'"
      writes: { found: found }
    ask: { agent: agent.grounded, input: { question: "input.question" } }
  edges:
    - { from: start, to: load }
    - { from: load, to: ask }
    - { from: ask, to: end }
"#;

    /// A store is one binding both consumption surfaces reach (PRD 5.8), and it
    /// carries the backend the active target resolved (grammar 11.3).
    #[test]
    fn a_store_is_one_binding_carrying_the_backend_its_target_resolved() {
        let emitted = emit(STORES);
        assert!(
            emitted.contains("const storePrefs: stores.StoreBinding = {"),
            "{emitted}"
        );
        assert!(emitted.contains("  kind: \"kv\",\n  scope: \"execution\","));
        assert!(
            emitted.contains("    provider: \"sqlite\",\n"),
            "`--target local` substitutes local storage for every store: {emitted}"
        );
        assert!(
            emitted.contains("    provider: \"sqlite_vec\",\n"),
            "…per kind: {emitted}"
        );
        // A `vector` store's `embed:` names the connection that computes the
        // vectors, which is a `provider.*` and never the backend (D116).
        assert!(
            emitted.contains("  embed: {\n    store: \"store.docs\",\n    model: \"text-embedding-3-small\",\n    provider: providerEmbeds,\n    dimensions: 4,\n  },"),
            "{emitted}"
        );
        // Absent and `{}` are different declarations (D114), and this is the
        // field the difference reaches the runtime through.
        assert!(emitted.contains("  metadata: true,\n"), "{emitted}");
        assert!(emitted.contains("  metadata: false,\n"), "{emitted}");

        // The store-op node evaluates its parameters in the input phase and
        // carries the idempotency key of grammar 9.4 to the backend.
        assert!(
            emitted.contains("    key: String(runtime.toJson(runtime.evaluate(\"'k'\", roots))),"),
            "{emitted}"
        );
        assert!(
            emitted.contains(
                "{ via: \"node\", idempotencyKey: [view.run.execution.id, ...runtime.instancePath(view, \"load\")].join(\"/\") }"
            ),
            "{emitted}"
        );
    }

    /// An attached store synthesizes grammar 11.5's tools, narrowed by
    /// `agent_access:` (Decision D37).
    #[test]
    fn an_attached_store_synthesizes_its_tools_and_agent_access_narrows_them() {
        let emitted = emit(STORES);
        let agent = emitted
            .split("const agentGrounded: runtime.AgentBinding = {")
            .nth(1)
            .expect("the agent is emitted");
        // `store.docs` is `agent_access: read`, so the write tool is withheld;
        // `store.prefs` takes the `read_write` default and gets both.
        assert!(agent.contains("name: \"docs_search\","), "{agent}");
        assert!(!agent.contains("docs_upsert"), "{agent}");
        assert!(agent.contains("name: \"prefs_get\","), "{agent}");
        assert!(agent.contains("name: \"prefs_set\","), "{agent}");
        // The arguments are parsed against the emitted Zod for the tool's own
        // surface — the same schema the JSON column constrains the model with —
        // and through `parseToolArguments`, so a refusal goes back to the model
        // like every other tool surface's (Decision D119).
        assert!(
            agent.contains("runtime.parseToolArguments(storeDocsToolSearchInput, args,"),
            "{agent}"
        );
        assert!(
            agent.contains("stores.runStoreTool(\n          storeDocs,\n          \"search\","),
            "{agent}"
        );
        // …and a store's own `description:` is what tells the model which store
        // it is choosing.
        assert!(
            agent.contains("Search by meaning in `store.docs`. The documentation."),
            "{agent}"
        );
    }

    /// Under a named target the store's backend is the deploy layer's, and a
    /// backend this release does not implement still **builds**: the refusal is
    /// the runtime's, at the op, naming where the binding came from.
    #[test]
    fn a_named_target_resolves_a_stores_backend_through_its_deploy_layer() {
        let directory = std::env::temp_dir().join(format!(
            "agent-compose-store-backend-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(directory.join("deploy")).expect("a scratch directory");
        std::fs::write(directory.join("main.yml"), STORES).expect("the entrypoint is writable");
        std::fs::write(
            directory.join("deploy/staging.yml"),
            "version: \"0.1\"\n\nstorage_backends:\n  defaults:\n    kv: { provider: redis, url: \"${REDIS_URL}\" }\n  aliases:\n    docs_db: { provider: chroma, url: \"${CHROMA_URL}\" }\n",
        )
        .expect("the deploy file is writable");

        let resolution = crate::resolve_with_target(directory.join("main.yml"), "staging");
        let _ = std::fs::remove_dir_all(&directory);
        assert!(
            resolution.diagnostics.is_empty(),
            "{:#?}",
            resolution.diagnostics
        );
        let ir = resolution.ir.expect("a clean resolution has an artifact");
        let mut names = Names::of(&ir);
        declare(&mut names, &ir);
        let emitted = module(&ir, &names).contents;

        assert!(
            emitted.contains(
                "    provider: \"redis\",\n    from: \"the `kv` default of the `staging` target\","
            ),
            "the per-kind default is what a store naming no alias resolves to: {emitted}"
        );
        // `store.docs` names no alias in this fixture, so it falls to the
        // built-in rather than to the `docs_db` alias beside it.
        assert!(
            emitted.contains("    provider: \"sqlite_vec\",\n    from: \"the built-in for `kind: vector`, which the `staging` target does not override\","),
            "{emitted}"
        );
    }

    /// A declared `route_on:` replaces the default rather than extending it.
    #[test]
    fn a_declared_route_on_is_what_the_ladder_fails_over_on() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
model.fast:
  provider: provider.p
  id: another-model

model.default:
  route: [model.m, model.fast]
  route_on: [server_error]

agent.routed:
  model: model.default
  prompt: Answer.
  output: {{ draft: {{ type: string }} }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    say: {{ agent: agent.routed, input: "input.goal" }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: end }}
"#
        ));
        assert!(
            emitted.contains("routeOn: [\"server_error\"],"),
            "{emitted}"
        );
        assert!(!emitted.contains("\"rate_limit\""), "{emitted}");
    }

    /// The output schema is offered under `<agent>_output` — except where the
    /// agent already attaches a tool of that name, which would make the pinned
    /// choice ambiguous on the wire.
    #[test]
    fn the_output_tool_never_shares_a_name_with_an_attached_tool() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
tool.collides_output:
  description: A tool that happens to be named like an output schema.
  input: {{ q: {{ type: string }} }}
  output: {{ a: {{ type: string }} }}
  exec:
    command: printf
    args: ["a"]

agent.collides:
  model: model.m
  prompt: Answer.
  tools: [tool.collides_output]
  output: {{ draft: {{ type: string }} }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    say: {{ agent: agent.collides, input: "input.goal" }}
  edges:
    - {{ from: start, to: say }}
    - {{ from: say, to: end }}
"#
        ));
        // `agent.reviewer`'s output keeps the plain spelling…
        assert!(emitted.contains("name: \"reviewer_output\","), "{emitted}");
        // …and the agent that attaches a tool of that name gets another.
        assert!(
            emitted.contains("name: \"collides_output_2\","),
            "{emitted}"
        );
    }

    /// A `start` edge carrying a guard needs a node to evaluate it at, because
    /// `start` is not one (grammar 7.2, 7.6.3 rule 2).
    #[test]
    fn guarded_start_edges_get_a_synthetic_entry_node() {
        let plain = emit(&format!(
            r#"{PREAMBLE}
flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    review: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: review }}
    - {{ from: review, to: end }}
"#
        ));
        assert!(plain.contains(".addEdge(START, \"review\")"), "{plain}");
        assert!(!plain.contains("$start"), "{plain}");

        let guarded = emit(&format!(
            r#"{PREAMBLE}
flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    review: {{ agent: agent.reviewer }}
    rework: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: rework, when: "state.draft != ''" }}
    - {{ from: start, to: review, else: true }}
    - {{ from: review, to: end }}
    - {{ from: rework, to: end }}
"#
        ));
        assert!(guarded.contains("node: \"$start\","), "{guarded}");
        assert!(guarded.contains(".addNode(\"$start\""), "{guarded}");
        assert!(guarded.contains(".addEdge(START, \"$start\")"), "{guarded}");
        assert!(
            guarded.contains("{ to: \"rework\", when: \"state.draft != ''\" },"),
            "the start edges are the synthetic node's:\n{guarded}"
        );
    }

    /// Grammar 4.3 class 2: an interpolable string reaches the runtime as its
    /// parts, and `$${` is the escape for a literal `${`.
    #[test]
    fn an_interpolable_string_is_emitted_as_its_parts() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
tool.run:
  description: Run something.
  input: {{}}
  output: {{}}
  exec:
    command: "${{TOOLBIN}}/rg"
    args: ["--config=$${{HOME}}/rc", "plain"]
"#
        ));
        assert!(
            emitted.contains(
                "command: [{ env: \"TOOLBIN\", site: \"tool.run.exec.command\" }, \"/rg\"],"
            ),
            "{emitted}"
        );
        assert!(
            emitted.contains("[\"--config=${HOME}/rc\"],"),
            "an escaped token is text, not a reference:\n{emitted}"
        );
    }

    /// A composition with no flows emits a registry with nothing in it rather
    /// than no registry at all.
    #[test]
    fn a_composition_with_no_flows_still_declares_its_surface() {
        let emitted = emit("version: \"0.1\"\n");
        assert!(
            emitted.contains("export const flows: Readonly<Record<string, CompiledFlow>> = {\n};"),
            "{emitted}"
        );
        assert!(
            emitted.contains("export async function runFlow("),
            "{emitted}"
        );
    }

    /// A channel called `shape` owns `state.shape`, and the shape table this
    /// module declares takes a name of its own rather than that one.
    ///
    /// The registry exists so a collision is impossible rather than unlikely
    /// (`codegen::names`), and two declarations sharing one *path* would defeat
    /// it from the inside: one name, two `const`s, and a project that does not
    /// compile the day `src/graph.ts` imports a state channel's schema. The
    /// suffix is the registry's ordinary disambiguation, and the composition's
    /// own surface keeps the plain spelling.
    #[test]
    fn a_channel_called_shape_does_not_share_a_name_with_the_shape_table() {
        let ir = ir_of("version: \"0.1\"\nstate:\n  shape: { type: string, default: \"\" }\n");
        let mut names = Names::of(&ir);
        declare(&mut names, &ir);
        assert_eq!(names.value("state.shape"), "stateShape");
        assert_eq!(names.value(STATE_SHAPE), "stateShape_2");

        let emitted = module(&ir, &names).contents;
        assert!(
            emitted.contains("const stateShape_2: runtime.Shape = {"),
            "{emitted}"
        );
        assert_eq!(
            emitted.matches("const stateShape:").count(),
            0,
            "the name `src/schemas.ts` exports the channel's Zod under is not \
             redeclared here:\n{emitted}"
        );

        // The common case is untouched: no channel named `shape`, plain name.
        let plain = ir_of("version: \"0.1\"\nstate:\n  draft: { type: string, default: \"\" }\n");
        let mut names = Names::of(&plain);
        declare(&mut names, &plain);
        assert_eq!(names.value(STATE_SHAPE), "stateShape");
    }

    /// Every host-registered function is listed for the README, which is where
    /// the cost of grammar 6.1's escape hatch is written down.
    #[test]
    fn the_host_functions_of_a_composition_are_listed() {
        let ir = ir_of(&format!(
            r#"{PREAMBLE}
tool.rank:
  description: Rank things.
  input: {{ text: {{ type: string }} }}
  output: {{ ranked: {{ type: string }} }}
  function:
    name: rank_candidates
"#
        ));
        assert_eq!(
            host_functions(&ir),
            [("rank_candidates".to_string(), "tool.rank".to_string())]
        );
        assert!(host_functions(&ir_of("version: \"0.1\"\n")).is_empty());
    }

    /// The superstep ceiling of one flow of this composition.
    fn ceiling(source: &str, address: &str) -> i64 {
        let ir = ir_of(source);
        let definition = ir
            .definitions
            .get(address)
            .unwrap_or_else(|| panic!("`{address}` is declared"));
        let DefinitionBody::Flow(flow) = &definition.body else {
            panic!("`{address}` is not a flow");
        };
        recursion_limit(flow)
    }

    /// A two-node flow of this composition, wrapped around the edges it is given.
    fn two_node_flow(edges: &str) -> String {
        format!(
            r#"{PREAMBLE}
flow.f:
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    write: {{ agent: agent.reviewer }}
    review: {{ agent: agent.reviewer }}
  edges:
{edges}
"#
        )
    }

    /// The ceiling is sized from **both** of grammar 7.4's clauses, because they
    /// are proofs of different strength (PRD 5.4).
    ///
    /// A counting bound is a number the composition states, so the net can be
    /// derived from it. A CEL exit condition states none — grammar 7.4 says
    /// outright that only clause 1 makes a loop provably finite — so a net
    /// derived from counting bounds alone gives a clause-2 cycle nothing, and a
    /// review loop written the way PRD 5.4 describes it stops at `25 + |nodes|`
    /// supersteps having declared no such limit anywhere.
    #[test]
    fn the_superstep_ceiling_budgets_a_cel_bounded_cycle_as_well_as_a_counted_one() {
        let acyclic = two_node_flow(
            "    - { from: start, to: write }\n    \
             - { from: write, to: review }\n    \
             - { from: review, to: end }",
        );
        assert_eq!(ceiling(&acyclic, "flow.f"), 25 + 2, "no cycle, no budget");

        let budgeted = two_node_flow(
            "    - { from: start, to: write }\n    \
             - { from: write, to: review }\n    \
             - { from: review, to: write, when: \"review.output.verdict == 'revise'\", max_iterations: 3 }\n    \
             - { from: review, to: end, else: true }",
        );
        assert_eq!(
            ceiling(&budgeted, "flow.f"),
            25 + 2 * (1 + 3),
            "a declared budget can be spent in full through every node"
        );

        // The same loop with the budget removed: still bounded (grammar 7.4
        // clause 2, the guarded back-edge beside an `else:` escape), and now
        // nothing in it says how many passes it takes.
        let unbudgeted = two_node_flow(
            "    - { from: start, to: write }\n    \
             - { from: write, to: review }\n    \
             - { from: review, to: write, when: \"review.output.verdict == 'revise'\" }\n    \
             - { from: review, to: end, else: true }",
        );
        assert_eq!(
            ceiling(&unbudgeted, "flow.f"),
            25 + 2 + 2 * CEL_BOUNDED_PASSES,
            "both of its nodes are in the unproven cycle, so both are budgeted"
        );

        // A self-edge is a one-node SCC and is bounded like any other cycle
        // (grammar 7.2), so it is budgeted like any other cycle too.
        let self_edge = format!(
            r#"{PREAMBLE}
flow.f:
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    review: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: review }}
    - {{ from: review, to: review, when: "review.output.verdict == 'revise'" }}
    - {{ from: review, to: end, else: true }}
"#
        );
        assert_eq!(ceiling(&self_edge, "flow.f"), 25 + 1 + CEL_BOUNDED_PASSES);
    }

    // -----------------------------------------------------------------------
    // Fan-out and subgraph emission (grammar 8.5, 8.6)
    // -----------------------------------------------------------------------

    /// The composition every fan-out test below is written against: a producer,
    /// a homogeneous map over its result, and a routed map over a union.
    const FANNED: &str = r#"
agent.planner:
  model: model.m
  prompt: Plan it.
  input:
    goal: { type: string }
  output:
    tasks:
      type: array
      max_items: 5
      items:
        type: object
        properties:
          title: { type: string }
          weight: { type: number }
    findings:
      type: array
      max_items: 5
      items:
        discriminator: kind
        variants:
          fixable:
            file: { type: string }
            note: { type: string }
          human:
            summary: { type: string }
            note: { type: string }
          stale:
            note: { type: string }

agent.worker:
  model: model.m
  prompt: Work it.
  input:
    title: { type: string }
  output:
    result: { type: string }

agent.plain:
  model: model.m
  prompt: Say it.
  output:
    line: { type: string }

flow.f:
  inputs: { goal: { type: string } }
  outputs: { draft: { type: string } }
  nodes:
    plan:
      agent: agent.planner
      input: { goal: "input.goal" }
    quiet:
      agent: agent.reviewer
      input: { goal: "input.goal", draft: "state.draft" }
    work:
      map:
        over: plan.output.tasks
        as: task
        node: agent.worker
        max_concurrency: 3
        input: { title: "task.title" }
        writes: { result: notes }
    route:
      map:
        over: plan.output.findings
        as: finding
        route_by: kind
        max_concurrency: 4
        on_item_error: skip
        routes:
          fixable:
            node: agent.worker
            max_concurrency: 2
            input: { title: "finding.file" }
            writes: { result: notes }
        default:
          node: agent.plain
          detach: true
          input: "finding.note"
  edges:
    - { from: start, to: plan }
    - { from: plan, to: quiet }
    - { from: quiet, to: work }
    - { from: work, to: route }
    - { from: route, to: end }
"#;

    /// The chunk of the emitted module that declares one name.
    fn declaration<'a>(emitted: &'a str, name: &str) -> &'a str {
        emitted
            .split(&format!("const {name}"))
            .nth(1)
            .unwrap_or_else(|| panic!("`{name}` is not declared:\n{emitted}"))
            .split("\n};\n")
            .next()
            .expect("the declaration closes")
    }

    /// Grammar 8.6's homogeneous form: one route, the per-item binding over the
    /// `as:` name, and the effective write map of rule 5.
    #[test]
    fn a_homogeneous_map_emits_one_route_with_its_binding_and_its_writes() {
        let emitted = emit(&format!("{PREAMBLE}{FANNED}"));
        let map = declaration(&emitted, "flowFNodeWorkMap");
        assert!(map.contains("as: \"task\","), "{map}");
        assert!(map.contains("path: \"plan.output.tasks\","), "{map}");
        assert!(
            map.contains("producer: \"plan\","),
            "the node whose result `over` reads is named, so the map can find it \
             in a later step (grammar 8.6 rule 11):\n{map}"
        );
        assert!(map.contains("maxConcurrency: 3,"), "{map}");
        assert!(
            map.contains("onItemError: \"fail\","),
            "the default:\n{map}"
        );
        assert!(
            !map.contains("routeBy:"),
            "the homogeneous form routes on nothing:\n{map}"
        );
        assert!(
            map.contains(
                "input: (roots) => ({\n        \"title\": runtime.toJson(runtime.evaluate(\"task.title\", roots)),\n      }),"
            ),
            "{map}"
        );
        assert!(
            map.contains("{ field: \"result\", channel: \"notes\", reduce: \"append\" },"),
            "one element per write, into the reduced channel the remap names:\n{map}"
        );
        // The item is bound through its own declared type, so `weight` is the
        // `double` the schema says rather than whatever its value looks like.
        assert!(map.contains("\"weight\": \"double\""), "{map}");
    }

    /// Each route sees **its variant's payload only**, and the catch-all sees
    /// the discriminator over the unrouted tags plus what every one of them
    /// declares identically (grammar 8.6 rule 4, Decision D30).
    #[test]
    fn a_routed_map_narrows_every_route_to_the_variants_that_reach_it() {
        let emitted = emit(&format!("{PREAMBLE}{FANNED}"));
        let map = declaration(&emitted, "flowFNodeRouteMap");
        assert!(map.contains("routeBy: \"kind\","), "{map}");

        let named = map
            .split("tag: \"fixable\",")
            .nth(1)
            .expect("the named route is emitted")
            .split("},\n")
            .next()
            .expect("it closes");
        assert!(named.contains("\"file\": \"string\""), "{named}");
        assert!(
            !named.contains("\"summary\""),
            "a named route is not handed another variant's payload:\n{named}"
        );
        assert!(
            named.contains("maxConcurrency: 2,"),
            "a route may tighten the map's bound (Decision D28):\n{named}"
        );

        let catch_all = map
            .split("fallback: {")
            .nth(1)
            .expect("the catch-all is emitted");
        assert!(
            catch_all.contains("\"kind\": \"string\"")
                && catch_all.contains("\"note\": \"string\""),
            "the catch-all sees the discriminator and the field every unrouted \
             variant declares identically:\n{catch_all}"
        );
        assert!(
            !catch_all.contains("\"file\"") && !catch_all.contains("\"summary\""),
            "…and nothing only one of them declares:\n{catch_all}"
        );
        assert!(catch_all.contains("detach: true,"), "{catch_all}");
        assert!(
            catch_all.contains("writes: [],"),
            "a detached dispatch writes no reduced state, name-based writes \
             included (grammar 8.6 rule 7):\n{catch_all}"
        );
        // The scalar `input:` form, which is how an object item feeds a
        // string-in agent (grammar 8.6 rule 12, Decision D75).
        assert!(
            catch_all.contains(
                "input: (roots) => runtime.toJson(runtime.evaluate(\"finding.note\", roots)),"
            ),
            "{catch_all}"
        );
        assert!(map.contains("onItemError: \"skip\","), "{map}");
    }

    /// Only the node a `map.over` reads keeps its result in `$run`
    /// (grammar 8.6 rule 11): a node result is arbitrarily large and that
    /// channel is carried through every superstep.
    #[test]
    fn only_the_node_a_map_reads_keeps_its_result() {
        let emitted = emit(&format!("{PREAMBLE}{FANNED}"));
        assert!(
            declaration(&emitted, "flowFNodePlan:").contains("retains: true,"),
            "the producer both maps read:\n{emitted}"
        );
        assert!(
            !declaration(&emitted, "flowFNodeQuiet:").contains("retains:"),
            "…and a node nothing reads keeps nothing:\n{emitted}"
        );
    }

    /// A `flow:` node binds its subflow's parameters totally, carries the
    /// instance path grammar 9.4 keys effects from, and hands down its `policy:`
    /// as grammar 9.3's level 1.
    #[test]
    fn a_flow_node_binds_its_subflow_totally_and_hands_down_its_policy() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
flow.inner:
  inputs:
    goal: {{ type: string }}
    tone: {{ type: string, default: calm }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    only: {{ agent: agent.reviewer, input: {{ goal: "input.goal", draft: "state.draft" }} }}
  edges:
    - {{ from: start, to: only }}
    - {{ from: only, to: end }}

flow.f:
  inputs: {{ goal: {{ type: string }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    sub:
      flow: flow.inner
      input: {{ goal: "input.goal" }}
      context: inherit
      policy: {{ timeout: 30s, on_error: skip }}
    ask:
      human:
        input: {{ question: {{ type: string }} }}
        output: {{ decision: {{ enum: [approve, reject] }} }}
  edges:
    - {{ from: start, to: sub }}
    - {{ from: sub, to: ask }}
    - {{ from: ask, to: end }}
"#
        ));
        let node = declaration(&emitted, "flowFNodeSub:");
        assert!(
            node.contains("\"goal\": runtime.toJson(runtime.evaluate(\"input.goal\", roots)),"),
            "{node}"
        );
        assert!(
            node.contains("\"tone\": \"calm\","),
            "a field the site left out carries its own `default:`, and nothing \
             falls through by name (Decision D68):\n{node}"
        );
        assert!(
            node.contains("path: runtime.instancePath(view, \"sub\"),"),
            "{node}"
        );
        assert!(
            node.contains("timeoutMs: 30000,") && node.contains("onError: \"skip\","),
            "the instantiation site's `policy:` is level 1 for the nodes inside:\n{node}"
        );
        assert!(
            node.contains("policy: runtime.instancePolicy(view.run.policy,"),
            "…laid under whatever already reached this instance, so the \
             outermost site wins (Decision D79):\n{node}"
        );
        assert!(
            node.contains("history: (view.state[\"messages\"] ?? []) as readonly unknown[],"),
            "`context: inherit` hands the caller's history to the instance:\n{node}"
        );

        // Decision D102: a `human` node takes neither `timeout` nor `retry` from
        // any level, level 1 included.
        assert!(
            declaration(&emitted, "flowFNodeAsk:").contains("exempt: true,"),
            "{emitted}"
        );

        // The module surface a `flow:` node reaches its target through.
        let binding = declaration(&emitted, "flowInnerBinding: runtime.SubflowBinding");
        assert!(binding.contains("address: \"flow.inner\","), "{binding}");
        assert!(binding.contains("outputs: [\"draft\"],"), "{binding}");
    }

    /// A flow that declares no `inputs:` is instantiable from **both** module
    /// boundaries, and from either it is started with an empty object.
    ///
    /// `inputs:` is optional (grammar 7.5) and an inputs-less flow is ordinary —
    /// `flow.tenant_recall` in the store acceptance fixture is one. Nothing
    /// writes the surface down, so `schema::surfaces` emits no `<flow>.inputs`
    /// for it; asking for that surface at a use site is what used to panic the
    /// emitter (`no schema surface is emitted for ...`) on a composition
    /// `validate` accepts clean, which is why the assertion below opens by
    /// checking that it does.
    #[test]
    fn a_flow_node_instantiates_a_subflow_that_declares_no_inputs() {
        let source = format!(
            r#"{PREAMBLE}
flow.inner:
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    only: {{ agent: agent.reviewer, input: {{ goal: "'fixed'", draft: "state.draft" }} }}
  edges:
    - {{ from: start, to: only }}
    - {{ from: only, to: end }}

flow.f:
  inputs: {{ goals: {{ type: array, max_items: 4, items: {{ type: string }} }} }}
  outputs: {{ draft: {{ type: string }} }}
  nodes:
    sub: {{ flow: flow.inner }}
    each:
      map:
        over: input.goals
        node: flow.inner
        max_concurrency: 2
        input: {{}}
        writes: {{ draft: notes }}
  edges:
    - {{ from: start, to: sub }}
    - {{ from: sub, to: each }}
    - {{ from: each, to: end }}
"#
        );
        let ir = ir_of(&source);
        assert!(
            crate::check(&ir).is_empty(),
            "the composition validates clean, so `build` owes it a project: {:#?}",
            crate::check(&ir)
        );

        let mut names = Names::of(&ir);
        declare(&mut names, &ir);
        let emitted = module(&ir, &names).contents;

        assert!(
            declaration(&emitted, "flowFNodeSub:").contains("input: (roots) => ({\n  }),"),
            "a `flow:` node onto an inputs-less subflow binds the empty \
             object:\n{emitted}"
        );
        assert!(
            declaration(&emitted, "flowFNodeEachMap:").contains("input: (roots) => ({\n      }),"),
            "…and so does a `map` dispatch onto the same flow, which is the \
             position that already answered:\n{emitted}"
        );
    }
}
