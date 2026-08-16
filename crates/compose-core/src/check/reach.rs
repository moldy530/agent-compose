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
//!
//! # Detached instances (grammar 8.6 rule 7, Decision D94)
//!
//! Detachment propagates by a different relation, so it is a different walk.
//! Derivation re-roots at a nested map's own item; detachment does not re-root
//! at anything — a detached dispatch is resolved the moment it is issued, so
//! *everything* the dispatched instance goes on to do happens after the join,
//! whether the instance reaches it through a `flow:` node or through a `map` of
//! its own. [`detached_instances`] is that reachability, one record per
//! (detaching dispatch, flow it reaches).

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::{Address, Namespace};
use crate::cel::Scope;
use crate::diag::{Span, Spanned};
use crate::ir::Ir;
use crate::ir::binding::{NodeInput, Writes};
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::{Flow, MapDispatch, NodeKind};

use super::Ctx;

/// The flow definition at this address.
pub(crate) fn flow_at<'a>(ir: &'a Ir, address: &str) -> Option<&'a Flow> {
    match ir.definitions.get(address).map(|found| &found.body) {
        Some(DefinitionBody::Flow(flow)) => Some(flow),
        _ => None,
    }
}

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

/// One dispatch a `map` block issues (grammar 8.6 rule 7).
///
/// The homogeneous form declares `input:`, `writes:` and `detach:` at map
/// level and the routed form declares them per route (Decision D85), so every
/// rule stated over a dispatch reads them from here rather than matching the
/// two forms again — and a key added to one form cannot be read in one place
/// and forgotten in another.
pub(crate) struct Dispatch<'a> {
    /// The dispatch target: `agent.*`, `tool.*`, or `flow.*`.
    pub(crate) target: &'a Address,
    /// The per-item binding, absent where the whole item is the input.
    pub(crate) input: Option<&'a NodeInput>,
    /// The write remap for this target.
    pub(crate) writes: Option<&'a Writes>,
    /// `detach:`, as written.
    pub(crate) detach: Option<&'a Spanned<bool>>,
}

impl Dispatch<'_> {
    /// Whether this dispatch is the one that wrote `detach: true`.
    pub(crate) fn is_detached(&self) -> bool {
        self.detach.is_some_and(|detach| detach.value)
    }
}

/// Every dispatch of a `map` block, in declaration order.
pub(crate) fn dispatches(dispatch: &MapDispatch) -> Vec<Dispatch<'_>> {
    match dispatch {
        MapDispatch::Homogeneous {
            node,
            input,
            writes,
            detach,
        } => vec![Dispatch {
            target: &node.value,
            input: input.as_ref(),
            writes: writes.as_ref(),
            detach: detach.as_ref(),
        }],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .map(|route| Dispatch {
                target: &route.node.value,
                input: route.input.as_ref(),
                writes: route.writes.as_ref(),
                detach: route.detach.as_ref(),
            })
            .collect(),
    }
}

/// Every dispatch target of a `map` block, in declaration order.
pub(crate) fn targets(dispatch: &MapDispatch) -> Vec<&Address> {
    dispatches(dispatch)
        .into_iter()
        .map(|dispatch| dispatch.target)
        .collect()
}

/// One flow instance that runs inside a **detached** dispatch (grammar 8.6
/// rule 7, Decision D94).
#[derive(Clone)]
pub(crate) struct Detached<'a> {
    /// The instance's flow address.
    pub(crate) address: String,
    /// Its definition.
    pub(crate) flow: &'a Flow,
    /// How the *detaching* dispatch is named in a diagnostic — the map that
    /// wrote `detach: true`, however many instances out from the write it is.
    pub(crate) dispatcher: String,
    /// Where `detach: true` was written.
    pub(crate) detach: Spanned<bool>,
}

