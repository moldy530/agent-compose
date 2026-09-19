//! Two traversals the schema checks need, and one place each is defined.
//!
//! # Component reachability (grammar 7.7)
//!
//! A flow **reaches** the components its own nodes name, its maps' dispatch
//! targets, the `human` nodes among them, the stores and tools of every agent it
//! reaches, and everything the flows it reaches reach in turn. One relation
//! serves five checks and each reads a different part of what [`reached`]
//! returns: session coherence reads the stores ([`stores_of`], grammar 11.3),
//! sync-trigger interrupt-freedom reads the `human` nodes (grammar 13.3),
//! detached-dispatch interrupt-freedom reads them from one dispatch target
//! ([`reached_by`], grammar 8.6 rule 7), placement colocation reads the
//! placeable components — clauses 1 and 2, the `agent.*` and `tool.*` a flow's
//! own nodes and maps name (grammar 14.1) — and recursion reads the flows, which
//! it needs as *edges* with their invocation sites rather than as a set, so it
//! walks [`calls`] instead (grammar 7.5). Stating the traversal once here is
//! what keeps the five from drifting apart (Decision D86).
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
//! **Derivation** is seeded from nothing outside the map block itself — a
//! nested map re-roots the computation at its own item — so which of a
//! dispatched flow's fields are item-derived is the same wherever the map's own
//! flow is instantiated. What propagates is the inside of a dispatched flow: a
//! `flow:` node there carries derivation inward exactly when its binding
//! expression is item-derived in the frame it sits in.
//!
//! **The bound is not seeded that way**, and it is the one part of a site that
//! is not a property of the map block: a map runs once per dispatch of the
//! fan-out it is *inside*, so `max_concurrency: 1` on a map that an outer map
//! fans four ways is four of its dispatches in flight at once and not one
//! (Decision D147, PRD resolved q61 ruling b). So the walk descends through
//! `map` nodes as well as `flow:` nodes, and a nested map's frames are recorded
//! at the **looser** of its own bound and the one carried in.
//!
//! **Nor is what those concurrent instances tell apart**, which is the same
//! observation about derivation and travels beside the bound rather than instead
//! of it ([`Frame::per_instance`]). Item-derivation above is one map's
//! dispatches against each other, which is all §11.4 asks of a store key; a rule
//! about *concurrency* also has to ask whether two instances of the map can
//! reach one value, and an item drawn from a list says nothing about that. So a
//! nested map's frames carry both answers, and the rules that are about
//! concurrency read both (Decision D147).
//!
//! Detachment needs no walk of its own. A dispatched flow instance holds its
//! own channel values (grammar 10.1), so nothing a node inside it writes
//! reaches the state of the flow that dispatched it; what crosses is the
//! dispatch site's effective write map, and both rules stated over that — 8.6
//! rule 5 and rule 7 — are decided at the site, in [`maps`](super::maps).
//! A store write is the one effect that *is* shared across instances, which is
//! why derivation, and only derivation, is traced inward from here.
//!
//! # Both traversals carry their own stack
//!
//! [`walk`] and [`push_frame`] descend through nested compositions, and a
//! composition's flow count is bounded by nothing this compiler controls — the
//! same reason [`Graph::sccs`](super::graph) is written iteratively. Written with
//! the call stack instead, a composition nested deeply enough aborts the process
//! on a stack overflow: no diagnostic, no report, and an exit code no consumer of
//! `validate` has a meaning for. Both therefore keep their pending work on the
//! heap. [`push_frame`]'s stack carries an explicit `Leave` marker, because its
//! recursion had an *after* — the path set a frame is removed from once its
//! nested flows are done with it.

use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use crate::ast::common::{Address, Namespace};
use crate::cel::Scope;
use crate::diag::{Span, Spanned};
use crate::ir::Ir;
use crate::ir::binding::NodeInput;
use crate::ir::definition::{Agent, DefinitionBody};
use crate::ir::flow::{Coder, Flow, Map, MapDispatch, Node, NodeKind};

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
/// Five checks quantify over this one relation and each reads a different part
/// of it: session coherence reads [`stores`](Self::stores) (grammar 11.3), the
/// two interrupt-freedom rules read [`humans`](Self::humans) — one over a
/// `respond: sync` trigger's flow (grammar 13.3) and one over a **detached**
/// dispatch's target (grammar 8.6 rule 7) — placement colocation reads
/// [`placeable`](Self::placeable) (grammar 14.1), and recursion reads the flows,
/// which it needs with their invocation sites rather than as a set, so it walks
/// [`calls`] instead (grammar 7.5).
#[derive(Debug, Default)]
pub(crate) struct Reached {
    /// The `store.*` addresses.
    pub(crate) stores: BTreeSet<String>,
    /// The `human` nodes, by the flow that declares them and their flow-local
    /// id, each with the id's span. A map rather than a list so the answer does
    /// not depend on the order the walk happened to take.
    pub(crate) humans: BTreeMap<(String, String), Span>,
    /// The `agent.*` and `tool.*` addresses a walked flow's own nodes and maps
    /// name — clauses 1 and 2, which are exactly the components a placement may
    /// hold (grammar 14.1).
    ///
    /// Deliberately **not** an agent's attached `tool.*`: clauses 3 and 4 stop
    /// at an agent's stores and its `flow.*` tools, and the one check that reads
    /// this wants that boundary. An attached tool already colocates with its
    /// agent by a rule of its own, so collecting it here would report one
    /// contradiction twice and point the second report at the wrong line.
    ///
    /// Each address carries the site that names it, earliest first by source and
    /// offset, because the walk pops its worklist in an order nothing fixes and
    /// a diagnostic's label may not depend on it.
    pub(crate) placeable: BTreeMap<String, Span>,
}

