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
//! # What is emitted for a construct this release does not execute
//!
//! `map`, `flow:`, `human` and `store` nodes are parsed, validated, and
//! **emitted as real nodes with their real topology** — their edges, their
//! budgets, their place in the graph — whose activity throws
//! `Unimplemented` naming the construct and the milestone bullet that lands it.
//! The alternative was refusing to emit a graph for those compositions at all,
//! which would leave `build` failing on two of the four committed goldens and
//! nothing type-checking the topology around the construct. A node that says
//! what it does not do is not the same as a node that pretends: nothing here
//! answers a plausible value.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::ast::common::{ControlTarget, EdgeSource, EdgeTarget, Interpolated};
use crate::ast::definition::ProviderKind;
use crate::ir::Ir;
use crate::ir::binding::{Exec, Http, NodeInput};
use crate::ir::definition::{Agent, DefinitionBody, Model, Tool};
use crate::ir::flow::{Edge, Flow, Node, NodeKind, ToolImplementation};
use crate::ir::schema::FieldMap;

use super::names::{self, Names};
use super::policy::{self, Strategy};
use super::{cel, schema};

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
                if needs_start_router(flow) {
                    names.declare(&format!("{address}.start"));
                }
                for node in &flow.nodes {
                    names.declare(&format!("{address}.node.{}", node.id.value.as_str()));
                }
            }
            DefinitionBody::Store(_) => {}
        }
    }
    names.declare("state.shape");
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
    body.push_str(&tools(ir, names, &surfaces, &mut imported));
    body.push_str(&agents(ir, names, &surfaces));

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

    contents.push_str(
        "\nimport { END, START, StateGraph, isInterrupted } from \"@langchain/langgraph\";\n",
    );
    contents.push_str("\nimport * as runtime from \"./runtime.ts\";\n");
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
        names.value("state.shape")
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
        // binding is still emitted, and calling it says which kind it is.
        ProviderKind::Bedrock => "bedrock",
        ProviderKind::Vertex => "vertex",
    }
}

