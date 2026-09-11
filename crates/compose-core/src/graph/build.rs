//! The resolved IR, read as a picture.
//!
//! One pass per flow, in the order [`Ir::definitions`](crate::ir::Ir) holds
//! them, which is sorted by address — so the document's flow order is a
//! property of the composition rather than of the order anything was visited
//! in (PRD 5.12).
//!
//! # What becomes a node, and what becomes an edge
//!
//! Every node of a flow becomes one [`GraphNode`], and so do the two
//! pseudo-nodes `start` and `end`, which a flow's edges name (grammar 7.2).
//! A `map` additionally contributes one **satellite** per route: the dispatch
//! target is where an item's work happens, and a canvas that drew the map alone
//! would show a fan-out with nothing to fan out to. A satellite's id is
//! `<map id>/<variant>` for a named route, and `<map id>/(default)` or
//! `<map id>/(item)` for the two routes that have no variant tag. Neither `/`
//! nor a parenthesis is legal in an identifier (grammar 2.1), so a satellite
//! collides neither with a node the author wrote nor — and this is what the
//! parentheses are for — with the satellite of a variant tagged `default`,
//! which is a legal tag and a sibling key of `routes:` rather than a member
//! of it (grammar 8.6).
//!
//! Edges are every transfer an execution could take, which is more than the
//! `edges:` list:
//!
//! | class | where it comes from |
//! |---|---|
//! | `unconditional`, `conditional`, `default` | the flow's own `edges:` (grammar 7.2, 7.3) |
//! | `map_route` | a `map`'s dispatch to one of its routes (grammar 8.6) |
//! | `error_fallback` | `on_error: { fallback: … }` (grammar 9.2) |
//! | `timeout` | `human.on_timeout:` (grammar 8.7) |
//!
//! The last two are the control-transfer positions grammar 7.8 counts
//! *alongside* edges when it decides reachability, which is exactly why they
//! are edges here: a node reachable only through a fallback is legal, and a
//! picture that drew it floating would be showing an unreachable node the
//! validator had accepted.
//!
//! **A map's join barrier is its outgoing edges, marked.** PRD 5.6 states it —
//! "the downstream edge is the barrier: it fires when all instances complete" —
//! so the barrier is a property carried on the edges the map already has,
//! rather than a synthetic edge from each satellite back to the map. The
//! alternative would put a cycle in a layout that has no cycles in it.

use std::collections::BTreeMap;

use crate::ast::common::{Address, ControlTarget, EdgeSource, EdgeTarget, EnvRef, PathStep};
use crate::ast::definition::{AgentAccess, StoreKind};
use crate::check::model;
use crate::codegen::policy::{self, Level, Strategy};
use crate::ir::binding::{Bindings, Exec, Http, InterpolatedEntry, NodeInput, Writes};
use crate::ir::definition::{Agent, DefinitionBody, Model, Provider, Store, Tool};
use crate::ir::flow::{
    Flow, Human, ItemError, Map, MapDispatch, MapRoute, Node, NodeKind as IrNodeKind, StoreParams,
    StoreValue, ToolImplementation,
};
use crate::ir::policy::Retry;
use crate::ir::schema::{FieldMap, TypeForm, TypeNode};
use crate::ir::trigger::TriggerKind;
use crate::ir::{Ir, Section};

use super::document::{
    AgentView, BindingView, DispatchView, EdgeClass, ExecView, FlowGraph, GRAPH_VERSION,
    GraphDocument, GraphEdge, GraphNode, HttpView, HumanView, InputView, InstancePolicyView,
    ItemErrorView, MapView, ModelView, NodeKind, OnErrorView, PolicyLevel, PolicyView,
    ProviderView, RetryView, RouteView, SchemaSource, SchemaView, SchemasView, SettingView,
    StoreView, SubflowView, TimeoutView, ToolSource, ToolView, TriggerView, WriteView,
};
use super::summary;

/// Every `flow.*` address the composition declares, sorted.
pub(super) fn flow_addresses(ir: &Ir) -> Vec<String> {
    ir.definitions
        .iter()
        .filter(|(_, definition)| matches!(definition.body, DefinitionBody::Flow(_)))
        .map(|(address, _)| address.clone())
        .collect()
}

/// The document for a whole composition, or for the one flow `--flow` named.
pub(super) fn document(ir: &Ir, only: Option<&str>) -> GraphDocument {
    let flows = ir
        .definitions
        .iter()
        .filter_map(|(address, definition)| match &definition.body {
            DefinitionBody::Flow(flow) if only.is_none_or(|named| named == address.as_str()) => {
                Some(graph(ir, address, flow))
            }
            _ => None,
        })
        .collect();
    GraphDocument {
        graph_version: GRAPH_VERSION,
        entrypoint: ir.entrypoint.clone(),
        target: ir.target.clone(),
        spec_version: ir.spec_version.clone(),
        flows,
    }
}

/// One flow, as one canvas.
fn graph(ir: &Ir, address: &str, flow: &Flow) -> FlowGraph {
    let mut nodes = vec![
        pseudo("start", NodeKind::Start),
        pseudo("end", NodeKind::End),
    ];
    for node in &flow.nodes {
        nodes.push(graph_node(ir, flow, node));
        if let IrNodeKind::Map { map } = &node.kind {
            let id = node.id.value.as_str();
            for route in routes(ir, flow, id, map) {
                nodes.push(satellite(ir, id, &route));
            }
        }
    }

    FlowGraph {
        address: address.to_string(),
        description: flow.description.as_ref().map(|text| text.value.clone()),
        inputs: flow
            .inputs
            .as_ref()
            .map(summary::fields)
            .unwrap_or_default(),
        outputs: summary::fields(&flow.outputs),
        triggers: triggers(ir, address),
        nodes,
        edges: edges(ir, flow),
    }
}

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