/// Record a placeable component at the site naming it, earliest site winning.
///
/// "Earliest" is by source name and byte offset, which is a total order over
/// spans and is the only thing available: [`Span`] is not `Ord`, and the walk's
/// own order is not a property of the composition.
fn record(found: &mut Reached, address: &Spanned<Address>) {
    let key = address.value.to_string();
    let position = |span: &Span| (span.source.as_str().to_string(), span.bytes.start);
    match found.placeable.get(&key) {
        Some(held) if position(held) <= position(&address.span) => {}
        _ => {
            found.placeable.insert(key, address.span.clone());
        }
    }
}

/// Everything a flow reaches (grammar 7.7).
///
/// Takes the artifact rather than a check context, because the relation is a
/// property of the composition and two of its readers are not the validator:
/// `codegen::graph` asks which `scope: session` stores a flow reaches so a run
/// can be refused at start without one (grammar 11.3), and a check asks the same
/// question of the same walk.
pub(crate) fn reached(ir: &Ir, flow: &str) -> Reached {
    let mut found = Reached::default();
    let mut seen = BTreeSet::new();
    walk(ir, flow, &mut seen, &mut found);
    found
}

/// The store addresses a flow reaches (grammar 7.7, 11.3).
pub(crate) fn stores_of(ir: &Ir, flow: &str) -> BTreeSet<String> {
    reached(ir, flow).stores
}

/// Everything one `map` dispatch **target** reaches (grammar 7.7 clause 2).
///
/// A target is an address rather than a flow, and the three namespaces §8.6
/// admits reach different amounts: a `flow.*` reaches what the flow reaches, an
/// `agent.*` reaches its stores and its `flow.*` tools (clauses 3 and 4), and a
/// `tool.*` is a request or a process and reaches nothing this relation is
/// about. Stated here rather than at the one check that asks, so a dispatch
/// target's reachability is [`walk`]'s answer wherever it is asked for.
pub(crate) fn reached_by(ir: &Ir, target: &Address) -> Reached {
    let mut found = Reached::default();
    let mut seen = BTreeSet::new();
    match target.namespace {
        Namespace::Flow => walk(ir, &target.to_string(), &mut seen, &mut found),
        Namespace::Agent => {
            let mut pending = Vec::new();
            agent_reaches(ir, &target.to_string(), &mut pending, &mut found);
            for address in pending {
                walk(ir, &address, &mut seen, &mut found);
            }
        }
        _ => {}
    }
    found
}

