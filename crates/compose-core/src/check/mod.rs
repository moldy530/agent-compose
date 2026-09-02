//! The validator: every static check `docs/grammar.md` and PRD §7 M0 define
//! over a resolved composition.
//!
//! The pass reads one resolved [`Ir`] and reports through [`Diagnostic`]. It
//! adds nothing to the artifact and changes nothing in it: an artifact exists
//! only when resolution rejected nothing (see [`resolve`](crate::resolve)), and
//! what this pass decides is whether the composition that produced it also
//! holds together.
//!
//! The rules fall into two families by the *evidence* they need, and the split
//! is the one Appendix B draws. `crates/compose-core/tests/static_check_inventory.rs`
//! is the whole account in executable form: each check, its pass, and the codes
//! it reports through.
//!
//! # The data checks
//!
//! The first family is decided from the IR's **schemas and expressions**. One
//! submodule per rule family, each rule reporting a stable code
//! ([`DiagnosticCode`](crate::diag::DiagnosticCode)):
//!
//! | Module | Grammar | What it decides |
//! |---|---|---|
//! | [`expr`] | 4.1 | every CEL expression, at the surface it was written on: roots, paths, constructs, result type |
//! | [`bindings`] | 8.0, 5.4, 11.5 | node input bindings against the target's declared input, name-based reads, `writes:` remaps and the injectivity of the effective write map, and what an agent's two attachment lists produce together |
//! | [`channels`] | 7.5, 10 | write typing against the reduce policy, undefined channels, and a flow's `outputs:` against the channels it materializes from |
//! | [`maps`] | 8.6 | `over` resolution and bounding, route narrowing and exhaustiveness, per-item bindings and the two `input:` forms, what a dispatch — detached or not — may write, and the environment slot a detached delivery shares with an `exec:` sink's input |
//! | [`stores`] | 11 | store-op parameters and values against the op's row and the store's schemas, map-write keying, session-scope coherence |
//! | [`providers`] | 11.2, 12 | `settings:` against the provider kind's published schema, and the three capability checks |
//! | [`triggers`] | 13 | a trigger's `input:` against its flow's declared inputs, and what its payload can supply |
//!
//! # The graph checks
//!
//! The second family reads the composition's **graphs** rather than its
//! schemas. Every one of them needs a relation over a flow's nodes, or over the
//! composition's definitions, that no key-and-value pair can supply:
//!
//! | Module | Grammar | What it decides |
//! |---|---|---|
//! | [`graph`] | 7.2, 7.4, 7.8 | the flow graph itself: adjacency, SCCs, reachability, dominance, and step distances. Answers questions; reports nothing |
//! | [`guards`] | 7.3.1 | the closed guard-shape table, read for coverage and for disjointness. Answers questions; reports nothing |
//! | [`routing`] | 7.3.1, 7.6.3 | routing exhaustiveness over enum output fields, and the two no-dead-end rules stated over a node |
//! | [`cycles`] | 7.4 | every SCC is bounded, and every bounded edge has an escape |
//! | [`convergence`] | 7.6.1, 7.6.2, 10.2 | forks and co-takeable pairs, balanced convergence, and the concurrent-branch half of the reduced-channel rule |
//! | [`reachable`] | 7.8 | every node is reachable from its flow's `start` |
//! | [`components`] | 7.5, 7.7, 13.3 | recursion, and a `respond: sync` trigger's flow reaching a `human` node |
//! | [`placements`] | 14.1 | an attached tool's placement against the placement of every agent that attaches it, and a store on a process-local backend against the processes that can open it |
//! | [`fanout`] | 8.6 | `map.over` dominance, and `detach:` under a durably checkpointed target |
//! | [`modules`] | 6.1 | the `module:` bindings of a composition read against each other: one authored file per tool, and one version per package across every binding's dependencies |
//!
//! Where a rule splits across the two families — `map.over` resolves a path in
//! [`maps`] and proves dominance in [`fanout`] — each half is stated where its
//! evidence is. The order the two families run in is not observable: every
//! diagnostic carries a span and the whole report is sorted into source order at
//! the end.
//!
//! # What is not here
//!
//! **Everything presence-shaped.** Reading a value that is legally absent fails
//! the execution (Decision D110); no static rule anticipates it. See
//! [`cel`](crate::cel) for the full account of what is deferred to run time.
//!
//! **Everything the earlier passes own.** A rule decidable from one file is the
//! parser's — the `start` edge of grammar 7.6.3 rule 2, an `else:` edge's
//! guarded sibling (Decision D107), a duplicate edge — and a rule decidable from
//! names and addresses is the resolver's. `docs/grammar.md` Appendix B is the
//! normative account of the split. Grammar 12.1's conditional credential rule
//! (Decision D120) is one file's business and so is the parser's; the table its
//! message reads is
//! [`ProviderKind::default_endpoint`](crate::ast::ProviderKind::default_endpoint),
//! and neither half of it is in [`providers`].
//!
//! **The one rule that reads the filesystem.** A `module:` binding's authored
//! file has to exist, and [`check`] cannot decide that from an [`Ir`]. It is
//! [`modules::missing`], a pass of its own that the two verbs answering a
//! verdict — `validate` and `build --check` — run beside this one, and that a
//! plain `build`, which scaffolds the absent file rather than refusing over it,
//! deliberately does not. See [`modules`] for the whole argument.

