//! Two traversals the schema checks need, and one place each is defined.
//!
//! # Component reachability (grammar 7.7)
//!
//! A flow **reaches** the components its own nodes name, its maps' dispatch
//! targets, the stores and tools of every agent it reaches, and everything the
//! flows it reaches reach in turn. One relation serves three checks; the one
//! that is this pass's is session coherence (grammar 11.3), so [`stores_of`]
//! is stated over stores. The other two — sync-trigger interrupt-freedom and
//! recursion — are the graph pass's, and are the reason this is a module rather
//! than a private helper of `stores`.
//!
//! # Dispatch sites (grammar 11.4, Decision D83)
//!
//! A store node lives inside a `flow.*`, so it is inside a fan-out exactly when
//! that flow is a `map` dispatch target, directly or through `flow:` nodes.
//! Derivation is computed **per dispatch site**, because one flow may be
//! dispatched by several maps and instantiated outside every map as well.
//! [`frames`] enumerates those sites: one [`Frame`] per (site, flow instance)
//! pair, carrying which of that instance's input fields are item-derived.
//!
//! The seeding of a site depends on nothing outside the map block itself — a
//! nested map re-roots the computation at its own item — so a map's frames are
//! the same wherever the map's own flow is instantiated. What propagates is the
//! inside of a dispatched flow: a `flow:` node there carries derivation inward
//! exactly when its binding expression is item-derived in the frame it sits in.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::Namespace;
use crate::cel::Scope;
use crate::diag::{Span, Spanned};
use crate::ir::binding::NodeInput;
use crate::ir::flow::{Flow, MapDispatch, MapRoute, NodeKind};

use super::Ctx;

/// The store addresses a flow reaches (grammar 7.7).
pub(crate) fn stores_of(ctx: &Ctx, flow: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    let mut seen = BTreeSet::new();
    walk_stores(ctx, flow, &mut seen, &mut found);
    found
}

fn walk_stores(
    ctx: &Ctx,
    address: &str,
    seen: &mut BTreeSet<String>,
    found: &mut BTreeSet<String>,
) {
    if !seen.insert(address.to_string()) {
        return;
    }
    let Some(flow) = ctx.flow_named(address) else {
        return;
    };
    for node in &flow.nodes {
        match &node.kind {
            NodeKind::Store { store, .. } => {
                found.insert(store.value.to_string());
            }
            NodeKind::Agent { agent } => agent_reaches(ctx, &agent.value.to_string(), seen, found),
            NodeKind::Flow { flow, .. } => {
                walk_stores(ctx, &flow.value.to_string(), seen, found);
            }
            NodeKind::Map { map } => {
                for target in targets(&map.dispatch) {
                    match target.namespace {
                        Namespace::Agent => agent_reaches(ctx, &target.to_string(), seen, found),
                        Namespace::Flow => walk_stores(ctx, &target.to_string(), seen, found),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
}

/// An agent reaches the stores it attaches and everything its `flow.*` tools
/// reach — flow-as-tool attachment is a call (grammar 7.7 clauses 3, 4).
fn agent_reaches(
    ctx: &Ctx,
    address: &str,
    seen: &mut BTreeSet<String>,
    found: &mut BTreeSet<String>,
) {
    let Some(agent) =
        ctx.ir
            .definitions
            .get(address)
            .and_then(|definition| match &definition.body {
                crate::ir::definition::DefinitionBody::Agent(agent) => Some(agent),
                _ => None,
            })
    else {
        return;
    };
    for store in &agent.stores {
        found.insert(store.value.to_string());
    }
    for tool in &agent.tools {
        if tool.value.namespace == Namespace::Flow {
            walk_stores(ctx, &tool.value.to_string(), seen, found);
        }
    }
}

/// Every dispatch target of a `map` block, in declaration order.
pub(crate) fn targets(dispatch: &MapDispatch) -> Vec<&crate::ast::common::Address> {
    match dispatch {
        MapDispatch::Homogeneous { node, .. } => vec![&node.value],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .map(|route| &route.node.value)
            .chain(default.iter().map(|route| &route.node.value))
            .collect(),
    }
}

/// One flow instance running inside a fan-out.
pub(crate) struct Frame<'a> {
    /// The dispatched flow's address.
    pub(crate) address: String,
    /// Its definition.
    pub(crate) flow: &'a Flow,
    /// Which of this instance's input fields are item-derived at this site
    /// (Decision D83).
    pub(crate) derived: BTreeMap<String, bool>,
    /// How the dispatching map is named in a diagnostic.
    pub(crate) dispatcher: String,
    /// The dispatching map node's span.
    pub(crate) span: Span,
    /// `detach: true` on the dispatch that opened this site, and where it was
    /// written. Carried inward: a `flow:` node inside a detached instance is
    /// dispatched no less fire-and-forget than its caller (grammar 8.6 rule 7).
    pub(crate) detached: Option<Spanned<bool>>,
}

/// Every flow instance any `map` in the composition dispatches, directly or
/// through nested `flow:` nodes.
pub(crate) fn frames<'a>(ctx: &Ctx<'a>) -> Vec<Frame<'a>> {
    let mut frames = Vec::new();
    for (address, definition) in &ctx.ir.definitions {
        let crate::ir::definition::DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        for node in &flow.nodes {
            let NodeKind::Map { map } = &node.kind else {
                continue;
            };
            let dispatcher = format!("the map `{}` of `{address}`", node.id.value);
            let item = map
                .item_binding
                .as_ref()
                .map_or("item", |binding| binding.value.as_str());
            type Dispatch<'a> = (
                &'a crate::ast::common::Address,
                Option<&'a NodeInput>,
                Option<&'a Spanned<bool>>,
            );
            let routes: Vec<Dispatch<'_>> = match &map.dispatch {
                MapDispatch::Homogeneous {
                    node,
                    input,
                    detach,
                    ..
                } => {
                    vec![(&node.value, input.as_ref(), detach.as_ref())]
                }
                MapDispatch::Routed {
                    routes, default, ..
                } => routes
                    .iter()
                    .chain(default.iter().map(|route| &**route))
                    .map(|route: &MapRoute| {
                        (
                            &route.node.value,
                            route.input.as_ref(),
                            route.detach.as_ref(),
                        )
                    })
                    .collect(),
            };
            for (target, input, detach) in routes {
                if target.namespace != Namespace::Flow {
                    continue;
                }
                let address = target.to_string();
                let Some(dispatched) = ctx.flow_named(&address) else {
                    continue;
                };
                let derived = seed(dispatched, input, item);
                let mut path = BTreeSet::new();
                push_frame(
                    ctx,
                    &mut frames,
                    &mut path,
                    Frame {
                        address,
                        flow: dispatched,
                        derived,
                        dispatcher: dispatcher.clone(),
                        span: node.span.clone(),
                        detached: detach.filter(|detach| detach.value).cloned(),
                    },
                );
            }
        }
    }
    frames
}