/// The traversal itself: a worklist of flow addresses, each visited once.
///
/// What it collects is order-independent — a set of stores and a map of `human`
/// nodes — so the order the pending addresses come off the list is not
/// observable, and `seen` is read on the way *out* of the list rather than on the
/// way in: an address may be queued twice and is walked once.
fn walk(ir: &Ir, address: &str, seen: &mut BTreeSet<String>, found: &mut Reached) {
    let mut pending: Vec<String> = vec![address.to_string()];
    while let Some(address) = pending.pop() {
        if !seen.insert(address.clone()) {
            continue;
        }
        let Some(flow) = flow_at(ir, &address) else {
            continue;
        };
        for node in &flow.nodes {
            match &node.kind {
                NodeKind::Store { store, .. } => {
                    found.stores.insert(store.value.to_string());
                }
                // Clause 1 names the `human` node itself, which is the one thing
                // a flow reaches that is not a typed address.
                NodeKind::Human { .. } => {
                    found.humans.insert(
                        (address.clone(), node.id.value.to_string()),
                        node.id.span.clone(),
                    );
                }
                NodeKind::Agent { agent } => {
                    record(found, agent);
                    agent_reaches(ir, &agent.value.to_string(), &mut pending, found);
                }
                // A `function:` node names a `tool.*` directly (grammar 8.4).
                // It reaches nothing further — a tool is a process or a request
                // — so clause 1 is the whole of what it contributes.
                NodeKind::Function { function } => record(found, function),
                NodeKind::Flow { flow, .. } => pending.push(flow.value.to_string()),
                NodeKind::Map { map } => {
                    for target in targets(&map.dispatch) {
                        match target.value.namespace {
                            Namespace::Agent => {
                                record(found, target);
                                agent_reaches(ir, &target.value.to_string(), &mut pending, found);
                            }
                            Namespace::Tool => record(found, target),
                            Namespace::Flow => pending.push(target.value.to_string()),
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

/// An agent reaches the stores it attaches and everything its `flow.*` tools
/// reach — flow-as-tool attachment is a call (grammar 7.7 clauses 3, 4). The
/// flows go on the caller's worklist rather than down a second stack.
fn agent_reaches(ir: &Ir, address: &str, pending: &mut Vec<String>, found: &mut Reached) {
    let Some(agent) = agent_at(ir, address) else {
        return;
    };
    for store in &agent.stores {
        found.stores.insert(store.value.to_string());
    }
    for tool in &agent.tools {
        if tool.value.namespace == Namespace::Flow {
            pending.push(tool.value.to_string());
        }
    }
}

/// The agent definition at this address.
fn agent_at<'a>(ir: &'a Ir, address: &str) -> Option<&'a Agent> {
    match ir.definitions.get(address).map(|found| &found.body) {
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
        let Some(agent) = agent_at(ctx.ir, agent) else {
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
    /// `max_concurrency:` as this dispatch really has it: the route's where the
    /// route tightens it, the map's otherwise (grammar 8.6 rule 1).
    ///
    /// The **effective** value rather than the map's, because the rule that
    /// reads it is about how many of this dispatch can be in flight at once
    /// (PRD resolved q61 ruling b) — and a route may only tighten.
    pub(crate) concurrency: i64,
}

/// Every dispatch of a `map` block, in declaration order.
pub(crate) fn dispatches(map: &Map) -> Vec<Dispatch<'_>> {
    match &map.dispatch {
        MapDispatch::Homogeneous { node, input, .. } => vec![Dispatch {
            target: &node.value,
            input: input.as_ref(),
            concurrency: map.max_concurrency,
        }],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .map(|route| Dispatch {
                target: &route.node.value,
                input: route.input.as_ref(),
                concurrency: route.max_concurrency.unwrap_or(map.max_concurrency),
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

/// One `coder:` node a flow contains, with the flow that declares it.
#[derive(Clone)]
pub(crate) struct Contained<'a> {
    /// The address of the flow the node is declared in — this flow, or one
    /// below it.
    pub(crate) address: String,
    /// The node itself.
    pub(crate) node: &'a Node,
    /// Its `coder:` block.
    pub(crate) coder: &'a Coder,
}

/// Every `coder:` node one flow **contains**: its own, and those of the flows
/// its `flow:` nodes instantiate and its `map` nodes dispatch, transitively.
///
/// This is the containment a *step* of the enclosing graph has. A `flow:` node
/// and a `map` node are each one node of their flow's graph (grammar 7.6.1) and
/// every harness run inside an instance they start happens while that node is in
/// flight, which is what the concurrent half of PRD resolved q61 ruling b is
/// stated over: two harness runs that can be in flight at once.
///
/// **A `map` is walked, and it has to be.** [`frames`] carries the per-dispatch
/// scope and decides the *refusal*, but it decides it only where a fan-out has
/// two dispatches in flight — [`dispatched_workspaces`](super::coder) skips
/// every frame bounded at 1 — and `max_concurrency: 1` is the repair that
/// refusal's own help offers. A serial map beside a sibling branch is two
/// harness runs in one directory with nothing serialising them: the map's bound
/// holds *within* the map and says nothing about the branch next to it. Reading
/// the boundary the other way let the recommended repair silence the collision
/// it does not fix.
///
/// **The boundary that stays is the agent's `flow.*` tool**, because a model
/// decides whether and when to call one and no static rule can put that call
/// beside another node — which is the boundary [`frames`] draws too, drawn here
/// again rather than differently.
///
/// **Answered out of a memo the caller carries across flows** ([`Coders`]),
/// because the relation is transitive and the rule reading it asks it of steps
/// all the way down a composition: a chain of *n* nested flows walked once per
/// link is quadratic in a nesting depth nothing in this compiler bounds
/// (`tests/check_scale.rs`), and `validate` is held to a millisecond budget. The
/// answer is a property of the composition, so one walk per address is one too
/// many and never one too few. The memo is a parameter rather than a cell on the
/// check context because a `RefCell` there would make [`Ctx`] invariant over its
/// artifact's lifetime, which is a cost every other check would pay for this one.
pub(crate) fn coders_within<'a>(
    ctx: &Ctx<'a>,
    memo: &mut Coders<'a>,
    address: &str,
) -> Rc<Vec<Contained<'a>>> {
    if let Some(held) = memo.get(address) {
        return Rc::clone(held);
    }
    let mut found = Vec::new();
    let mut seen = BTreeSet::new();
    let mut pending = vec![address.to_string()];
    while let Some(address) = pending.pop() {
        if !seen.insert(address.clone()) {
            continue;
        }
        let Some(flow) = ctx.flow_named(&address) else {
            continue;
        };
        let mut nested = Vec::new();
        for node in &flow.nodes {
            match &node.kind {
                NodeKind::Coder { coder } => found.push(Contained {
                    address: address.clone(),
                    node,
                    coder,
                }),
                NodeKind::Flow { flow, .. } => nested.push(flow.value.to_string()),
                // A dispatch target that is not a `flow.*` holds no coder node:
                // an agent has none, and neither does a tool (grammar 8.9).
                NodeKind::Map { map } => nested.extend(
                    targets(&map.dispatch)
                        .into_iter()
                        .filter(|target| target.value.namespace == Namespace::Flow)
                        .map(|target| target.value.to_string()),
                ),
                _ => {}
            }
        }
        // Reversed onto the stack, so the walk is the depth-first,
        // declaration-order one a recursive one would make and the order a
        // diagnostic picks a run out of is a property of the composition.
        nested.reverse();
        pending.extend(nested);
    }
    let found = Rc::new(found);
    memo.insert(address.to_string(), Rc::clone(&found));
    found
}

/// [`coders_within`]'s answers, held across the flows one pass walks.
pub(crate) type Coders<'a> = BTreeMap<String, Rc<Vec<Contained<'a>>>>;

/// The fan-out a dispatch is itself inside, where that fan-out runs more than
/// one instance at a time (Decision D147, PRD resolved q61 ruling b).
///
/// Carried so a diagnostic about the inner dispatch can name the bound it is
/// really read at, and the map that states it: a message quoting
/// `max_concurrency: 1` off the inner map while refusing it over four
/// concurrent runs would be pointing at the line the author already wrote
/// correctly (PRD G3).
///
/// Recorded whenever the enclosing fan-out is concurrent, and **not** only where
/// it is looser than the dispatch's own bound. Two things are asked of it and
/// they part at exactly that case: how many runs the directory holds, which the
/// looser of the two answers ([`Frame::concurrency`]), and whether a value told
/// apart per dispatch is told apart *between* those instances, which the
/// enclosing bound answers however tight or loose it is
/// ([`Frame::per_instance`]). A map fanning four ways inside a flow another map
/// fans four ways raises nothing, and its dispatches still overlap across four
/// concurrent instances of it.
#[derive(Clone)]
pub(crate) struct Enclosing {
    /// How the enclosing map is named in a diagnostic.
    pub(crate) dispatcher: String,
    /// Its **effective** bound, which is the one the dispatch inside it is read
    /// at.
    pub(crate) concurrency: i64,
    /// The enclosing map node's span.
    pub(crate) span: Span,
}

/// One flow instance running inside a fan-out.
pub(crate) struct Frame<'a> {
    /// The dispatched flow's address.
    pub(crate) address: String,
    /// Its definition.
    pub(crate) flow: &'a Flow,
    /// Which of this instance's input fields are item-derived at this site
    /// (Decision D83).
    ///
    /// **The dispatching map's own item, and nothing outside it.** A nested map
    /// re-roots this at the item it declares, which is what grammar 11.4 asks of
    /// a store key: a key derived from the item the write belongs to is what
    /// keeps N items' content out of one slot, whatever fan-out the map itself
    /// runs inside. What that leaves unanswered — whether the value differs
    /// between two *instances* of the map — is [`Frame::per_instance`]'s
    /// question, asked by the rules that are about concurrency.
    pub(crate) derived: BTreeMap<String, bool>,
    /// Which of this instance's input fields take a distinct value across the
    /// instances the **enclosing** fan-out runs, as against across the dispatches
    /// of the map that issues this one ([`Frame::derived`]).
    ///
    /// The two are different questions and a nested map answers them
    /// differently, which is the whole of why both are carried. `input: { file:
    /// "f" }` off a map's own item is per-dispatch and says nothing at all about
    /// the instance around it: four instances of the enclosing flow each stepping
    /// their own files serially still put two runs in one directory the moment
    /// two of them draw the same file name, and `src/main.rs` under two
    /// repositories is that. A binding reading an `input.<field>` the enclosing
    /// instance itself varies is what separates them, and it is the shape a
    /// nested dispatch's repair takes: `input: { dir: "input.root + '/' + f" }`
    /// (Decision D147, PRD resolved q61 ruling b).
    ///
    /// Every field is `true` where nothing encloses the dispatching map, and
    /// that is the answer rather than an absence of one: with a single instance
    /// there is nothing for a value to be distinct *between*, so no field can
    /// fail to be. It is what lets [`push_frame`] merge two routes onto one site
    /// by conjunction — the route that reaches a map from outside a fan-out
    /// contributes nothing, and the one that reaches it from inside contributes
    /// what it really found.
    pub(crate) per_instance: BTreeMap<String, bool>,
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
    /// How many harness runs of this instance may be in flight at once, carried
    /// inward unchanged through the `flow:` nodes below it: an instance nested
    /// inside a dispatched one runs once per dispatch of the outermost, so the
    /// bound that decides whether two of *it* overlap is the dispatch's
    /// (grammar 8.6 rule 1, PRD resolved q61 ruling b).
    ///
    /// **The effective bound rather than the map's own**, and the two differ two
    /// ways. Where one map reaches one flow from **several** routes, this is the
    /// loosest of their bounds rather than the first one walked: see
    /// [`push_frame`], which merges it. And where the dispatching map is itself
    /// inside a fan-out, it is the looser of that fan-out's bound and its own
    /// ([`Frame::enclosing`]) — a map declaring `max_concurrency: 1` inside a
    /// flow an outer map fans four ways issues four of its dispatches at once,
    /// one per concurrent instance, and a rule reading the inner 1 would accept
    /// the very race Decision D147 makes unwritable.
    pub(crate) concurrency: i64,
    /// What the dispatching map's own `max_concurrency:` says
    /// ([`Dispatch::concurrency`]) — the number a diagnostic quotes when it
    /// names that map's key, as against [`Frame::concurrency`], which is what a
    /// rule tests.
    pub(crate) declared: i64,
    /// The fan-out this dispatch is itself inside, where that fan-out runs more
    /// than one instance at a time ([`Enclosing`]).
    pub(crate) enclosing: Option<Enclosing>,
}

impl Frame<'_> {
    /// How many instances of the fan-out enclosing this dispatch can be in
    /// flight at once — 1 where nothing encloses it, which is where
    /// [`Frame::per_instance`] has nothing to decide.
    pub(crate) fn instances(&self) -> i64 {
        self.enclosing.as_ref().map_or(1, |outer| outer.concurrency)
    }

    /// Whether more than one instance of this dispatch can be in flight at
    /// once — the condition [`Frame::per_instance`] is a real question under.
    fn nested(&self) -> bool {
        self.instances() > 1
    }

    /// Which of this instance's input fields take a distinct value in **every
    /// pair of runs of it that can overlap** — the two questions above, read
    /// together.
    ///
    /// Two runs of this instance differ in which dispatch of its map they are,
    /// in which instance of the enclosing fan-out issued them, or in both — so a
    /// field separates every such pair only when it separates both, which is the
    /// conjunction below.
    ///
    /// It is read as a conjunction even where one of the two axes has a single
    /// value on it, and deliberately: [`Frame::declared`] is one route's bound
    /// and a site merged from several routes keeps the tightest of them, so
    /// "this map's dispatches are serial" is not a fact this frame holds.
    /// Answering the stricter question costs the caller nothing it is entitled
    /// to — a field the map's own item does not derive is one the author has a
    /// repair for either way — while answering the looser one off a bound that
    /// belongs to a sibling route would exempt a composition from a race
    /// refusal on a number written somewhere else (grammar 8.6 rule 1,
    /// Decision D28).
    ///
    /// This is what a `map` **nested in this instance** measures its own
    /// bindings against: the instances enclosing that map are the overlapping
    /// runs of this frame, and what tells those apart is what its dispatches can
    /// inherit (Decision D147, PRD resolved q61 ruling b).
    fn varies(&self) -> BTreeMap<String, bool> {
        self.derived
            .iter()
            .map(|(name, derived)| {
                let by_instance = self.per_instance.get(name).copied().unwrap_or(false);
                (name.clone(), *derived && by_instance)
            })
            .collect()
    }
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
            // Seeded at the map's own bound and nothing else, because this scan
            // has no idea what the map's flow is instantiated inside. Where that
            // is a fan-out, the walk below reaches the same map from the outer
            // dispatch and merges the looser bound — and what varies across that
            // fan-out's instances — onto these frames.
            for frame in seeded(ctx, address, node, map, None, None) {
                let mut path = BTreeSet::new();
                push_frame(ctx, &mut frames, &mut path, frame);
            }
        }
    }
    frames
}

/// The frames one `map` node seeds: one per `flow.*` dispatch it issues, at the
/// bound that dispatch really runs under.
///
/// Stated once and read twice — by [`frames`]'s scan over every definition, and
/// by [`push_frame`] where the walk reaches a map *inside* a dispatched
/// instance — so the two cannot seed a site differently. Item-derivation is the
/// map block's own either way (Decision D83); the bound is not, and neither is
/// what the enclosing fan-out varies, which is what `enclosing` and `within`
/// carry in.
///
/// `within` is the enclosing instance's [`Frame::varies`] — which of *its* input
/// fields separate every pair of its runs that can overlap — or `None` where
/// nothing encloses this map, in which case every field of the dispatch is
/// recorded as separating instances there are none of. A binding of this map's
/// that reads a varying field separates the instances too, and that is the only
/// way a dispatch inside a fan-out inherits the distinction: this map's own item
/// does not have it, because two instances drawing from two lists can draw one
/// and the same value (Decision D147, PRD resolved q61 ruling b).
fn seeded<'a>(
    ctx: &Ctx<'a>,
    address: &str,
    node: &Node,
    map: &Map,
    enclosing: Option<&Enclosing>,
    within: Option<&BTreeMap<String, bool>>,
) -> Vec<Frame<'a>> {
    let dispatcher = format!("the map `{}` of `{address}`", node.id.value);
    let item = map
        .item_binding
        .as_ref()
        .map_or("item", |binding| binding.value.as_str());
    // A fan-out of one is a fan-out nothing overlaps inside, so it is not one a
    // frame has to carry: the dispatch is read at its own bound and its bindings
    // have nothing to be distinct *between*.
    let outer = enclosing.filter(|outer| outer.concurrency > 1);
    let within = outer.and(within);
    let mut seeded = Vec::new();
    for dispatch in dispatches(map) {
        if dispatch.target.namespace != Namespace::Flow {
            continue;
        }
        let dispatched_at = dispatch.target.to_string();
        let Some(dispatched) = ctx.flow_named(&dispatched_at) else {
            continue;
        };
        let (derived, per_instance, bound_at) = seed(dispatched, dispatch.input, item, within);
        seeded.push(Frame {
            address: dispatched_at,
            flow: dispatched,
            derived,
            per_instance,
            bound_at,
            dispatcher: dispatcher.clone(),
            span: node.span.clone(),
            // The looser of the two: a map bounded more tightly than what it
            // sits inside still has one run per concurrent instance in flight.
            concurrency: outer.map_or(dispatch.concurrency, |outer| {
                outer.concurrency.max(dispatch.concurrency)
            }),
            declared: dispatch.concurrency,
            enclosing: outer.cloned(),
        });
    }
    seeded
}

/// One item of [`push_frame`]'s worklist: a frame to record, or a frame whose
/// nested flows are done with and whose address leaves the path.
enum Step<'a> {
    /// Record this frame and queue the `flow:` and `map` nodes inside it. Boxed
    /// so the worklist's element is the size of the marker beside it rather than
    /// the size of a frame.
    Enter(Box<Frame<'a>>),
    /// Every frame below this address has been recorded; drop it from the path.
    Leave(String),
}

/// Record a frame and follow the `flow:` nodes inside it, carrying derivation
/// inward through their bindings (Decision D83).
///
/// A frame that repeats one already recorded — the same flow, dispatched by the
/// same map with the same derivation — is dropped: it is the same *site* said
/// twice, and every rule stated over frames would otherwise report one mistake
/// once per repetition. The dedup reads the derivation itself and not
/// [`Frame::bound_at`], which only says where the same answer was written.
///
/// **[`Frame::concurrency`] is merged rather than deduplicated**, because it is
/// the one part of a frame that is not a property of the site: a routed map may
/// name one flow from two routes (grammar 8.6 rule 1), and a route that
/// tightens its own `max_concurrency:` says nothing whatever about its
/// sibling's. Collapsing the two frames onto the first route's bound would let
/// `max_concurrency: 1` on any one route silence [the shared-workspace
/// refusal](super::coder::dispatched_workspaces) for every other route onto the
/// same flow — a concurrent fan-out into one directory validating clean, which
/// is exactly what PRD resolved q61 ruling b makes unwritable. So a repeat
/// raises the recorded bound to the **loosest** of the two and, when it really
/// raised it, goes on to walk the nested flows again: those were recorded with
/// the tighter bound and carry it inward.
///
/// [`Frame::per_instance`] is merged the other way round and for the same
/// reason: a field one route cannot tell its enclosing instances apart by is a
/// field the rule cannot read as per-dispatch, whatever the sibling route
/// happens to bind it from. The recorded answer is therefore the **tightest** of
/// the two, and tightening it re-walks the nested flows exactly as raising the
/// bound does — a loosest bound with a tightest derivation is the worst case the
/// author has to answer for, and a route that really does carry the distinction
/// in is not what excuses the one that does not.
///
/// **A `map` node inside a dispatched instance is followed too**, and that is
/// the other half of the bound not being a property of the map block. A map runs
/// once per dispatch of the fan-out above it, so a map declaring
/// `max_concurrency: 1` inside a flow an outer map fans four ways has four of
/// its dispatches in flight at once — one per concurrent instance — and reading
/// only the inner 1 accepts the race Decision D147 exists to refuse. Such a
/// frame is seeded at the looser of the two bounds and carries the outer
/// fan-out in [`Frame::enclosing`] so a diagnostic can name it. The *site* it
/// produces is the one [`frames`]'s own scan produces for that map — same
/// address, same dispatcher, same derivation — so the dedup above merges the two
/// and only the bound differs.
///
/// The worklist is a stack and each frame's nested flows go onto it in reverse,
/// so the order frames are recorded in is the depth-first, declaration-order one
/// a recursive walk would produce — which is what makes *which* repetition the
/// dedup keeps a property of the composition rather than of this loop.
fn push_frame<'a>(
    ctx: &Ctx<'a>,
    frames: &mut Vec<Frame<'a>>,
    path: &mut BTreeSet<String>,
    frame: Frame<'a>,
) {
    let mut pending = vec![Step::Enter(Box::new(frame))];
    while let Some(step) = pending.pop() {
        let mut frame = match step {
            Step::Enter(frame) => *frame,
            Step::Leave(address) => {
                path.remove(&address);
                continue;
            }
        };
        let repeat = frames.iter().position(|recorded| {
            recorded.address == frame.address
                && recorded.dispatcher == frame.dispatcher
                && recorded.derived == frame.derived
        });
        if let Some(at) = repeat {
            let looser = frame.concurrency > frames[at].concurrency;
            // The enclosing fan-out is merged on **its own** bound and not on
            // the frame's, which are two different numbers the moment a map is
            // as loose as what it sits inside. A route reaching this map from
            // outside every fan-out carries none, and a merge that let that
            // silence stand would read a site dispatched inside a four-way
            // fan-out as a site nothing encloses — and with it drop the whole
            // per-instance question.
            let widened = frame.instances() > frames[at].instances();
            let tightened: BTreeMap<String, bool> = frames[at]
                .per_instance
                .iter()
                .map(|(name, held)| {
                    let value = *held && frame.per_instance.get(name).copied().unwrap_or(false);
                    (name.clone(), value)
                })
                .collect();
            // The same site with a bound no looser than the one already
            // recorded, no wider a fan-out around it, and nothing newly
            // indistinct is the same site said twice, and there is nothing left
            // to carry inward.
            if !looser && !widened && tightened == frames[at].per_instance {
                continue;
            }
            if looser {
                frames[at].concurrency = frame.concurrency;
            }
            if widened {
                // …and the fan-out travels with its own bound, or the
                // diagnostic would quote a number and name a map that does not
                // state it.
                frames[at].enclosing.clone_from(&frame.enclosing);
            }
            frames[at].per_instance.clone_from(&tightened);
            // The walk below carries the **merged** frame inward, not this
            // route's own: the nested flows were recorded against the answers
            // that have just changed.
            frame.concurrency = frames[at].concurrency;
            frame.enclosing.clone_from(&frames[at].enclosing);
            frame.per_instance = tightened;
        }
        // A flow that reaches itself is refused by the graph pass's recursion
        // check; guarding the path here keeps this traversal finite meanwhile.
        let address = frame.address.clone();
        if !path.insert(address.clone()) {
            continue;
        }
        let flow = frame.flow;
        let derived = frame.derived.clone();
        let per_instance = frame.per_instance.clone();
        let enclosed = frame.nested();
        let dispatcher = frame.dispatcher.clone();
        let span = frame.span.clone();
        let concurrency = frame.concurrency;
        let declared = frame.declared;
        // The fan-out this frame's bound *is* — the enclosing one where that
        // supplies it, this frame's own dispatch otherwise. Either way its
        // `concurrency` is the frame's, which is what a map below it is read
        // against.
        let widest = match &frame.enclosing {
            Some(outer) if outer.concurrency >= concurrency => outer.clone(),
            _ => Enclosing {
                dispatcher: dispatcher.clone(),
                concurrency,
                span: span.clone(),
            },
        };
        // …and what tells that fan-out's instances apart, which is what a map
        // below can bind its own dispatches from.
        let varies = frame.varies();
        let enclosing = frame.enclosing.clone();
        if repeat.is_none() {
            frames.push(frame);
        }
        pending.push(Step::Leave(address.clone()));
        let mut nested_frames = Vec::new();
        for node in &flow.nodes {
            match &node.kind {
                NodeKind::Flow { flow: target, .. } => {
                    let address = target.value.to_string();
                    let Some(nested) = ctx.flow_named(&address) else {
                        continue;
                    };
                    let mut inner = BTreeMap::new();
                    let mut instance = BTreeMap::new();
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
                            // The same binding read against the other question:
                            // both travel inward through a `flow:` node, because
                            // both are properties of the one dispatch this
                            // instance belongs to. Where nothing encloses the
                            // dispatch there is nothing to be distinct between,
                            // and every field says so.
                            let between = !enclosed
                                || binding.is_some_and(|binding| {
                                    reads_a_varying_field(
                                        binding.value.value.as_str(),
                                        &per_instance,
                                    )
                                });
                            if let Some(binding) = binding {
                                bound_at.insert(name.clone(), binding.value.span.clone());
                            }
                            inner.insert(name.clone(), value);
                            instance.insert(name, between);
                        }
                    }
                    // One instance of the same dispatch, so everything the site
                    // is travels unchanged: the bound, the map that states it,
                    // and the fan-out it is inside.
                    nested_frames.push(Step::Enter(Box::new(Frame {
                        address,
                        flow: nested,
                        derived: inner,
                        per_instance: instance,
                        bound_at,
                        dispatcher: dispatcher.clone(),
                        span: span.clone(),
                        concurrency,
                        declared,
                        enclosing: enclosing.clone(),
                    })));
                }
                // A new site rather than the same one carried inward: its
                // item-derivation is re-rooted at this map's own item, its bound
                // is the looser of this map's and the fan-out it is inside, and
                // what that fan-out's instances *vary* is what its bindings can
                // inherit the distinction from.
                NodeKind::Map { map } => nested_frames.extend(
                    seeded(ctx, &address, node, map, Some(&widest), Some(&varies))
                        .into_iter()
                        .map(|frame| Step::Enter(Box::new(frame))),
                ),
                _ => {}
            }
        }
        nested_frames.reverse();
        pending.extend(nested_frames);
    }
}

