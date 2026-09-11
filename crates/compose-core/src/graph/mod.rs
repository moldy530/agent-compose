//! `agent-compose visualize`: the resolved graph, as a picture of itself.
//!
//! PRD resolved q56 names the two halves and this module is both of them: a
//! **graph document** — a versioned JSON surface derived from the resolved IR,
//! normatively specified by `docs/graph.md` — and a **template** that renders
//! it, embedded in the compiler and emitting one self-contained HTML file.
//!
//! # Why the document is a surface of its own
//!
//! The IR is the artifact a deployment ships and diffs (PRD 5.1), and it is
//! *faithful rather than convenient*: defaults are not materialized, a policy
//! chain is not resolved, a model is an address rather than a provider. A
//! renderer needs the opposite — the resolved answer, one hop from the node it
//! is drawn on — so reading the IR in the template would put grammar 9.3's
//! chain, grammar 11.5's synthesized tool table and grammar 8.6's variant
//! narrowing into JavaScript, three places this compiler already decides them.
//!
//! So the compiler decides them once more and writes the answers down. That
//! makes the document worth versioning on its own terms, which is what
//! `graph_version` is: `--format json` prints exactly what the template
//! renders, so anything else that wants a picture of a composition reads the
//! same bytes the picture does.
//!
//! # Why the artifact fetches nothing
//!
//! PRD 5.12's single-binary posture extends to what the binary emits: the file
//! must open on a machine with no network, so the template, its styles and its
//! layout code are embedded on resolved q23's terms — nothing is looked up at
//! run time. [`render`] is the one place that is enforced in code, and
//! `crates/compose-core/tests/graph_artifact_is_self_contained.rs` is where it
//! is enforced over the bytes.
//!
//! # What it never carries
//!
//! An `${ENV}` reference reaches this document exactly as the author wrote it,
//! because the IR never substitutes one (PRD 5.9, resolved q15) and nothing
//! here resolves the process environment. A picture of a composition is a file
//! people paste into pull requests; a rendered credential would be a credential
//! in a file somebody is about to share.

mod build;
mod document;
mod render;
mod summary;

use crate::ir::Ir;

pub use document::{
    AgentView, BindingView, DispatchView, EdgeClass, ExecView, FieldView, FlowGraph, GRAPH_VERSION,
    GraphDocument, GraphEdge, GraphNode, HttpView, HumanView, InputView, InstancePolicyView,
    ItemErrorView, MapView, ModelView, NodeKind, OnErrorView, PolicyLevel, PolicyView,
    ProviderView, RetryView, RouteView, SchemaSource, SchemaView, SchemasView, SettingView,
    StoreView, SubflowView, TimeoutView, ToolSource, ToolView, TriggerView, WriteView,
};
pub use render::{TEMPLATE, render};

/// The graph document for a whole composition: one canvas per `flow.*`, in
/// address order.
#[must_use]
pub fn graph(ir: &Ir) -> GraphDocument {
    build::document(ir, None)
}

/// The graph document for one flow, named by its typed address.
///
/// `None` where the composition declares no such flow — which is a usage error
/// for the caller to report with the vocabulary [`flows`] answers, rather than
/// a diagnostic: rendering adds usage errors and never a new failure class
/// (PRD resolved q56).
#[must_use]
pub fn graph_of(ir: &Ir, flow: &str) -> Option<GraphDocument> {
    let document = build::document(ir, Some(flow));
    (!document.flows.is_empty()).then_some(document)
}

/// Every `flow.*` address the composition declares, sorted — the vocabulary a
/// refusal lists.
#[must_use]
pub fn flows(ir: &Ir) -> Vec<String> {
    build::flow_addresses(ir)
}

/// The closest flow address to `name`, when one is close enough to suggest.
///
/// The same suggestion every unknown name in this compiler gets (PRD G3): a
/// refusal that lists a vocabulary is better with the near miss named, and
/// `--flow flow.triage` mistyped is the likeliest way anyone meets this one.
#[must_use]
pub fn nearest_flow(ir: &Ir, name: &str) -> Option<String> {
    let known = flows(ir);
    let borrowed: Vec<&str> = known.iter().map(String::as_str).collect();
    crate::parse::reader::suggest(name, &borrowed).map(str::to_string)
}