pub(crate) mod bindings;
pub(crate) mod channels;
pub(crate) mod components;
pub(crate) mod convergence;
pub(crate) mod cycles;
pub(crate) mod expr;
pub(crate) mod fanout;
pub(crate) mod graph;
pub(crate) mod guards;
pub(crate) mod maps;
pub(crate) mod model;
pub mod modules;
pub(crate) mod placements;
pub(crate) mod providers;
pub(crate) mod reach;
pub(crate) mod reachable;
pub(crate) mod routing;
pub(crate) mod stores;
pub(crate) mod triggers;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::{Address, Ident, Namespace};
use crate::cel::ty::{Origin, Property, Type};
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Span, Spanned};
use crate::ir::binding::Http;
use crate::ir::definition::{Agent, DefinitionBody, Model, Provider, Store, Tool};
use crate::ir::flow::{Flow, Node, NodeKind};
use crate::ir::schema::FieldMap;
use crate::ir::{Channel, Ir};

/// Run every static check over one resolved composition.
///
/// Diagnostics come back in source order, so a report reads top to bottom
/// whatever order the checks visited constructs in.
#[must_use]
pub fn check(ir: &Ir) -> Vec<Diagnostic> {
    let mut ctx = Ctx::new(ir);
    providers::check(&mut ctx);
    stores::check_embeddings(&mut ctx);
    triggers::check(&mut ctx);
    stores::check_session_scope(&mut ctx);
    stores::check_map_writes(&mut ctx);
    placements::check(&mut ctx);
    placements::check_stores(&mut ctx);
    components::check(&mut ctx);
    modules::check(&mut ctx);

    let ir = ctx.ir;
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Flow(flow) => flow_definition(&mut ctx, address, flow),
            DefinitionBody::Tool(tool) => bindings::tool_implementation(&mut ctx, address, tool),
            DefinitionBody::Agent(agent) => bindings::agent_tools(&mut ctx, address, agent),
            _ => {}
        }
    }

    // The graph checks run after the data ones on purpose: the routing analyses
    // read only the guards that type-checked, and which those are is what the
    // pass above has just decided (see `Ctx::reject_guard`).
    for (address, definition) in &ir.definitions {
        let DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        let cx = FlowCx { address, flow };
        let graph = graph::Graph::new(flow);
        reachable::check(&mut ctx, &cx, &graph);
        routing::check(&mut ctx, &cx, &graph);
        cycles::check(&mut ctx, &cx, &graph);
        convergence::check(&mut ctx, &cx, &graph);
        fanout::check(&mut ctx, &cx, &graph);
    }

    ctx.finish()
}