/// Every flow instance a detached dispatch reaches, by the flow's address
/// (grammar 8.6 rule 7).
///
/// A detached dispatch is resolved at dispatch, so the join is over before the
/// instance has done anything at all — and that is as true of the instances it
/// goes on to dispatch as of its own nodes. The walk therefore follows both
/// positions a flow instance is entered from inside another flow: a `flow:`
/// node and a `map` dispatch. One flow reached by two detached dispatches is
/// recorded twice, because each dispatch is its own mistake to fix.
pub(crate) fn detached_instances(ir: &Ir) -> BTreeMap<String, Vec<Detached<'_>>> {
    let mut found: BTreeMap<String, Vec<Detached<'_>>> = BTreeMap::new();
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        for node in &flow.nodes {
            let NodeKind::Map { map } = &node.kind else {
                continue;
            };
            let dispatcher = format!("the map `{}` of `{address}`", node.id.value);
            for dispatch in dispatches(&map.dispatch) {
                let Some(detach) = dispatch.detach.filter(|detach| detach.value) else {
                    continue;
                };
                // An `agent.*` or `tool.*` target has no nodes of its own, so
                // nothing it does is reached from here; what such a dispatch
                // writes is the dispatch site's own write map, checked there.
                if dispatch.target.namespace != Namespace::Flow {
                    continue;
                }
                let mut seen = BTreeSet::new();
                walk_detached(
                    ir,
                    &dispatch.target.to_string(),
                    &dispatcher,
                    detach,
                    &mut seen,
                    &mut found,
                );
            }
        }
    }
    found
}

fn walk_detached<'a>(
    ir: &'a Ir,
    address: &str,
    dispatcher: &str,
    detach: &Spanned<bool>,
    seen: &mut BTreeSet<String>,
    found: &mut BTreeMap<String, Vec<Detached<'a>>>,
) {
    // One record per instance per detaching dispatch: a flow this dispatch
    // reaches twice is one detached instance, and the guard also keeps a
    // composition the recursion check has yet to refuse from looping here.
    if !seen.insert(address.to_string()) {
        return;
    }
    let Some(flow) = flow_at(ir, address) else {
        return;
    };
    found
        .entry(address.to_string())
        .or_default()
        .push(Detached {
            address: address.to_string(),
            flow,
            dispatcher: dispatcher.to_string(),
            detach: detach.clone(),
        });
    for node in &flow.nodes {
        let reached: Vec<String> = match &node.kind {
            NodeKind::Flow { flow, .. } => vec![flow.value.to_string()],
            // A dispatch that detached itself is the seed of its own walk, and
            // what it reaches is reported against *it* — the nearer of the two
            // `detach: true`s the author would go and read. Following it from
            // out here would record the same instance a second time and say
            // one write twice.
            NodeKind::Map { map } => dispatches(&map.dispatch)
                .into_iter()
                .filter(|dispatch| {
                    !dispatch.is_detached() && dispatch.target.namespace == Namespace::Flow
                })
                .map(|dispatch| dispatch.target.to_string())
                .collect(),
            _ => Vec::new(),
        };
        for address in reached {
            walk_detached(ir, &address, dispatcher, detach, seen, found);
        }
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
    /// The flow whose `map` opened this site — the instance this dispatch is
    /// issued *from*.
    pub(crate) owner: &'a str,
    /// Whether the dispatch that opened this site wrote `detach: true`, carried
    /// inward through `flow:` nodes: a `flow:` node inside a detached instance
    /// is dispatched no less fire-and-forget than its caller. What such an
    /// instance writes is refused by rule 7 over [`detached_instances`], so the
    /// rules stated over frames read this only to stay silent about a write
    /// that rule has already spoken about (grammar 8.6 rule 7).
    pub(crate) detached: bool,
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
            for dispatch in dispatches(&map.dispatch) {
                if dispatch.target.namespace != Namespace::Flow {
                    continue;
                }
                let dispatched_at = dispatch.target.to_string();
                let Some(dispatched) = ctx.flow_named(&dispatched_at) else {
                    continue;
                };
                let derived = seed(dispatched, dispatch.input, item);
                let mut path = BTreeSet::new();
                push_frame(
                    ctx,
                    &mut frames,
                    &mut path,
                    Frame {
                        address: dispatched_at,
                        flow: dispatched,
                        derived,
                        dispatcher: dispatcher.clone(),
                        span: node.span.clone(),
                        owner: address,
                        detached: dispatch.is_detached(),
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
            && recorded.detached == frame.detached
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
    let owner = frame.owner;
    let detached = frame.detached;
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
                owner,
                detached,
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