/// Which input fields of a dispatched flow are item-derived at one site, which
/// of them the **enclosing** fan-out tells apart, and where each answer was
/// written (Decisions D83, D147).
///
/// The two answers come off the same binding and part on what they read of it.
/// Item-derivation is the map's own item, its index, and nothing else; the
/// enclosing question is `input.<field>` alone — of the fields `within` says the
/// instance around this map varies. The map's item is deliberately not one of
/// them, however that item was produced: two instances iterating two lists can
/// iterate onto one and the same value, and `execution.item_index` is the
/// *innermost* dispatch's index, which nested maps repeat across outer items
/// (grammar 4.1, D115).
fn seed(
    flow: &Flow,
    input: Option<&NodeInput>,
    item: &str,
    within: Option<&BTreeMap<String, bool>>,
) -> (
    BTreeMap<String, bool>,
    BTreeMap<String, bool>,
    BTreeMap<String, Span>,
) {
    let mut derived = BTreeMap::new();
    let mut per_instance = BTreeMap::new();
    let mut bound_at = BTreeMap::new();
    let Some(inputs) = &flow.inputs else {
        return (derived, per_instance, bound_at);
    };
    for field in &inputs.fields {
        let name = field.name.value.to_string();
        let (value, between) = match input {
            // The whole item is the instance's input, so every field is
            // item-derived — and no binding was written to point at, nor one
            // that could have carried the enclosing instance's own value in.
            None => (true, false),
            Some(NodeInput::Fields { bindings }) => {
                let binding = bindings
                    .entries
                    .iter()
                    .find(|binding| binding.name.value == name);
                if let Some(binding) = binding {
                    bound_at.insert(name.clone(), binding.value.span.clone());
                }
                (
                    binding.is_some_and(|binding| {
                        is_item_derived(binding.value.value.as_str(), Some(item), &BTreeMap::new())
                    }),
                    binding.is_some_and(|binding| {
                        within.is_some_and(|within| {
                            reads_a_varying_field(binding.value.value.as_str(), within)
                        })
                    }),
                )
            }
            // The scalar form binds a string-in agent, and an agent contains no
            // store nodes (grammar 11.4).
            Some(NodeInput::Scalar { .. }) => (false, false),
        };
        derived.insert(name.clone(), value);
        // Nothing encloses this map, so there is no second instance for a value
        // to fail to be distinct from ([`Frame::per_instance`]).
        per_instance.insert(name, within.is_none() || between);
    }
    (derived, per_instance, bound_at)
}