fn flow_definition<'a>(ctx: &mut Ctx<'a>, address: &'a str, flow: &'a Flow) {
    let cx = FlowCx { address, flow };
    channels::flow_outputs(ctx, &cx);
    for node in &flow.nodes {
        bindings::node(ctx, &cx, node);
        bindings::builtin_node_targets(ctx, node);
        channels::node_writes(ctx, &cx, node);
        match &node.kind {
            NodeKind::Map { map } => maps::map_node(ctx, &cx, node, map),
            NodeKind::Store { store, params, .. } => {
                stores::store_node(ctx, &cx, node, store, params);
            }
            NodeKind::Http { http } => bindings::inline_http(ctx, &cx, node, http),
            _ => {}
        }
    }
    for edge in &flow.edges {
        expr::edge_guard(ctx, &cx, edge);
    }
}

/// The flow a check is inside, and the address it is defined at — which is
/// what every diagnostic and every scope label names it by.
#[derive(Clone, Copy)]
pub(crate) struct FlowCx<'a> {
    /// The `flow.<name>` address.
    pub(crate) address: &'a str,
    /// The definition.
    pub(crate) flow: &'a Flow,
}

/// What a construct accepts as its input (grammar 8.0, 5.3).
#[derive(Clone, Copy, Debug)]
pub(crate) enum InputContract<'a> {
    /// A declared, named input object: the field-map binding form.
    Fields(&'a FieldMap),
    /// A declared input surface with no fields: a flow with no `inputs:`, a
    /// no-argument tool. Named like [`Fields`](Self::Fields) — every binding is
    /// an unknown field — but with nothing to require.
    Empty,
    /// A **string-in** agent: one unnamed string, bound with the scalar form
    /// (grammar 5.3, Decision D14).
    StringIn,
    /// An inline node, which declares no input schema of its own: its bindings
    /// build an ad-hoc object (grammar 8.2, 8.3).
    AdHoc,
    /// Nothing was resolvable — the reference was refused earlier, or the kind
    /// takes no `input:` at all.
    Unknown,
}

impl<'a> InputContract<'a> {
    /// The declared input fields, where the contract names any.
    pub(crate) const fn declared(self) -> Option<&'a FieldMap> {
        match self {
            Self::Fields(fields) => Some(fields),
            _ => None,
        }
    }
}

/// Everything the checks share: the artifact, the diagnostics they report
/// into, and the lookups they all need.
pub(crate) struct Ctx<'a> {
    /// The composition under check.
    pub(crate) ir: &'a Ir,
    diagnostics: Diagnostics,
    channels: BTreeMap<&'a str, &'a Channel>,
    /// The edge guards the CEL front-end refused, by the position they were
    /// written at — a file and a byte offset, which is one expression.
    rejected: BTreeSet<(String, usize)>,
}

impl<'a> Ctx<'a> {
    fn new(ir: &'a Ir) -> Self {
        let channels = ir.state.as_ref().map_or_else(BTreeMap::new, |section| {
            section
                .entries
                .iter()
                .map(|(name, channel)| (name.as_str(), channel))
                .collect()
        });
        Self {
            ir,
            diagnostics: Diagnostics::new(),
            channels,
            rejected: BTreeSet::new(),
        }
    }

    /// Record that an edge guard did not type-check, so the routing analyses
    /// leave it alone (see [`routing`], [`convergence`]).
    pub(crate) fn reject_guard(&mut self, at: &Span) {
        self.rejected
            .insert((at.source.as_str().to_string(), at.bytes.start));
    }

    /// Whether the guard written at this position type-checked.
    pub(crate) fn guard_type_checked(&self, at: &Span) -> bool {
        !self
            .rejected
            .contains(&(at.source.as_str().to_string(), at.bytes.start))
    }

    fn finish(mut self) -> Vec<Diagnostic> {
        self.diagnostics.sort();
        self.diagnostics.into_vec()
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn error(&mut self, code: DiagnosticCode, span: &Span, message: impl Into<String>) {
        self.diagnostics.error(code, span.clone(), message);
    }

    // --- definition lookups ----------------------------------------------

    fn body(&self, address: &Address) -> Option<&'a DefinitionBody> {
        self.ir
            .definitions
            .get(&address.to_string())
            .map(|definition| &definition.body)
    }