fn models(ir: &Ir, names: &Names) -> String {
    let mut text = String::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Model(model) = &definition.body else {
            continue;
        };
        let (bound, note) = match model {
            Model::Direct(direct) => (direct, String::new()),
            Model::Route(route) => {
                // Failover is a later M1 bullet ("model routing with
                // trace-recorded failover"). A route binds its **first** member
                // — the one a live call reaches first either way — so a
                // composition that declares one runs rather than refusing to
                // build, and the note says what is missing rather than leaving
                // a reader to infer it from behaviour.
                let first = route.route.first().expect("a route has members");
                let Some(member) = ir.definitions.get(&first.value.to_string()) else {
                    continue;
                };
                let DefinitionBody::Model(Model::Direct(direct)) = &member.body else {
                    continue;
                };
                (
                    direct,
                    format!(
                        " This is `{}`, the first member of the route `{address}` declares: \
                         failover is not executed by this compiler release (PRD §7 M1: \
                         model routing with trace-recorded failover), so a condition in \
                         `route_on:` fails the node rather than moving to the next member.",
                        first.value
                    ),
                )
            }
        };
        text.push('\n');
        text.push_str(&names::doc(
            "",
            &[format!(
                "`{address}` — `{}` on `{}` (grammar 12.2).{note}",
                bound.id.value, bound.provider.value
            )],
        ));
        text.push_str(&format!(
            "const {}: runtime.ModelBinding = {{\n",
            names.value(address)
        ));
        text.push_str(&format!("  address: {},\n", names::string(address)));
        text.push_str(&format!("  id: {},\n", names::string(&bound.id.value)));
        text.push_str(&format!(
            "  provider: {},\n",
            names.value(&bound.provider.value.to_string())
        ));
        if bound.settings.is_empty() {
            text.push_str("  settings: {},\n");
        } else {
            text.push_str("  settings: {\n");
            for (key, value) in &bound.settings {
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
    text
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
                 checked signature grammar 8.4 asks for and the model's arguments are \
                 held to.",
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
        text.push_str(&format!("  const input = {input_schema}.parse(args);\n"));
        match &tool.implementation {
            ToolImplementation::Exec { exec } => {
                text.push_str(&format!(
                    "  return {output_schema}.parse(\n    await runtime.runExec({}, input, context),\n  );\n",
                    exec_binding(exec, &tool.output, address, "    ", true)
                ));
            }
            ToolImplementation::Http { http } => {
                let roots = format!(
                    "{{ input: runtime.bind(input, {}) }}",
                    cel::shape_of_field_map(input_fields.as_ref(), "  ")
                );
                text.push_str(&format!("  const roots = {roots};\n"));
                text.push_str(&format!(
                    "  return {output_schema}.parse(\n    await runtime.runHttp({}, {}, context),\n  );\n",
                    http_binding(http, &tool.output, address, "    ", true),
                    http_request(http, None, "    ")
                ));
            }
            ToolImplementation::Function { function } => {
                text.push_str(&format!(
                    "  return {output_schema}.parse(\n    await runtime.callFunction({}, input, context),\n  );\n",
                    names::string(function.name.value.as_str())
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

fn agents(ir: &Ir, names: &Names, surfaces: &[schema::Surface<'_>]) -> String {
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
        if agent.tools.is_empty() {
            text.push_str("  tools: [],\n");
        } else {
            text.push_str("  tools: [\n");
            for reference in &agent.tools {
                let tool_address = reference.value.to_string();
                let Some(tool) = ir.definitions.get(&tool_address) else {
                    continue;
                };
                let DefinitionBody::Tool(tool) = &tool.body else {
                    // A `flow.*` in a tool list is flow-as-tool, which needs
                    // subgraph instantiation — a later M1 bullet.
                    text.push_str(&format!(
                        "    // `{tool_address}` is a flow attached as a tool, which this \
                         compiler release does not run.\n"
                    ));
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
                text.push_str(&format!("      invoke: {},\n", names.value(&tool_address)));
                text.push_str("    },\n");
            }
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

/// The name the agent's output schema is offered to the model under.
///
/// `<local name>_output`, which is what makes a transcript readable — except
/// where the agent already attaches a tool of that name, in which case the two
/// would be one tool on the wire and the pinned choice would be ambiguous.
fn output_tool_name(ir: &Ir, agent: &Agent, local: &str) -> String {
    let mut name = format!("{local}_output");
    let attached: Vec<String> = agent
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
    let state_shape = names.value("state.shape");
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
        text.push_str(&format!(
            "  shapes: {{ input: {input_shape}, state: {state_shape}, output: {output_shape} }},\n"
        ));
        text.push_str(&input_builder(ir, address, node, surfaces));
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
    text
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
        text.push_str("    retry: {\n");
        text.push_str(&format!("      max: {},\n", retry.max));
        text.push_str(&format!(
            "      backoffMs: {},\n",
            policy::milliseconds(&retry.backoff.value)
        ));
        text.push_str(&format!(
            "      multiplier: {},\n",
            names::number(&crate::ast::schema::Number::Float(
                retry.multiplier.unwrap_or(2.0)
            ))
        ));
        if let Some(max_backoff) = &retry.max_backoff {
            text.push_str(&format!(
                "      maxBackoffMs: {},\n",
                policy::milliseconds(&max_backoff.value)
            ));
        }
        text.push_str(&format!(
            "      jitter: {},\n",
            retry.jitter.unwrap_or(true)
        ));
        text.push_str("    },\n");
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

// ---------------------------------------------------------------------------
// Inputs (grammar 8.0)
// ---------------------------------------------------------------------------

fn input_builder(ir: &Ir, address: &str, node: &Node, surfaces: &[schema::Surface<'_>]) -> String {
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
            text.push_str(&http_request_fields(http, node.input.as_ref(), "    "));
            text.push_str("  }),\n");
            text
        }
        NodeKind::Flow { .. }
        | NodeKind::Map { .. }
        | NodeKind::Human { .. }
        | NodeKind::Store { .. } => "  input: () => null,\n".to_string(),
    }
    .to_string()
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
    let output_schema = output_path(ir, address, node).map(|path| {
        let name = names.value(&path).to_string();
        imported.push(name.clone());
        name
    });

    match &node.kind {
        NodeKind::Agent { agent } => {
            let binding = names.value(&agent.value.to_string());
            let schema = output_schema.expect("an agent node has an output surface");
            format!(
                "  run: async (input, context, view) => {{\n    \
                 const answer = await runtime.callAgent(\n      {binding},\n      input,\n      \
                 runtime.historyTurns(view.state[\"messages\"] as unknown[]),\n      context,\n    );\n    \
                 return {{ output: {schema}.parse(answer.output), history: answer.history }};\n  }},\n"
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
                "  run: async (input, context) => ({{\n    output: {schema}.parse(\n      \
                 await runtime.runExec({}, input, context),\n    ),\n  }}),\n",
                exec_binding(
                    exec,
                    output.as_ref(),
                    &format!("{address}.node.{id}"),
                    "      ",
                    false
                )
            )
        }
        NodeKind::Http { http } => {
            let output = surface_fields(surfaces, &format!("{address}.node.{id}.output"));
            let schema = output_schema.expect("an http node has an output surface");
            format!(
                "  run: async (input, context) => ({{\n    output: {schema}.parse(\n      \
                 await runtime.runHttp(\n{},\n        input as {{ query?: Record<string, unknown>; body?: unknown }},\n        context,\n      ),\n    ),\n  }}),\n",
                indent_block(
                    &http_binding(
                        http,
                        output.as_ref(),
                        &format!("{address}.node.{id}"),
                        "        ",
                        false
                    ),
                    "        "
                )
            )
        }
        NodeKind::Flow { flow, .. } => {
            unimplemented_run(&format!("instantiating `{}`", flow.value), "subgraphs")
        }
        NodeKind::Map { .. } => unimplemented_run(
            "a `map` dispatch",
            "homogeneous + discriminator-routed `map`→`Send` with index-tagged reducers",
        ),
        NodeKind::Human { .. } => unimplemented_run(
            "a `human` pause",
            "`agent-compose serve` (generated Fastify app for http triggers: start/resume/status)",
        ),
        NodeKind::Store { store, op, .. } => unimplemented_run(
            &format!("a `{}` on `{}`", op.as_str(), store.value),
            "store-op nodes + synthesized store tools with SQLite/local-disk backends",
        ),
    }
}

fn unimplemented_run(what: &str, bullet: &str) -> String {
    format!(
        "  run: () => {{\n    throw new runtime.Unimplemented({}, {});\n  }},\n",
        names::string(what),
        names::string(bullet)
    )
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
    // that property is string-typed takes the trimmed raw stream whole. The
    // count is over the whole property set (Decision D109), and the exception is
    // a `tool.*`-surface rule (Decision D91).
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

/// A tool binding's request: its `query:`/`body:` values over the tool's own
/// `input` (grammar 6.1, Decision D65).
fn http_request(http: &Http, node_input: Option<&NodeInput>, indent: &str) -> String {
    let mut text = String::from("{\n");
    text.push_str(&http_request_fields(
        http,
        node_input,
        &format!("{indent}  "),
    ));
    text.push_str(&format!("{indent}}}"));
    text
}

/// The `query` and `body` an `http:` surface sends (grammar 6.1, 8.3).
fn http_request_fields(http: &Http, node_input: Option<&NodeInput>, indent: &str) -> String {
    let mut text = String::new();
    let bound = match node_input {
        Some(NodeInput::Fields { bindings }) => Some(bindings),
        _ => None,
    };
    let body_bearing = !matches!(
        http.method.value,
        crate::ast::binding::HttpMethod::Get | crate::ast::binding::HttpMethod::Head
    );

    if let Some(query) = &http.query {
        text.push_str(&format!("{indent}query: {{\n"));
        for binding in &query.entries {
            text.push_str(&format!(
                "{indent}  {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(&binding.name.value),
                names::string(binding.value.value.as_str())
            ));
        }
        text.push_str(&format!("{indent}}},\n"));
    } else if !body_bearing && let Some(bindings) = bound {
        text.push_str(&format!("{indent}query: {{\n"));
        for binding in &bindings.entries {
            text.push_str(&format!(
                "{indent}  {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(&binding.name.value),
                names::string(binding.value.value.as_str())
            ));
        }
        text.push_str(&format!("{indent}}},\n"));
    }

    if let Some(body) = &http.body {
        text.push_str(&format!("{indent}body: {{\n"));
        for binding in &body.entries {
            text.push_str(&format!(
                "{indent}  {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(&binding.name.value),
                names::string(binding.value.value.as_str())
            ));
        }
        text.push_str(&format!("{indent}}},\n"));
    } else if body_bearing && let Some(bindings) = bound {
        text.push_str(&format!("{indent}body: {{\n"));
        for binding in &bindings.entries {
            text.push_str(&format!(
                "{indent}  {}: runtime.toJson(runtime.evaluate({}, roots)),\n",
                names::string(&binding.name.value),
                names::string(binding.value.value.as_str())
            ));
        }
        text.push_str(&format!("{indent}}},\n"));
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
    let remap: BTreeMap<&str, &str> = node
        .writes
        .as_ref()
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
            "    {{ field: {}, channel: {}, reduce: {} }},\n",
            names::string(name),
            names::string(channel),
            names::string(reduce)
        ));
    }
    if text.is_empty() {
        "  writes: [],\n".to_string()
    } else {
        format!("  writes: [\n{text}  ],\n")
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
        text.push_str(&format!(
            "    outputs: [{}],\n",
            flow.outputs
                .fields
                .iter()
                .map(|field| names::string(field.name.value.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        text.push_str(&format!("    recursionLimit: {},\n", recursion_limit(flow)));
        text.push_str(&match &flow.inputs {
            Some(_) => {
                let schema = names.value(&format!("{address}.inputs")).to_string();
                imported.push(schema.clone());
                format!(
                    "    parse: (inputs: unknown) => {schema}.parse(inputs) as Record<string, unknown>,\n"
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

/// How many supersteps a flow instance may take before LangGraph refuses.
///
/// A safety net rather than a semantic bound: the bounds that decide a run are
/// the composition's own (`max_iterations`, grammar 7.4). This is sized so that
/// every counting bound can be spent in full — one pass per budget through every
/// node — with room for the acyclic part on either side, and `runFlow` takes an
/// override for a CEL-bounded loop (grammar 7.4 clause 2) that legitimately runs
/// longer.
fn recursion_limit(flow: &Flow) -> i64 {
    let budgets: i64 = flow
        .edges
        .iter()
        .filter_map(|edge| edge.max_iterations)
        .sum();
    25 + (flow.nodes.len() as i64) * (1 + budgets)
}

const REGISTRY_DOC: &str = r#"
/** One compiled flow: what it takes, what it answers, and how to run it. */
export interface CompiledFlow {
  /** Its typed address (grammar 2.2). */
  readonly address: string;
  /** The fields its `inputs:` declares (grammar 7.5). */
  readonly inputs: readonly string[];
  /** The fields its `outputs:` declares, each read from the channel of that name. */
  readonly outputs: readonly string[];
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
 * Run one flow to quiescence and materialize its outputs.
 *
 * This is the invocation surface `agent-compose run` and the generated `serve`
 * app are built on (PRD 5.11's `start`), and what an ejected project calls
 * directly. The inputs are parsed against the flow's own `inputs:` schema before
 * anything runs, which is where an invocation that the flow cannot accept is
 * refused by field name (grammar 13.2).
 *
 * A run that does not reach quiescence raises `runtime.FlowFailure`, which
 * carries the trace it did make and the original error as its `cause` — see
 * `CompiledFlow.stream` for why the run is streamed to keep it.
 */
export async function runFlow(
  address: string,
  inputs: unknown = {},
  options: {
    readonly executionId?: string;
    readonly sessionKey?: string;
    readonly recursionLimit?: number;
  } = {},
): Promise<FlowRun> {
  const flow = flows[address];
  if (flow === undefined) {
    throw new Error(
      `\`${address}\` is not a flow of this composition: ${Object.keys(flows).join(", ")}`,
    );
  }
  const parsed = flow.parse(inputs);
  let state: GraphState | undefined;
  try {
    const supersteps = await flow.stream(
      {
        $run: {
          ...runtime.emptyRun(),
          input: parsed,
          execution: {
            id: options.executionId ?? `exec_${globalThis.crypto.randomUUID()}`,
            session_key: options.sessionKey ?? "",
          },
        },
      },
      { recursionLimit: options.recursionLimit ?? flow.recursionLimit },
    );
    for await (const superstep of supersteps) {
      // What LangGraph's own `invoke` keeps: the last chunk that is a state.
      // An interrupt is announced as a chunk of its own rather than as one, and
      // reading it as state would lose the run's — `human:` nodes are the
      // construct that raises one, and resuming them is a later bullet (PRD §7).
      if (!isInterrupted(superstep)) state = superstep;
    }
  } catch (error) {
    throw new runtime.FlowFailure(address, runtime.failedTrace(state, error), error);
  }
  if (state === undefined) {
    // Unreachable: `streamMode: "values"` emits the state the run started from
    // before any node has run. A run with no state at all is still not one this
    // function can answer for, and saying so beats reading `undefined` as empty.
    throw new Error(`\`${address}\` produced no state`);
  }

  const outputs: Record<string, unknown> = {};
  for (const field of flow.outputs) {
    outputs[field] = runtime.channelValue(
      state as unknown as Record<string, unknown>,
      field,
      `\`${address}\`'s output field \`${field}\``,
    );
  }
  return { outputs, trace: state.$run.trace, state };
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

    /// A construct this release does not execute is emitted as a real node with
    /// its real topology, whose activity says what it is and which milestone
    /// bullet lands it — rather than as a plausible answer.
    #[test]
    fn an_unimplemented_kind_throws_and_names_the_bullet_it_waits_on() {
        let emitted = emit(&format!(
            r#"{PREAMBLE}
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
    rescue: {{ agent: agent.reviewer }}
  edges:
    - {{ from: start, to: ask }}
    - {{ from: ask, to: end }}
    - {{ from: rescue, to: end }}
"#
        ));
        assert!(
            emitted.contains("throw new runtime.Unimplemented(\"a `human` pause\","),
            "{emitted}"
        );
        // Grammar 7.8 clause 3: a node reached only by `on_timeout` is live
        // code, and the graph has to be told so or it will not compile.
        assert!(
            emitted.contains("ends: [END, \"rescue\"],"),
            "the control-transfer target is an end of the node that transfers to it:\n{emitted}"
        );
    }

    /// A `model.*` route binds its first member, and the note says failover is
    /// not what this release does with the rest.
    #[test]
    fn a_route_binds_its_first_member_and_says_what_is_missing() {
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
            .split("const modelDefault: runtime.ModelBinding")
            .nth(1)
            .expect("the route is emitted");
        assert!(routed.contains("id: \"some-model\","), "{routed}");
        assert!(
            emitted.contains("failover is not executed by this compiler release"),
            "{emitted}"
        );
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
}