fn pseudo(id: &str, kind: NodeKind) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        kind,
        binding: None,
        description: None,
        input: None,
        writes: Vec::new(),
        policy: None,
        schemas: None,
        agent: None,
        exec: None,
        http: None,
        tool: None,
        store: None,
        subflow: None,
        human: None,
        map: None,
        dispatch: None,
    }
}

fn graph_node(ir: &Ir, flow: &Flow, node: &Node) -> GraphNode {
    let mut held = pseudo(node.id.value.as_str(), kind_of(&node.kind));
    held.binding = binding_of(node);
    held.description = node
        .description
        .as_ref()
        .map(|text| text.value.clone())
        .or_else(|| definition_description(ir, node));
    held.input = node.input.as_ref().map(input_view);
    held.writes = node
        .writes
        .as_ref()
        .map(|writes| write_views(ir, writes))
        .unwrap_or_default();
    held.policy = Some(policy_view(ir, node));
    held.schemas = schemas(
        input_schema(ir, node),
        output_schema(ir, node).map(|(source, fields)| SchemaView {
            source,
            fields: summary::fields(&fields),
        }),
    );

    match &node.kind {
        IrNodeKind::Agent { agent } => {
            held.agent = agent_view(ir, &agent.value);
        }
        IrNodeKind::Exec { exec } => held.exec = Some(exec_view(exec)),
        IrNodeKind::Http { http } => held.http = Some(http_view(http)),
        IrNodeKind::Function { function } => {
            let address = function.value.to_string();
            if let Some(tool) = tool_of(ir, &address) {
                held.tool = Some(tool_view(&address, tool));
                match &tool.implementation {
                    ToolImplementation::Exec { exec } => held.exec = Some(exec_view(exec)),
                    ToolImplementation::Http { http } => held.http = Some(http_view(http)),
                    _ => {}
                }
            }
        }
        IrNodeKind::Flow {
            flow: target,
            context,
            policy,
        } => {
            held.subflow = Some(SubflowView {
                address: target.value.to_string(),
                context: context.map(|held| held.as_str().to_string()),
                policy: policy.as_ref().map(|policy| InstancePolicyView {
                    retry: policy
                        .retry
                        .as_ref()
                        .map(|retry| retry_view(retry, PolicyLevel::Node)),
                    timeout: policy
                        .timeout
                        .as_ref()
                        .map(|held| held.value.as_str().to_string()),
                    on_error: policy
                        .on_error
                        .as_ref()
                        .map(|held| on_error_strategy(held).0),
                }),
            });
        }
        IrNodeKind::Human { human } => held.human = Some(human_view(human)),
        IrNodeKind::Store { store, op, params } => {
            let address = store.value.to_string();
            if let Some(declared) = store_of(ir, &address) {
                held.store = Some(StoreView {
                    address,
                    kind: declared.kind.as_str().to_string(),
                    scope: declared.scope.as_str().to_string(),
                    op: op.as_str().to_string(),
                    backend: declared
                        .backend
                        .as_ref()
                        .map(|alias| alias.value.as_str().to_string()),
                    params: store_params(params),
                });
            }
        }
        IrNodeKind::Map { map } => {
            held.map = Some(map_view(ir, flow, node.id.value.as_str(), map));
        }
    }
    held
}

/// One satellite: the node an item's work actually happens on.
fn satellite(ir: &Ir, map: &str, route: &Route<'_>) -> GraphNode {
    let target = route.target.to_string();
    let mut held = pseudo(&route.id, address_kind(ir, &target));
    held.binding = Some(target.clone());
    held.description = match ir.definitions.get(&target).map(|held| &held.body) {
        Some(DefinitionBody::Agent(agent)) => {
            agent.description.as_ref().map(|text| text.value.clone())
        }
        Some(DefinitionBody::Tool(tool)) => Some(tool.description.value.clone()),
        Some(DefinitionBody::Flow(flow)) => {
            flow.description.as_ref().map(|text| text.value.clone())
        }
        _ => None,
    };
    held.input = route.input.map(input_view);
    held.writes = route
        .writes
        .map(|writes| write_views(ir, writes))
        .unwrap_or_default();
    held.dispatch = Some(DispatchView {
        map: map.to_string(),
        variant: route.variant.clone(),
        default: route.default,
        covers: route.covers.clone(),
        max_concurrency: route.max_concurrency,
        detach: route.detach,
    });
    held.schemas = schemas(
        dispatch_input_schema(ir, &target),
        dispatch_output_schema(ir, &target),
    );
    match ir.definitions.get(&target).map(|held| &held.body) {
        Some(DefinitionBody::Agent(_)) => held.agent = agent_view(ir, route.target),
        Some(DefinitionBody::Tool(tool)) => {
            held.tool = Some(tool_view(&target, tool));
            match &tool.implementation {
                ToolImplementation::Exec { exec } => held.exec = Some(exec_view(exec)),
                ToolImplementation::Http { http } => held.http = Some(http_view(http)),
                _ => {}
            }
        }
        Some(DefinitionBody::Flow(_)) => {
            held.subflow = Some(SubflowView {
                address: target,
                context: None,
                policy: None,
            });
        }
        _ => {}
    }
    held
}

const fn kind_of(kind: &IrNodeKind) -> NodeKind {
    match kind {
        IrNodeKind::Agent { .. } => NodeKind::Agent,
        IrNodeKind::Exec { .. } => NodeKind::Exec,
        IrNodeKind::Http { .. } => NodeKind::Http,
        IrNodeKind::Function { .. } => NodeKind::Function,
        IrNodeKind::Flow { .. } => NodeKind::Flow,
        IrNodeKind::Map { .. } => NodeKind::Map,
        IrNodeKind::Human { .. } => NodeKind::Human,
        IrNodeKind::Store { .. } => NodeKind::Store,
    }
}

/// The kind a dispatch target renders as: a `tool.*` reached from a `map` is
/// invoked the way a `function:` node invokes one (grammar 8.6 rule 2).
fn address_kind(ir: &Ir, address: &str) -> NodeKind {
    match ir.definitions.get(address).map(|held| &held.body) {
        Some(DefinitionBody::Agent(_)) => NodeKind::Agent,
        Some(DefinitionBody::Flow(_)) => NodeKind::Flow,
        _ => NodeKind::Function,
    }
}