/// Whether one expression reads an `input.<field>` its own scope's caller tells
/// apart — the fields `varying` marks true ([`Frame::varies`]).
///
/// [`is_item_derived`]'s question asked of the *enclosing* fan-out rather than of
/// the dispatching map, and with neither of that predicate's two non-`input`
/// roots: the map's item is a value drawn from a list, and two instances
/// iterating two lists can iterate onto the same one, while
/// `execution.item_index` is the innermost enclosing dispatch's index, which
/// nested maps repeat across outer items (grammar 4.1, Decisions D115, D147).
/// What carries a distinction inward is a value the instance was handed, and
/// that is an `input.<field>` of it.
///
/// An unresolved index — `input[state.which]` — names a field nobody here can
/// name and is read as carrying nothing, which is the answer a refusal has to
/// give an unknown (PRD resolved q61 ruling b).
pub(crate) fn reads_a_varying_field(source: &str, varying: &BTreeMap<String, bool>) -> bool {
    crate::cel::analyze(source, &Scope::default())
        .reads
        .iter()
        .any(|read| {
            read.root == "input"
                && match read.path.first() {
                    Some(field) => varying.get(field).copied().unwrap_or(false),
                    None if read.indexed => false,
                    None => varying.values().any(|value| *value),
                }
        })
}