    pub(crate) fn agent(&self, address: &Address) -> Option<&'a Agent> {
        match self.body(address) {
            Some(DefinitionBody::Agent(agent)) => Some(agent),
            _ => None,
        }
    }

    pub(crate) fn tool(&self, address: &Address) -> Option<&'a Tool> {
        match self.body(address) {
            Some(DefinitionBody::Tool(tool)) => Some(tool),
            _ => None,
        }
    }

    pub(crate) fn flow(&self, address: &Address) -> Option<&'a Flow> {
        match self.body(address) {
            Some(DefinitionBody::Flow(flow)) => Some(flow),
            _ => None,
        }
    }

    pub(crate) fn store(&self, address: &Address) -> Option<&'a Store> {
        match self.body(address) {
            Some(DefinitionBody::Store(store)) => Some(store),
            _ => None,
        }
    }

    pub(crate) fn provider(&self, address: &Address) -> Option<&'a Provider> {
        match self.body(address) {
            Some(DefinitionBody::Provider(provider)) => Some(provider),
            _ => None,
        }
    }

    pub(crate) fn model(&self, address: &Address) -> Option<&'a Model> {
        match self.body(address) {
            Some(DefinitionBody::Model(model)) => Some(model),
            _ => None,
        }
    }

    /// The flow definition at this address, by its `flow.<name>` string.
    pub(crate) fn flow_named(&self, address: &str) -> Option<&'a Flow> {
        reach::flow_at(self.ir, address)
    }

    // --- state -----------------------------------------------------------

    /// The channel of this name, if `state:` declares one.
    pub(crate) fn channel(&self, name: &str) -> Option<&'a Channel> {
        self.channels.get(name).copied()
    }

    /// The `state` root: an object whose members are the declared channels
    /// (grammar 4.1, 10.1).
    pub(crate) fn state_type(&self) -> Type {
        Type::object(
            Origin::State,
            self.channels
                .values()
                .map(|channel| Property {
                    name: channel.name.value.to_string(),
                    // A nested object is named by the declaration that carries
                    // it, exactly as `model::properties` names an agent's, so a
                    // bad member of `state.totals` reports against the channel
                    // rather than against an anonymous "this object".
                    ty: model::type_of_named(
                        &channel.ty,
                        format!("the channel `{}`", channel.name.value),
                    ),
                    // A channel is a value the flow instance holds, not a
                    // property of an object: whether it is *set* is a runtime
                    // question (Decision D78), never a declared one, and a
                    // channel `default:` is the initial value it holds rather
                    // than something a reader of `state` supplies (grammar
                    // 3.6, 10.1).
                    optional: false,
                    defaulted: false,
                })
                .collect(),
        )
    }

    // --- node shapes -----------------------------------------------------

    /// A node's result schema: the one its target declares, the one its kind
    /// defaults to, or the one grammar 11.4 derives (see [`model`]).
    ///
    /// `None` where the node has no output at all — a `map` node (grammar 8.6
    /// rule 9) — or where the reference it names did not resolve to a
    /// definition of the right namespace, which the resolver has already
    /// reported.
    pub(crate) fn node_output(&self, node: &'a Node) -> Option<Cow<'a, FieldMap>> {
        match &node.kind {
            NodeKind::Agent { agent } => self.agent(&agent.value).map(|a| Cow::Borrowed(&a.output)),
            NodeKind::Function { function } => {
                self.tool(&function.value).map(|t| Cow::Borrowed(&t.output))
            }
            NodeKind::Flow { flow, .. } => {
                self.flow(&flow.value).map(|f| Cow::Borrowed(&f.outputs))
            }
            NodeKind::Human { human } => Some(Cow::Borrowed(&human.output)),
            NodeKind::Exec { exec } => Some(exec.output.as_ref().map_or_else(
                || Cow::Owned(model::exec_default_output(&exec.span)),
                Cow::Borrowed,
            )),
            NodeKind::Http { http } => Some(http.output.as_ref().map_or_else(
                || Cow::Owned(model::http_default_output(&http.span)),
                Cow::Borrowed,
            )),
            NodeKind::Store { store, op, params } => self.store(&store.value).map(|definition| {
                Cow::Owned(model::store_output(
                    definition.kind,
                    *op,
                    definition.value_schema.as_ref(),
                    definition.metadata_schema.as_ref(),
                    params.top_k.or(params.limit),
                    &node.span,
                ))
            }),
            NodeKind::Map { .. } => None,
        }
    }

    /// What a node's target accepts as its input (grammar 8.0).
    pub(crate) fn node_input(&self, node: &'a Node) -> InputContract<'a> {
        match &node.kind {
            NodeKind::Agent { agent } => self
                .agent(&agent.value)
                .map_or(InputContract::Unknown, |agent| self.agent_input(agent)),
            NodeKind::Function { function } => self
                .tool(&function.value)
                .map_or(InputContract::Unknown, |tool| {
                    InputContract::Fields(&tool.input)
                }),
            NodeKind::Flow { flow, .. } => {
                self.flow(&flow.value)
                    .map_or(InputContract::Unknown, |flow| {
                        flow.inputs
                            .as_ref()
                            .map_or(InputContract::Empty, InputContract::Fields)
                    })
            }
            NodeKind::Human { human } => InputContract::Fields(&human.input),
            NodeKind::Exec { .. } | NodeKind::Http { .. } => InputContract::AdHoc,
            NodeKind::Map { .. } | NodeKind::Store { .. } => InputContract::Unknown,
        }
    }

    /// What a dispatch target accepts (grammar 8.6 rule 12).
    pub(crate) fn target_input(&self, address: &Address) -> InputContract<'a> {
        match address.namespace {
            Namespace::Agent => self
                .agent(address)
                .map_or(InputContract::Unknown, |agent| self.agent_input(agent)),
            Namespace::Tool => self.tool(address).map_or(InputContract::Unknown, |tool| {
                InputContract::Fields(&tool.input)
            }),
            Namespace::Flow => self.flow(address).map_or(InputContract::Unknown, |flow| {
                flow.inputs
                    .as_ref()
                    .map_or(InputContract::Empty, InputContract::Fields)
            }),
            _ => InputContract::Unknown,
        }
    }

    /// A dispatch target's result schema (grammar 8.6 rule 5, 7).
    pub(crate) fn target_output(&self, address: &Address) -> Option<&'a FieldMap> {
        match address.namespace {
            Namespace::Agent => self.agent(address).map(|agent| &agent.output),
            Namespace::Tool => self.tool(address).map(|tool| &tool.output),
            Namespace::Flow => self.flow(address).map(|flow| &flow.outputs),
            _ => None,
        }
    }

    fn agent_input(&self, agent: &'a Agent) -> InputContract<'a> {
        agent
            .input
            .as_ref()
            .map_or(InputContract::StringIn, InputContract::Fields)
    }

    /// The node with this flow-local id.
    pub(crate) fn node_of<'f>(&self, flow: &'f Flow, id: &Ident) -> Option<&'f Node> {
        flow.nodes.iter().find(|node| node.id.value == *id)
    }
}

/// A `tool.*` implementation's `http:` block, where one exists — the surface
/// whose CEL sees only the tool's own `input` (grammar 6.1, Decision D65).
pub(crate) fn tool_http(tool: &Tool) -> Option<&Http> {
    match &tool.implementation {
        crate::ir::flow::ToolImplementation::Http { http } => Some(http),
        _ => None,
    }
}

/// The names a field map declares.
pub(crate) fn field_names(map: &FieldMap) -> Vec<&str> {
    map.fields
        .iter()
        .map(|field| field.name.value.as_str())
        .collect()
}

/// A spanned identifier's text.
pub(crate) fn text(name: &Spanned<Ident>) -> &str {
    name.value.as_str()
}