/// The one line a node is read by under its id.
fn binding_of(node: &Node) -> Option<String> {
    match &node.kind {
        IrNodeKind::Agent { agent } => Some(agent.value.to_string()),
        IrNodeKind::Function { function } => Some(function.value.to_string()),
        IrNodeKind::Flow { flow, .. } => Some(flow.value.to_string()),
        IrNodeKind::Exec { exec } => Some(exec_line(exec)),
        IrNodeKind::Http { http } => Some(http_line(http)),
        IrNodeKind::Store { store, op, .. } => Some(format!("{} · {}", store.value, op.as_str())),
        IrNodeKind::Map { map } => Some(format!("over {}", map.over.value.as_str())),
        // A `human` node's badge already says what it is; what a reader cannot
        // see from the shape is how long it waits.
        IrNodeKind::Human { human } => human
            .timeout
            .as_ref()
            .map(|timeout| format!("waits {}", timeout.value.as_str())),
    }
}

fn definition_description(ir: &Ir, node: &Node) -> Option<String> {
    let address = match &node.kind {
        IrNodeKind::Agent { agent } => agent.value.to_string(),
        IrNodeKind::Function { function } => function.value.to_string(),
        IrNodeKind::Flow { flow, .. } => flow.value.to_string(),
        IrNodeKind::Store { store, .. } => store.value.to_string(),
        _ => return None,
    };
    match ir.definitions.get(&address).map(|held| &held.body) {
        Some(DefinitionBody::Agent(agent)) => {
            agent.description.as_ref().map(|text| text.value.clone())
        }
        Some(DefinitionBody::Tool(tool)) => Some(tool.description.value.clone()),
        Some(DefinitionBody::Flow(flow)) => {
            flow.description.as_ref().map(|text| text.value.clone())
        }
        Some(DefinitionBody::Store(store)) => {
            store.description.as_ref().map(|text| text.value.clone())
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Bindings, writes, policy
// ---------------------------------------------------------------------------

fn input_view(input: &NodeInput) -> InputView {
    match input {
        NodeInput::Scalar { value } => InputView::Scalar {
            value: value.value.as_str().to_string(),
        },
        NodeInput::Fields { bindings } => InputView::Fields {
            bindings: binding_views(bindings),
        },
    }
}

fn binding_views(bindings: &Bindings) -> Vec<BindingView> {
    bindings
        .entries
        .iter()
        .map(|entry| BindingView {
            name: entry.name.value.clone(),
            expression: entry.value.value.as_str().to_string(),
        })
        .collect()
}

fn interpolated_views(entries: &[InterpolatedEntry]) -> Vec<BindingView> {
    entries
        .iter()
        .map(|entry| BindingView {
            name: entry.name.value.clone(),
            expression: entry.value.value.as_str().to_string(),
        })
        .collect()
}

fn write_views(ir: &Ir, writes: &Writes) -> Vec<WriteView> {
    writes
        .entries
        .iter()
        .map(|entry| WriteView {
            field: entry.field.value.as_str().to_string(),
            channel: entry.channel.value.as_str().to_string(),
            reduce: channel_reduce(ir.state.as_ref(), entry.channel.value.as_str()),
        })
        .collect()
}

fn channel_reduce(state: Option<&Section<crate::ir::Channel>>, name: &str) -> Option<String> {
    state
        .and_then(|section| section.get(name))
        .and_then(|channel| channel.reduce)
        .map(|reduce| reduce.as_str().to_string())
}

fn policy_view(ir: &Ir, node: &Node) -> PolicyView {
    let resolved = policy::resolve(ir, node);
    let (strategy, target) = match resolved.on_error.0 {
        Strategy::Fail => ("fail".to_string(), None),
        Strategy::Skip => ("skip".to_string(), None),
        Strategy::Fallback(target) => ("fallback".to_string(), Some(control_target(target))),
    };
    PolicyView {
        retry: resolved
            .retry
            .0
            .zip(level(resolved.retry.1))
            .map(|(retry, at)| retry_view(retry, at)),
        timeout: resolved
            .timeout
            .0
            .zip(level(resolved.timeout.1))
            .map(|(timeout, at)| TimeoutView {
                value: timeout.as_str().to_string(),
                level: at,
            }),
        on_error: OnErrorView {
            strategy,
            target,
            // `on_error:` has no exemption — every node resolves one, at one of
            // the three levels (grammar 9.3) — so the bottom of the chain is
            // the answer no branch here reaches.
            level: level(resolved.on_error.1).unwrap_or(PolicyLevel::BuiltIn),
        },
    }
}

fn retry_view(retry: &Retry, at: PolicyLevel) -> RetryView {
    RetryView {
        max: retry.max,
        backoff: retry.backoff.value.as_str().to_string(),
        multiplier: retry.multiplier,
        max_backoff: retry
            .max_backoff
            .as_ref()
            .map(|held| held.value.as_str().to_string()),
        jitter: retry.jitter,
        level: at,
    }
}

/// The document's name for a level of grammar 9.3's chain.
///
/// `None` for the exemption a `human` node's `timeout:` and `retry:` carry
/// (Decision D102): it pairs only with a value that is absent, and this
/// document writes the exemption as the key's absence rather than as a level of
/// its own (`docs/graph.md` §5.2, §5.7).
const fn level(at: Level) -> Option<PolicyLevel> {
    match at {
        Level::Node => Some(PolicyLevel::Node),
        Level::Defaults => Some(PolicyLevel::Defaults),
        Level::BuiltIn => Some(PolicyLevel::BuiltIn),
        Level::Exempt => None,
    }
}

/// The `on_error:` of a level-1 override, which takes no `fallback`
/// (Decision D103) — the target half is answered all the same, so one reader
/// serves both levels.
fn on_error_strategy(on_error: &crate::ir::policy::OnError) -> (String, Option<String>) {
    match on_error {
        crate::ir::policy::OnError::Fail { .. } => ("fail".to_string(), None),
        crate::ir::policy::OnError::Skip { .. } => ("skip".to_string(), None),
        crate::ir::policy::OnError::Fallback { target, .. } => {
            ("fallback".to_string(), Some(control_target(&target.value)))
        }
    }
}

fn control_target(target: &ControlTarget) -> String {
    match target {
        ControlTarget::Node(id) => id.as_str().to_string(),
        ControlTarget::End => "end".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Per-kind blocks
// ---------------------------------------------------------------------------

fn exec_view(exec: &Exec) -> ExecView {
    ExecView {
        command: exec.command.value.as_str().to_string(),
        args: exec
            .args
            .iter()
            .map(|arg| arg.value.as_str().to_string())
            .collect(),
        cwd: exec.cwd.as_ref().map(|cwd| cwd.value.as_str().to_string()),
        env: interpolated_views(&exec.env),
        expect_exit: exec.expect_exit.clone(),
    }
}

fn exec_line(exec: &Exec) -> String {
    let mut text = exec.command.value.as_str().to_string();
    for arg in &exec.args {
        text.push(' ');
        text.push_str(arg.value.as_str());
    }
    text
}

fn http_view(http: &Http) -> HttpView {
    HttpView {
        method: http.method.value.as_str().to_string(),
        url: http.url.value.as_str().to_string(),
        headers: interpolated_views(&http.headers),
        query: http.query.as_ref().map(binding_views).unwrap_or_default(),
        body: http.body.as_ref().map(binding_views).unwrap_or_default(),
        expect_status: http.expect_status.clone(),
    }
}

fn http_line(http: &Http) -> String {
    format!("{} {}", http.method.value.as_str(), http.url.value.as_str())
}

fn human_view(human: &Human) -> HumanView {
    HumanView {
        timeout: human
            .timeout
            .as_ref()
            .map(|held| held.value.as_str().to_string()),
        on_timeout: human
            .on_timeout
            .as_ref()
            .map(|held| control_target(&held.value)),
    }
}

fn store_params(params: &StoreParams) -> Vec<BindingView> {
    let mut held = Vec::new();
    let mut push = |name: &str, expression: String| {
        held.push(BindingView {
            name: name.to_string(),
            expression,
        });
    };
    if let Some(key) = &params.key {
        push("key", key.value.as_str().to_string());
    }
    match &params.value {
        Some(StoreValue::Expression { value }) => push("value", value.value.as_str().to_string()),
        Some(StoreValue::Fields { bindings }) => {
            for entry in &bindings.entries {
                push(
                    &format!("value.{}", entry.name.value),
                    entry.value.value.as_str().to_string(),
                );
            }
        }
        None => {}
    }
    if let Some(query) = &params.query {
        push("query", query.value.as_str().to_string());
    }
    if let Some(prefix) = &params.prefix {
        push("prefix", prefix.value.as_str().to_string());
    }
    if let Some(top_k) = params.top_k {
        push("top_k", top_k.to_string());
    }
    if let Some(limit) = params.limit {
        push("limit", limit.to_string());
    }
    for (name, bindings) in [("filter", &params.filter), ("metadata", &params.metadata)] {
        let Some(bindings) = bindings else { continue };
        for entry in &bindings.entries {
            push(
                &format!("{name}.{}", entry.name.value),
                entry.value.value.as_str().to_string(),
            );
        }
    }
    if let Some(content_type) = &params.content_type {
        push("content_type", content_type.value.clone());
    }
    held
}

// ---------------------------------------------------------------------------
// Agents: model resolution and the tool surface
// ---------------------------------------------------------------------------

fn agent_view(ir: &Ir, address: &Address) -> Option<AgentView> {
    let address = address.to_string();
    let DefinitionBody::Agent(agent) = &ir.definitions.get(&address)?.body else {
        return None;
    };
    Some(AgentView {
        address,
        model: model_view(ir, &agent.model.value.to_string(), 0),
        tools: agent_tools(ir, agent),
        stores: agent
            .stores
            .iter()
            .map(|store| store.value.to_string())
            .collect(),
        prompt: agent.prompt.value.clone(),
        max_tool_iterations: agent.max_tool_iterations,
    })
}

/// One `model.*`, resolved through to its provider — and, for a route, through
/// each of its members in failover order (grammar 12.2).
///
/// `depth` bounds the walk. A route member is a `model.*` like any other and
/// the validator refuses a cycle, but a picture of a graph is not the place to
/// discover that a check has a hole in it.
fn model_view(ir: &Ir, address: &str, depth: usize) -> ModelView {
    let mut held = ModelView {
        address: address.to_string(),
        form: "direct".to_string(),
        description: None,
        id: None,
        provider: None,
        settings: Vec::new(),
        route: Vec::new(),
        route_on: Vec::new(),
    };
    let Some(DefinitionBody::Model(model)) = ir.definitions.get(address).map(|held| &held.body)
    else {
        return held;
    };
    match model {
        Model::Direct(direct) => {
            held.description = direct.description.as_ref().map(|text| text.value.clone());
            held.id = Some(direct.id.value.clone());
            held.provider = Some(provider_view(ir, &direct.provider.value.to_string()));
            held.settings = direct
                .settings
                .iter()
                .map(|(name, value)| SettingView {
                    name: name.clone(),
                    value: summary::literal_json(&value.value),
                })
                .collect();
        }
        Model::Route(route) => {
            held.form = "route".to_string();
            held.description = route.description.as_ref().map(|text| text.value.clone());
            held.route_on = route
                .route_on
                .as_ref()
                .map(|conditions| {
                    conditions
                        .iter()
                        .map(|condition| condition.value.as_str().to_string())
                        .collect()
                })
                .unwrap_or_default();
            if depth < 4 {
                held.route = route
                    .route
                    .iter()
                    .map(|member| model_view(ir, &member.value.to_string(), depth + 1))
                    .collect();
            }
        }
    }
    held
}

fn provider_view(ir: &Ir, address: &str) -> ProviderView {
    let mut held = ProviderView {
        address: address.to_string(),
        kind: String::new(),
        config: Vec::new(),
        server_tools: Vec::new(),
    };
    let Some(DefinitionBody::Provider(provider)) =
        ir.definitions.get(address).map(|held| &held.body)
    else {
        return held;
    };
    held.kind = provider.kind.as_str().to_string();
    held.config = provider_config(provider);
    held.server_tools = provider
        .config
        .server_tools
        .iter()
        .map(|tool| tool.type_name.value.clone())
        .collect();
    held
}

/// The connection keys a provider declares, sorted by key.
///
/// Every `${ENV}` reference reaches this document **as written** — the IR never
/// substitutes one and neither does this (PRD 5.9, resolved q15). A picture of
/// a composition that resolved a credential would be a credential in a file
/// somebody is about to share.
fn provider_config(provider: &Provider) -> Vec<SettingView> {
    let config = &provider.config;
    let mut held: BTreeMap<String, String> = BTreeMap::new();
    let credentials: [(&str, Option<&crate::diag::Spanned<EnvRef>>); 6] = [
        ("api_key", config.api_key.as_ref()),
        ("base_url", config.base_url.as_ref()),
        ("access_key_id", config.access_key_id.as_ref()),
        ("secret_access_key", config.secret_access_key.as_ref()),
        ("session_token", config.session_token.as_ref()),
        ("credentials_json", config.credentials_json.as_ref()),
    ];
    for (name, value) in credentials {
        if let Some(value) = value {
            held.insert(name.to_string(), value.value.as_str().to_string());
        }
    }
    for (name, value) in [
        ("api_version", config.api_version.as_ref()),
        ("organization", config.organization.as_ref()),
        ("region", config.region.as_ref()),
        ("location", config.location.as_ref()),
        ("project", config.project.as_ref()),
        ("profile", config.profile.as_ref()),
    ] {
        if let Some(value) = value {
            held.insert(name.to_string(), value.value.as_str().to_string());
        }
    }
    for header in &config.headers {
        held.insert(
            format!("headers.{}", header.name.value),
            header.value.value.as_str().to_string(),
        );
    }
    held.into_iter()
        .map(|(name, value)| SettingView {
            name,
            value: serde_json::Value::String(value),
        })
        .collect()
}

/// Every tool on an agent's wire: declared, built-in, then synthesized from an
/// attached store (grammar 5.5, 11.5, PRD 5.8).
fn agent_tools(ir: &Ir, agent: &Agent) -> Vec<ToolView> {
    let mut held = Vec::new();
    for attached in &agent.tools {
        let address = attached.value.to_string();
        match ir.definitions.get(&address).map(|entry| &entry.body) {
            Some(DefinitionBody::Tool(tool)) => held.push(tool_view(&address, tool)),
            Some(DefinitionBody::Flow(flow)) => held.push(ToolView {
                name: attached.value.name.as_str().to_string(),
                source: ToolSource::Flow,
                address: Some(address),
                binding: None,
                op: None,
                description: flow.description.as_ref().map(|text| text.value.clone()),
                detail: Some("a flow instantiated once per model tool call".to_string()),
            }),
            _ => {}
        }
    }
    for builtin in &agent.builtins {
        held.push(ToolView {
            name: builtin.value.as_str().to_string(),
            source: ToolSource::Builtin,
            address: Some(builtin.value.address().to_string()),
            binding: Some("builtin".to_string()),
            op: None,
            description: Some(model::builtin_description(builtin.value)),
            detail: Some(
                "attached by the `tools:` shorthand, under its default bounds".to_string(),
            ),
        });
    }
    for attached in &agent.stores {
        let address = attached.value.to_string();
        let Some(store) = store_of(ir, &address) else {
            continue;
        };
        let access = store.agent_access.unwrap_or(AgentAccess::ReadWrite);
        let local = attached.value.name.as_str();
        for op in model::store_tools(store.kind, access) {
            held.push(ToolView {
                name: model::store_tool_name(local, *op),
                source: ToolSource::Store,
                address: Some(address.clone()),
                binding: None,
                op: Some(op.as_str().to_string()),
                description: store.description.as_ref().map(|text| text.value.clone()),
                detail: Some(format!(
                    "synthesized from {address} ({}, agent_access: {})",
                    store.kind.as_str(),
                    access.as_str()
                )),
            });
        }
    }
    held
}

fn tool_view(address: &str, tool: &Tool) -> ToolView {
    let (binding, detail) = match &tool.implementation {
        ToolImplementation::Exec { exec } => ("exec", Some(exec_line(exec))),
        ToolImplementation::Http { http } => ("http", Some(http_line(http))),
        ToolImplementation::Function { function } => {
            ("function", Some(function.name.value.as_str().to_string()))
        }
        ToolImplementation::Module { module } => ("module", Some(module.path.value.clone())),
        ToolImplementation::Builtin { builtin } => (
            "builtin",
            Some(format!(
                "{}, on the wire as `{}`",
                builtin.builtin.value.address(),
                builtin.builtin.value.as_str()
            )),
        ),
    };
    ToolView {
        name: address
            .split_once('.')
            .map_or(address, |(_, local)| local)
            .to_string(),
        source: ToolSource::Tool,
        address: Some(address.to_string()),
        binding: Some(binding.to_string()),
        op: None,
        description: Some(tool.description.value.clone()),
        detail,
    }
}

fn tool_of<'ir>(ir: &'ir Ir, address: &str) -> Option<&'ir Tool> {
    match ir.definitions.get(address).map(|held| &held.body) {
        Some(DefinitionBody::Tool(tool)) => Some(tool),
        _ => None,
    }
}

fn store_of<'ir>(ir: &'ir Ir, address: &str) -> Option<&'ir Store> {
    match ir.definitions.get(address).map(|held| &held.body) {
        Some(DefinitionBody::Store(store)) => Some(store),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

/// The pair, or nothing at all: a node with neither half — a `map`, which
/// declares no surface of its own — should carry no key rather than an empty
/// object.
fn schemas(input: Option<SchemaView>, output: Option<SchemaView>) -> Option<SchemasView> {
    (input.is_some() || output.is_some()).then_some(SchemasView { input, output })
}

fn input_schema(ir: &Ir, node: &Node) -> Option<SchemaView> {
    match &node.kind {
        IrNodeKind::Agent { agent } => {
            let DefinitionBody::Agent(declared) =
                &ir.definitions.get(&agent.value.to_string())?.body
            else {
                return None;
            };
            Some(match &declared.input {
                Some(input) => SchemaView {
                    source: SchemaSource::Declared,
                    fields: summary::fields(input),
                },
                None => SchemaView {
                    source: SchemaSource::StringInput,
                    fields: Vec::new(),
                },
            })
        }
        IrNodeKind::Function { function } => {
            let tool = tool_of(ir, &function.value.to_string())?;
            Some(SchemaView {
                source: SchemaSource::Declared,
                fields: summary::fields(&tool.input),
            })
        }
        IrNodeKind::Flow { flow, .. } => {
            let DefinitionBody::Flow(declared) = &ir.definitions.get(&flow.value.to_string())?.body
            else {
                return None;
            };
            Some(SchemaView {
                source: SchemaSource::Declared,
                fields: declared
                    .inputs
                    .as_ref()
                    .map(summary::fields)
                    .unwrap_or_default(),
            })
        }
        IrNodeKind::Human { human } => Some(SchemaView {
            source: SchemaSource::Declared,
            fields: summary::fields(&human.input),
        }),
        _ => None,
    }
}

fn dispatch_input_schema(ir: &Ir, address: &str) -> Option<SchemaView> {
    match ir.definitions.get(address).map(|held| &held.body) {
        Some(DefinitionBody::Agent(agent)) => Some(match &agent.input {
            Some(input) => SchemaView {
                source: SchemaSource::Declared,
                fields: summary::fields(input),
            },
            None => SchemaView {
                source: SchemaSource::StringInput,
                fields: Vec::new(),
            },
        }),
        Some(DefinitionBody::Tool(tool)) => Some(SchemaView {
            source: SchemaSource::Declared,
            fields: summary::fields(&tool.input),
        }),
        Some(DefinitionBody::Flow(flow)) => Some(SchemaView {
            source: SchemaSource::Declared,
            fields: flow
                .inputs
                .as_ref()
                .map(summary::fields)
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

fn dispatch_output_schema(ir: &Ir, address: &str) -> Option<SchemaView> {
    let fields = match ir.definitions.get(address).map(|held| &held.body) {
        Some(DefinitionBody::Agent(agent)) => summary::fields(&agent.output),
        Some(DefinitionBody::Tool(tool)) => summary::fields(&tool.output),
        Some(DefinitionBody::Flow(flow)) => summary::fields(&flow.outputs),
        _ => return None,
    };
    Some(SchemaView {
        source: SchemaSource::Declared,
        fields,
    })
}

/// The field map a node answers with, and whether the composition declared it
/// or the construct's kind supplied it.
fn output_schema(ir: &Ir, node: &Node) -> Option<(SchemaSource, FieldMap)> {
    match &node.kind {
        IrNodeKind::Agent { agent } => match &ir.definitions.get(&agent.value.to_string())?.body {
            DefinitionBody::Agent(declared) => {
                Some((SchemaSource::Declared, declared.output.clone()))
            }
            _ => None,
        },
        IrNodeKind::Function { function } => Some((
            SchemaSource::Declared,
            tool_of(ir, &function.value.to_string())?.output.clone(),
        )),
        IrNodeKind::Exec { exec } => Some(match &exec.output {
            Some(output) => (SchemaSource::Declared, output.clone()),
            None => (
                SchemaSource::KindDefault,
                model::exec_default_output(&exec.span),
            ),
        }),
        IrNodeKind::Http { http } => Some(match &http.output {
            Some(output) => (SchemaSource::Declared, output.clone()),
            None => (
                SchemaSource::KindDefault,
                model::http_default_output(&http.span),
            ),
        }),
        IrNodeKind::Flow { flow, .. } => match &ir.definitions.get(&flow.value.to_string())?.body {
            DefinitionBody::Flow(declared) => {
                Some((SchemaSource::Declared, declared.outputs.clone()))
            }
            _ => None,
        },
        IrNodeKind::Human { human } => Some((SchemaSource::Declared, human.output.clone())),
        IrNodeKind::Store { store, op, params } => {
            let declared = store_of(ir, &store.value.to_string())?;
            let bound = match declared.kind {
                StoreKind::Vector => params.top_k,
                _ => params.limit,
            };
            Some((
                SchemaSource::KindDefault,
                model::store_output(
                    declared.kind,
                    *op,
                    declared.value_schema.as_ref(),
                    declared.metadata_schema.as_ref(),
                    bound,
                    &node.span,
                ),
            ))
        }
        // A `map` writes through its dispatches and answers with nothing of its
        // own (grammar 8.6 rule 9).
        IrNodeKind::Map { .. } => None,
    }
}

/// The field map a node's `<node>.output` selector reads, which is what a
/// `map`'s `over:` walks through.
fn node_output_map(ir: &Ir, node: &Node) -> Option<FieldMap> {
    output_schema(ir, node).map(|(_, fields)| fields)
}

// ---------------------------------------------------------------------------
// Maps
// ---------------------------------------------------------------------------

/// One route of a map, with everything the satellite and the edge both need.
struct Route<'ir> {
    id: String,
    variant: Option<String>,
    default: bool,
    covers: Vec<String>,
    target: &'ir Address,
    max_concurrency: Option<i64>,
    input: Option<&'ir NodeInput>,
    writes: Option<&'ir Writes>,
    detach: Option<bool>,
}

fn routes<'ir>(ir: &Ir, flow: &Flow, id: &str, map: &'ir Map) -> Vec<Route<'ir>> {
    match &map.dispatch {
        MapDispatch::Homogeneous {
            node,
            input,
            writes,
            detach,
        } => vec![Route {
            id: format!("{id}/(item)"),
            variant: None,
            default: false,
            covers: Vec::new(),
            target: &node.value,
            max_concurrency: None,
            input: input.as_ref(),
            writes: writes.as_ref(),
            detach: detach.as_ref().map(|held| held.value),
        }],
        MapDispatch::Routed {
            routes, default, ..
        } => {
            let declared = variants(ir, flow, map);
            let named: Vec<String> = routes
                .iter()
                .filter_map(|route| route.tag.as_ref())
                .map(|tag| tag.value.as_str().to_string())
                .collect();
            let mut held: Vec<Route<'ir>> = routes
                .iter()
                .map(|route| {
                    let tag = route
                        .tag
                        .as_ref()
                        .map(|tag| tag.value.as_str().to_string())
                        .unwrap_or_default();
                    let covers = vec![tag.clone()];
                    route_of(id, &tag, Some(tag.clone()), false, covers, route)
                })
                .collect();
            if let Some(route) = default {
                // Grammar 8.6 rule 4: `default:` sees every variant no named
                // route claims, which is what narrows the payload its target is
                // checked against (Decision D30).
                let covers: Vec<String> = declared
                    .into_iter()
                    .filter(|variant| !named.contains(variant))
                    .collect();
                // `(default)` rather than `default`: a variant tag is an
                // identifier (grammar 3.8), so a union may declare one spelled
                // `default`, and a satellite id the author's tag can also spell
                // would collide with it.
                held.push(route_of(id, "(default)", None, true, covers, route));
            }
            held
        }
    }
}

fn route_of<'ir>(
    map: &str,
    slug: &str,
    variant: Option<String>,
    default: bool,
    covers: Vec<String>,
    route: &'ir MapRoute,
) -> Route<'ir> {
    Route {
        id: format!("{map}/{slug}"),
        variant,
        default,
        covers,
        target: &route.node.value,
        max_concurrency: route.max_concurrency,
        input: route.input.as_ref(),
        writes: route.writes.as_ref(),
        detach: route.detach.as_ref().map(|held| held.value),
    }
}

fn map_view(ir: &Ir, flow: &Flow, id: &str, map: &Map) -> MapView {
    let array = over_array(ir, flow, map);
    let (dispatch, route_by) = match &map.dispatch {
        MapDispatch::Homogeneous { .. } => ("homogeneous", None),
        MapDispatch::Routed { route_by, .. } => {
            ("routed", Some(route_by.value.as_str().to_string()))
        }
    };
    MapView {
        over: map.over.value.as_str().to_string(),
        max_items: array.as_ref().and_then(|node| match &node.form {
            TypeForm::Array(array) => array.max_items,
            _ => None,
        }),
        item_binding: map.item_binding.as_ref().map_or_else(
            || "item".to_string(),
            |name| name.value.as_str().to_string(),
        ),
        max_concurrency: map.max_concurrency,
        dispatch: dispatch.to_string(),
        route_by,
        on_item_error: map.on_item_error.as_ref().map(item_error_view),
        join: "the map node's outgoing edges fire once every dispatched instance has completed or \
               been resolved by `on_item_error`"
            .to_string(),
        routes: routes(ir, flow, id, map)
            .into_iter()
            .map(|route| RouteView {
                node: route.id,
                variant: route.variant,
                default: route.default,
                covers: route.covers,
                target: route.target.to_string(),
                max_concurrency: route.max_concurrency,
                input: route.input.map(input_view),
                writes: route
                    .writes
                    .map(|writes| write_views(ir, writes))
                    .unwrap_or_default(),
                detach: route.detach,
            })
            .collect(),
    }
}

fn item_error_view(on_item_error: &ItemError) -> ItemErrorView {
    match on_item_error {
        ItemError::Fail { .. } => ItemErrorView {
            strategy: "fail".to_string(),
            retry: None,
        },
        ItemError::Skip { .. } => ItemErrorView {
            strategy: "skip".to_string(),
            retry: None,
        },
        ItemError::Retry { retry, .. } => ItemErrorView {
            strategy: "retry".to_string(),
            // Carried inline: there is no chain for a per-item `retry` to
            // inherit a bound from (grammar 8.6 rule 10).
            retry: Some(retry_view(retry, PolicyLevel::Node)),
        },
    }
}

/// The union variants a routed map's items declare, in declaration order.
fn variants(ir: &Ir, flow: &Flow, map: &Map) -> Vec<String> {
    let Some(array) = over_array(ir, flow, map) else {
        return Vec::new();
    };
    let TypeForm::Array(array) = &array.form else {
        return Vec::new();
    };
    match &array.items.form {
        TypeForm::Union(union) => union
            .variants
            .iter()
            .map(|variant| variant.tag.value.as_str().to_string())
            .collect(),
        _ => Vec::new(),
    }
}

/// The array a `map`'s `over:` resolves to (grammar 4.2, 8.6 rule 1).
///
/// The walk is [`crate::check::maps::walk`]'s, over the same three roots the
/// validator reads it through — so what a picture says the fan-out is bounded
/// by is what the validator proved it is bounded by. A composition that reached
/// this point has been validated, so a `None` here means a root this compiler
/// does not know rather than a spec that is wrong.
fn over_array(ir: &Ir, flow: &Flow, map: &Map) -> Option<TypeNode> {
    let path = &map.over.value;
    let steps = &path.steps;
    let (root, consumed) = match path.root.as_str() {
        "state" => {
            let PathStep::Field(name) = steps.first()? else {
                return None;
            };
            (ir.state.as_ref()?.get(name.as_str())?.ty.clone(), 1_usize)
        }
        "input" => {
            let PathStep::Field(name) = steps.first()? else {
                return None;
            };
            (flow.inputs.as_ref()?.field(name.as_str())?.ty.clone(), 1)
        }
        id => {
            let producer = flow
                .nodes
                .iter()
                .find(|node| node.id.value.as_str() == id)?;
            let output = node_output_map(ir, producer)?;
            let (PathStep::Field(selector), PathStep::Field(name)) =
                (steps.first()?, steps.get(1)?)
            else {
                return None;
            };
            if selector.as_str() != "output" {
                return None;
            }
            (output.field(name.as_str())?.ty.clone(), 2)
        }
    };
    crate::check::maps::walk(root, &steps[consumed.min(steps.len())..])
}

// ---------------------------------------------------------------------------
// Edges
// ---------------------------------------------------------------------------

fn edges(ir: &Ir, flow: &Flow) -> Vec<GraphEdge> {
    let mut held: Vec<GraphEdge> = Vec::new();
    let barriers: Vec<&str> = flow
        .nodes
        .iter()
        .filter(|node| matches!(node.kind, IrNodeKind::Map { .. }))
        .map(|node| node.id.value.as_str())
        .collect();

    for edge in &flow.edges {
        let from = match &edge.from.value {
            EdgeSource::Start => "start".to_string(),
            EdgeSource::Node(id) => id.as_str().to_string(),
        };
        let to = match &edge.to.value {
            EdgeTarget::End => "end".to_string(),
            EdgeTarget::Node(id) => id.as_str().to_string(),
        };
        let (class, label) = match (&edge.when, &edge.else_edge) {
            (Some(when), _) => (
                EdgeClass::Conditional,
                Some(when.value.as_str().to_string()),
            ),
            (None, Some(_)) => (EdgeClass::Default, Some("else".to_string())),
            (None, None) => (EdgeClass::Unconditional, None),
        };
        let join_barrier = barriers.contains(&from.as_str());
        held.push(GraphEdge {
            from,
            to,
            class,
            label,
            when: edge
                .when
                .as_ref()
                .map(|when| when.value.as_str().to_string()),
            max_iterations: edge.max_iterations,
            join_barrier,
        });
    }

    for node in &flow.nodes {
        let id = node.id.value.as_str();
        if let IrNodeKind::Map { map } = &node.kind {
            for route in routes(ir, flow, id, map) {
                let label = match (&route.variant, route.default) {
                    (Some(variant), _) => variant.clone(),
                    (None, true) => "default".to_string(),
                    (None, false) => "each item".to_string(),
                };
                held.push(GraphEdge {
                    from: id.to_string(),
                    to: route.id,
                    class: EdgeClass::MapRoute,
                    label: Some(label),
                    when: None,
                    max_iterations: None,
                    join_barrier: false,
                });
            }
        }
        if let Some(crate::ir::policy::OnError::Fallback { target, .. }) = &node.policy.on_error {
            held.push(GraphEdge {
                from: id.to_string(),
                to: control_target(&target.value),
                class: EdgeClass::ErrorFallback,
                label: Some("on_error: fallback".to_string()),
                when: None,
                max_iterations: None,
                join_barrier: false,
            });
        }
        if let IrNodeKind::Human { human } = &node.kind
            && let Some(target) = &human.on_timeout
        {
            let label = human.timeout.as_ref().map_or_else(
                || "on_timeout".to_string(),
                |timeout| format!("on_timeout ({})", timeout.value.as_str()),
            );
            held.push(GraphEdge {
                from: id.to_string(),
                to: control_target(&target.value),
                class: EdgeClass::Timeout,
                label: Some(label),
                when: None,
                max_iterations: None,
                join_barrier: false,
            });
        }
    }
    held
}

// ---------------------------------------------------------------------------
// Triggers
// ---------------------------------------------------------------------------

fn triggers(ir: &Ir, flow: &str) -> Vec<TriggerView> {
    let Some(section) = &ir.triggers else {
        return Vec::new();
    };
    section
        .entries
        .values()
        .filter(|trigger| trigger.flow.value.to_string() == flow)
        .map(|trigger| {
            let (ty, summary, input) = match &trigger.kind {
                TriggerKind::Manual => (
                    "manual",
                    "invoked from the CLI or the SDK".to_string(),
                    Vec::new(),
                ),
                TriggerKind::Http(http) => {
                    let method = http
                        .method
                        .map_or("POST", crate::ast::trigger::TriggerMethod::as_str);
                    let path = http.path.as_ref().map_or_else(
                        || format!("/triggers/{}", trigger.name.value),
                        |path| path.value.clone(),
                    );
                    let respond = http
                        .respond
                        .map_or("async", crate::ast::trigger::Respond::as_str);
                    (
                        "http",
                        format!("{method} {path} · respond {respond}"),
                        http.input.as_ref().map(binding_views).unwrap_or_default(),
                    )
                }
                TriggerKind::Schedule(schedule) => {
                    let zone = schedule
                        .timezone
                        .as_ref()
                        .map_or("UTC", |zone| zone.value.as_str());
                    (
                        "schedule",
                        format!("cron {} ({zone}) · reserved in v0", schedule.cron.value),
                        schedule
                            .input
                            .as_ref()
                            .map(binding_views)
                            .unwrap_or_default(),
                    )
                }
                TriggerKind::Event(event) => (
                    "event",
                    format!("source {} · reserved in v0", event.source.value.as_str()),
                    event.input.as_ref().map(binding_views).unwrap_or_default(),
                ),
            };
            TriggerView {
                name: trigger.name.value.as_str().to_string(),
                ty: ty.to_string(),
                summary,
                session_key: trigger
                    .session_key
                    .as_ref()
                    .map(|key| key.value.as_str().to_string()),
                description: trigger.description.as_ref().map(|text| text.value.clone()),
                input,
            }
        })
        .collect()
}