/// Record a frame and follow the `flow:` nodes inside it, carrying derivation
/// inward through their bindings (Decision D83).
///
/// A frame that repeats one already recorded — the same flow, dispatched by the
/// same map with the same derivation and the same detachment — is dropped: it
/// is the same *site* said twice, and every rule stated over frames would
/// otherwise report one mistake once per repetition. Two routes of one map that
/// differ in `detach:` are two sites, not one, because the rules stated over a
/// frame read that key.
fn push_frame<'a>(
    ctx: &Ctx<'a>,
    frames: &mut Vec<Frame<'a>>,
    path: &mut BTreeSet<String>,
    frame: Frame<'a>,
) {
    if frames.iter().any(|recorded| {
        recorded.address == frame.address
            && recorded.dispatcher == frame.dispatcher
            && recorded.derived == frame.derived
            && recorded.detached.is_some() == frame.detached.is_some()
    }) {
        return;
    }
    // A flow that reaches itself is refused by the graph pass's recursion
    // check; guarding the path here keeps this traversal finite meanwhile.
    let address = frame.address.clone();
    if !path.insert(address.clone()) {
        return;
    }
    let flow = frame.flow;
    let derived = frame.derived.clone();
    let dispatcher = frame.dispatcher.clone();
    let span = frame.span.clone();
    let detached = frame.detached.clone();
    frames.push(frame);
    for node in &flow.nodes {
        let NodeKind::Flow { flow: target, .. } = &node.kind else {
            continue;
        };
        let address = target.value.to_string();
        let Some(nested) = ctx.flow_named(&address) else {
            continue;
        };
        let mut inner = BTreeMap::new();
        if let Some(inputs) = &nested.inputs {
            for field in &inputs.fields {
                let name = field.name.value.to_string();
                let value = match &node.input {
                    Some(NodeInput::Fields { bindings }) => bindings
                        .entries
                        .iter()
                        .find(|binding| binding.name.value == name)
                        .is_some_and(|binding| {
                            is_item_derived(binding.value.value.as_str(), None, &derived)
                        }),
                    _ => false,
                };
                inner.insert(name, value);
            }
        }
        push_frame(
            ctx,
            frames,
            path,
            Frame {
                address,
                flow: nested,
                derived: inner,
                dispatcher: dispatcher.clone(),
                span: span.clone(),
                detached: detached.clone(),
            },
        );
    }
    path.remove(&address);
}

/// Which input fields of a dispatched flow are item-derived at one site
/// (Decision D83).
fn seed(flow: &Flow, input: Option<&NodeInput>, item: &str) -> BTreeMap<String, bool> {
    let mut derived = BTreeMap::new();
    let Some(inputs) = &flow.inputs else {
        return derived;
    };
    for field in &inputs.fields {
        let name = field.name.value.to_string();
        let value = match input {
            // The whole item is the instance's input, so every field is
            // item-derived.
            None => true,
            Some(NodeInput::Fields { bindings }) => bindings
                .entries
                .iter()
                .find(|binding| binding.name.value == name)
                .is_some_and(|binding| {
                    is_item_derived(binding.value.value.as_str(), Some(item), &BTreeMap::new())
                }),
            // The scalar form binds a string-in agent, and an agent contains no
            // store nodes (grammar 11.4).
            Some(NodeInput::Scalar { .. }) => false,
        };
        derived.insert(name, value);
    }
    derived
}

/// Whether one expression is item-derived: it references the item binding, the
/// per-item index, or an `input.<field>` that is itself item-derived here.
pub(crate) fn is_item_derived(
    source: &str,
    item: Option<&str>,
    derived: &BTreeMap<String, bool>,
) -> bool {
    let analysis = crate::cel::analyze(source, &Scope::default());
    analysis.reads.iter().any(|read| {
        if item.is_some_and(|item| read.root == item) {
            return true;
        }
        if read.root == "execution" && read.path.first().is_some_and(|first| first == "item_index")
        {
            return true;
        }
        read.root == "input"
            && read.path.first().map_or_else(
                // A read of the whole `input` object embeds every field.
                || derived.values().any(|value| *value),
                |field| derived.get(field).copied().unwrap_or(false),
            )
    })
}
