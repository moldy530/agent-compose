//! Two traversals the schema checks need, and one place each is defined.
//!
//! # Component reachability (grammar 7.7)
//!
//! A flow **reaches** the components its own nodes name, its maps' dispatch
//! targets, the `human` nodes among them, the stores and tools of every agent it
//! reaches, and everything the flows it reaches reach in turn. One relation
//! serves three checks and each reads a different part of what [`reached`]
//! returns: session coherence reads the stores ([`stores_of`], grammar 11.3),
//! interrupt-freedom reads the `human` nodes (grammar 13.3), and recursion reads
//! the flows — which it needs as *edges* with their invocation sites rather than
//! as a set, so it walks [`calls`] instead (grammar 7.5). Stating the traversal
//! once here is what keeps the three from drifting apart (Decision D86).
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
//! Detachment needs no walk of its own. A dispatched flow instance holds its
//! own channel values (grammar 10.1), so nothing a node inside it writes
//! reaches the state of the flow that dispatched it; what crosses is the
//! dispatch site's effective write map, and both rules stated over that — 8.6
//! rule 5 and rule 7 — are decided at the site, in [`maps`](super::maps).
//! A store write is the one effect that *is* shared across instances, which is
//! why derivation, and only derivation, is traced inward from here.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::{Address, Namespace};
use crate::cel::Scope;
use crate::diag::{Span, Spanned};
use crate::ir::Ir;
use crate::ir::binding::NodeInput;
use crate::ir::definition::{Agent, DefinitionBody};
use crate::ir::flow::{Flow, MapDispatch, NodeKind};

use super::Ctx;

/// The flow definition at this address.
pub(crate) fn flow_at<'a>(ir: &'a Ir, address: &str) -> Option<&'a Flow> {
    match ir.definitions.get(address).map(|found| &found.body) {
        Some(DefinitionBody::Flow(flow)) => Some(flow),
        _ => None,
    }
}

/// What one flow **reaches** (grammar 7.7).
///
/// Three checks quantify over this one relation and each reads a different part
/// of it: session coherence reads [`stores`](Self::stores) (grammar 11.3),
/// sync-trigger interrupt-freedom reads [`humans`](Self::humans) (grammar 13.3),
/// and recursion reads the flows — which it needs with their invocation sites
/// rather than as a set, so it walks [`calls`] instead (grammar 7.5).
#[derive(Debug, Default)]
pub(crate) struct Reached {
    /// The `store.*` addresses.
    pub(crate) stores: BTreeSet<String>,
    /// The `human` nodes, by the flow that declares them and their flow-local
    /// id, each with the id's span. A map rather than a list so the answer does
    /// not depend on the order the walk happened to take.
    pub(crate) humans: BTreeMap<(String, String), Span>,
}

/// Everything a flow reaches (grammar 7.7).
pub(crate) fn reached(ctx: &Ctx, flow: &str) -> Reached {
    let mut found = Reached::default();
    let mut seen = BTreeSet::new();
    walk(ctx, flow, &mut seen, &mut found);
    found
}

/// The store addresses a flow reaches (grammar 7.7, 11.3).
pub(crate) fn stores_of(ctx: &Ctx, flow: &str) -> BTreeSet<String> {
    reached(ctx, flow).stores
}