/// Whether one expression reads **no root at all** — no `input`, no `state`,
/// no `execution` — so that every scope it is ever evaluated in answers it the
/// same way.
///
/// The property that makes two *instances* comparable. Two coder nodes of one
/// flow share one scope, so two values written identically are one directory
/// however they read it; two instances of a flow do not (grammar 10.1), and
/// `workspace: "input.worktree"` is the repair the ruling is about rather than
/// a collision. A closed expression is the case where the difference cannot
/// matter (PRD resolved q61 ruling b).
pub(crate) fn reads_nothing(source: &str) -> bool {
    crate::cel::analyze(source, &Scope::default())
        .reads
        .is_empty()
}

/// Whether one expression is item-derived: it references the item binding, the
/// per-item index, or an `input.<field>` that is itself item-derived here.
///
/// A read that names no field is answered the way a rule this predicate
/// *exempts* a composition from has to answer an unknown. A read of the whole
/// `input` object embeds every field, so one derived field makes it derived; but
/// an index whose key this compiler could not resolve — `input[state.which]` —
/// names a field nobody here can name, and calling it derived would clear a
/// store write (grammar 11.4) or a shared workspace (Decision D147) on the
/// strength of a field nothing established. A **constant** index key is not one
/// of those: `input['worktree']` is `input.worktree` written the other way
/// round, and the CEL front-end resolves it to that name.
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
            && match read.path.first() {
                Some(field) => derived.get(field).copied().unwrap_or(false),
                None if read.indexed => false,
                None => derived.values().any(|value| *value),
            }
    })
}