fn walk(ctx: &Ctx, address: &str, seen: &mut BTreeSet<String>, found: &mut Reached) {
    if !seen.insert(address.to_string()) {
        return;
    }
    let Some(flow) = ctx.flow_named(address) else {
        return;
    };
    for node in &flow.nodes {
        match &node.kind {
            NodeKind::Store { store, .. } => {
                found.stores.insert(store.value.to_string());
            }
            // Clause 1 names the `human` node itself, which is the one thing a
            // flow reaches that is not a typed address.
            NodeKind::Human { .. } => {
                found.humans.insert(
                    (address.to_string(), node.id.value.to_string()),
                    node.id.span.clone(),
                );
            }
            NodeKind::Agent { agent } => agent_reaches(ctx, &agent.value.to_string(), seen, found),
            NodeKind::Flow { flow, .. } => {
                walk(ctx, &flow.value.to_string(), seen, found);
            }
            NodeKind::Map { map } => {
                for target in targets(&map.dispatch) {
                    match target.value.namespace {
                        Namespace::Agent => {
                            agent_reaches(ctx, &target.value.to_string(), seen, found);
                        }
                        Namespace::Flow => walk(ctx, &target.value.to_string(), seen, found),
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
fn agent_reaches(ctx: &Ctx, address: &str, seen: &mut BTreeSet<String>, found: &mut Reached) {
    let Some(agent) = agent_at(ctx, address) else {
        return;
    };
    for store in &agent.stores {
        found.stores.insert(store.value.to_string());
    }
    for tool in &agent.tools {
        if tool.value.namespace == Namespace::Flow {
            walk(ctx, &tool.value.to_string(), seen, found);
        }
    }
}

/// The agent definition at this address.
fn agent_at<'a>(ctx: &Ctx<'a>, address: &str) -> Option<&'a Agent> {
    match ctx.ir.definitions.get(address).map(|found| &found.body) {
        Some(DefinitionBody::Agent(agent)) => Some(agent),
        _ => None,
    }
}

/// One flow invoking another, with the construct that invokes it (grammar 7.7).
pub(crate) struct Call {
    /// The `flow.*` address invoked.
    pub(crate) target: String,
    /// Where the invocation is written.
    pub(crate) span: Span,
    /// The verb a diagnostic uses for it.
    pub(crate) verb: &'static str,
}

/// The flows one flow invokes **directly** (grammar 7.7 clauses 1, 2, and 4).
///
/// Transitivity — clause 5 — is the caller's, because recursion is a property of
/// the whole invocation graph rather than of one walk from one flow: the same
/// edges are read once and every cycle in them is found together
/// ([`components`](super::components)).
pub(crate) fn calls(ctx: &Ctx, address: &str) -> Vec<Call> {
    let mut calls = Vec::new();
    let Some(flow) = ctx.flow_named(address) else {
        return calls;
    };
    let from_agent = |calls: &mut Vec<Call>, agent: &str| {
        let Some(agent) = agent_at(ctx, agent) else {
            return;
        };
        for tool in &agent.tools {
            if tool.value.namespace == Namespace::Flow {
                calls.push(Call {
                    target: tool.value.to_string(),
                    span: tool.span.clone(),
                    verb: "attaches",
                });
            }
        }
    };
    for node in &flow.nodes {
        match &node.kind {
            NodeKind::Flow { flow, .. } => calls.push(Call {
                target: flow.value.to_string(),
                span: flow.span.clone(),
                verb: "instantiates",
            }),
            NodeKind::Agent { agent } => from_agent(&mut calls, &agent.value.to_string()),
            NodeKind::Map { map } => {
                for target in targets(&map.dispatch) {
                    match target.value.namespace {
                        Namespace::Flow => calls.push(Call {
                            target: target.value.to_string(),
                            span: target.span.clone(),
                            verb: "dispatches",
                        }),
                        Namespace::Agent => from_agent(&mut calls, &target.value.to_string()),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    calls
}

/// One dispatch a `map` block issues (grammar 8.6 rule 7).
///
/// The homogeneous form declares `input:` at map level and the routed form
/// declares it per route (Decision D85), so a rule stated over a dispatch reads
/// it from here rather than matching the two forms again — and a key added to
/// one form cannot be read in one place and forgotten in another. The
/// per-dispatch keys the *site* rules need instead — `writes:` and `detach:` —
/// are read where those rules run, beside the route's own narrowing
/// ([`maps`](super::maps)), which the flattened view cannot carry.
pub(crate) struct Dispatch<'a> {
    /// The dispatch target: `agent.*`, `tool.*`, or `flow.*`.
    pub(crate) target: &'a Address,
    /// The per-item binding, absent where the whole item is the input.
    pub(crate) input: Option<&'a NodeInput>,
}

/// Every dispatch of a `map` block, in declaration order.
pub(crate) fn dispatches(dispatch: &MapDispatch) -> Vec<Dispatch<'_>> {
    match dispatch {
        MapDispatch::Homogeneous { node, input, .. } => vec![Dispatch {
            target: &node.value,
            input: input.as_ref(),
        }],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .map(|route| Dispatch {
                target: &route.node.value,
                input: route.input.as_ref(),
            })
            .collect(),
    }
}

/// Every dispatch target of a `map` block, in declaration order, with the span
/// of the reference itself — which is what a diagnostic about the *invocation*
/// points at, as against one about the dispatch's bindings.
pub(crate) fn targets(dispatch: &MapDispatch) -> Vec<&Spanned<Address>> {
    match dispatch {
        MapDispatch::Homogeneous { node, .. } => vec![node],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .map(|route| &route.node)
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
    /// Where each of those fields was bound, for the fields a binding decided:
    /// the `input:` entry at the dispatch, or at the `flow:` node that carried
    /// derivation inward. A dispatch that passes the whole item, and a field
    /// nothing binds, leave no entry — there is no binding to point at.
    ///
    /// This is what lets a diagnostic name the edit its reader has to make: the
    /// store node it reports is innocent, and so is the map, so the binding
    /// that flipped a field to *not derived* is the third site grammar 11.4's
    /// worked example is about ("reached through a binding that merely *looks*
    /// item-derived at the store node").
    pub(crate) bound_at: BTreeMap<String, Span>,
    /// How the dispatching map is named in a diagnostic.
    pub(crate) dispatcher: String,
    /// The dispatching map node's span.
    pub(crate) span: Span,
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
                let (derived, bound_at) = seed(dispatched, dispatch.input, item);
                let mut path = BTreeSet::new();
                push_frame(
                    ctx,
                    &mut frames,
                    &mut path,
                    Frame {
                        address: dispatched_at,
                        flow: dispatched,
                        derived,
                        bound_at,
                        dispatcher: dispatcher.clone(),
                        span: node.span.clone(),
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
/// same map with the same derivation — is dropped: it is the same *site* said
/// twice, and every rule stated over frames would otherwise report one mistake
/// once per repetition. The dedup reads the derivation itself and not
/// [`Frame::bound_at`], which only says where the same answer was written.
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
        let mut bound_at = BTreeMap::new();
        if let Some(inputs) = &nested.inputs {
            for field in &inputs.fields {
                let name = field.name.value.to_string();
                let binding = match &node.input {
                    Some(NodeInput::Fields { bindings }) => bindings
                        .entries
                        .iter()
                        .find(|binding| binding.name.value == name),
                    _ => None,
                };
                let value = binding.is_some_and(|binding| {
                    is_item_derived(binding.value.value.as_str(), None, &derived)
                });
                if let Some(binding) = binding {
                    bound_at.insert(name.clone(), binding.value.span.clone());
                }
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
                bound_at,
                dispatcher: dispatcher.clone(),
                span: span.clone(),
            },
        );
    }
    path.remove(&address);
}

/// Which input fields of a dispatched flow are item-derived at one site, and
/// where each answer was written (Decision D83).
fn seed(
    flow: &Flow,
    input: Option<&NodeInput>,
    item: &str,
) -> (BTreeMap<String, bool>, BTreeMap<String, Span>) {
    let mut derived = BTreeMap::new();
    let mut bound_at = BTreeMap::new();
    let Some(inputs) = &flow.inputs else {
        return (derived, bound_at);
    };
    for field in &inputs.fields {
        let name = field.name.value.to_string();
        let value = match input {
            // The whole item is the instance's input, so every field is
            // item-derived — and no binding was written to point at.
            None => true,
            Some(NodeInput::Fields { bindings }) => {
                let binding = bindings
                    .entries
                    .iter()
                    .find(|binding| binding.name.value == name);
                if let Some(binding) = binding {
                    bound_at.insert(name.clone(), binding.value.span.clone());
                }
                binding.is_some_and(|binding| {
                    is_item_derived(binding.value.value.as_str(), Some(item), &BTreeMap::new())
                })
            }
            // The scalar form binds a string-in agent, and an agent contains no
            // store nodes (grammar 11.4).
            Some(NodeInput::Scalar { .. }) => false,
        };
        derived.insert(name, value);
    }
    (derived, bound_at)
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
