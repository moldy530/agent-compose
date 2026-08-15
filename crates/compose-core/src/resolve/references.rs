//! Resolving every reference the composition writes (grammar 2.3, 2.4).
//!
//! Two relations, kept apart because they have different domains:
//!
//! * a **typed address** (`model.smart`) resolves against the composition's
//!   definitions, in the namespaces grammar 2.3's position table admits at that
//!   position;
//! * a **flow-local node id** (`review`) resolves against the nodes of the flow
//!   that writes it — two flows may both have a node `review` (grammar 2.4).
//!
//! Both report at the **reference**, never at the definition: the reference is
//! what the author has to change. The parser has already refused a reference
//! whose *spelling* names a namespace the position does not admit, so lookup
//! here asks one question — is this address defined, in a namespace this
//! position accepts — and lets the answer carry both failures. That is why the
//! wrong-namespace arm exists without a fixture of its own: it is the position
//! table restated where resolution happens, not a second chance to catch a
//! spelling.

use std::collections::BTreeSet;

use crate::ast::common::{Address, ControlTarget, EdgeSource, EdgeTarget, Ident, Namespace};
use crate::ast::definition::{DefinitionBody, ModelDef, StoreDef};
use crate::ast::flow::{FlowDef, MapDispatch, MapRoute, Node, NodeKind};
use crate::ast::policy::OnError;
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Span, Spanned};
use crate::parse::reader::{list, suggest};

use super::index::Index;

/// Namespaces a `map` dispatch target and a placement key both accept.
const COMPONENTS: &[Namespace] = &[Namespace::Agent, Namespace::Tool, Namespace::Flow];

/// Resolve every reference in the composition's definitions and triggers.
pub(crate) fn check(index: &Index<'_>, diagnostics: &mut Diagnostics) {
    let mut cx = Cx { index, diagnostics };
    for declared in index.definitions.values() {
        match &declared.definition.body {
            DefinitionBody::Agent(agent) => {
                if let Some(model) = agent.model.as_ref() {
                    cx.address(model, &[Namespace::Model]);
                }
                for tool in &agent.tools {
                    cx.address(tool, &[Namespace::Tool, Namespace::Flow]);
                }
                for store in &agent.stores {
                    cx.address(store, &[Namespace::Store]);
                }
            }
            // A tool names no composition address: its `function:` binding is a
            // host registry entry, looked up at build time (grammar 6.1), and
            // its `exec:`/`http:` blocks name no definition at all.
            DefinitionBody::Tool(_) => {}
            DefinitionBody::Flow(flow) => cx.flow(&declared.definition.address, flow),
            DefinitionBody::Store(store) => cx.store(store),
            DefinitionBody::Provider(_) => {}
            DefinitionBody::Model(model) => match model {
                ModelDef::Direct(direct) => {
                    if let Some(provider) = direct.provider.as_ref() {
                        cx.address(provider, &[Namespace::Provider]);
                    }
                }
                ModelDef::Route(route) => {
                    for member in &route.route {
                        cx.address(member, &[Namespace::Model]);
                    }
                }
            },
        }
    }

    if let Some(section) = index.triggers.as_ref() {
        for trigger in &section.value.triggers {
            if let Some(flow) = trigger.flow.as_ref() {
                cx.address(flow, &[Namespace::Flow]);
            }
        }
    }
}

/// The walk's state: what is defined, and where to report.
pub(crate) struct Cx<'a, 'd> {
    pub(crate) index: &'a Index<'a>,
    pub(crate) diagnostics: &'d mut Diagnostics,
}

impl Cx<'_, '_> {
    /// Resolve one typed address against the namespaces this position accepts
    /// (grammar 2.3).
    pub(crate) fn address(&mut self, reference: &Spanned<Address>, accepts: &[Namespace]) {
        let found = self.index.get(&reference.value);
        if found.is_some() && accepts.contains(&reference.value.namespace) {
            return;
        }

        let written = reference.value.to_string();
        if let Some(declared) = found {
            // The same failure class the parser reports when a reference is
            // spelled with a namespace its position does not admit, so it
            // carries the same code — noticed here only if a position ever
            // accepts a wider set of spellings than of definitions.
            self.diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidReference,
                    reference.span.clone(),
                    format!(
                        "expected {} reference, found `{written}`",
                        expected_namespaces(accepts)
                    ),
                )
                .with_label(
                    declared.definition.address.span.clone(),
                    format!("`{written}` is defined here, in `{}`", declared.file),
                ),
            );
            return;
        }

        let candidates = self.index.addresses_in(accepts);
        let help = suggest(&written, &candidates)
            .map(|candidate| format!("did you mean `{candidate}`?"))
            .unwrap_or_else(|| {
                if candidates.is_empty() {
                    format!(
                        "the composition defines no {}: a definition is part of it only when the entrypoint imports the file that declares it (grammar 1.4)",
                        namespace_list(accepts)
                    )
                } else {
                    format!(
                        "the composition defines {}; a definition is part of it only when the entrypoint imports the file that declares it (grammar 1.4)",
                        list(&candidates)
                    )
                }
            });
        self.diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::UndefinedReference,
                reference.span.clone(),
                format!("`{written}` is not defined in this composition"),
            )
            .with_help(help),
        );
    }

    /// Resolve everything one flow definition names.
    fn flow(&mut self, address: &Spanned<Address>, flow: &FlowDef) {
        let ids: BTreeSet<&str> = flow
            .nodes
            .iter()
            .map(|node| node.id.value.as_str())
            .collect();
        let owner = address.value.to_string();

        for node in &flow.nodes {
            self.node(node);
            if let Some(Spanned {
                value: OnError::Fallback(target),
                ..
            }) = node.policy.on_error.as_ref()
            {
                self.control_target(target, &ids, &owner);
            }
            if let NodeKind::Human(human) = &node.kind
                && let Some(target) = human.on_timeout.as_ref()
            {
                self.control_target(target, &ids, &owner);
            }
        }

        for edge in &flow.edges {
            if let Some(from) = edge.from.as_ref()
                && let EdgeSource::Node(id) = &from.value
            {
                self.node_id(id, &from.span, &ids, &owner);
            }
            if let Some(to) = edge.to.as_ref()
                && let EdgeTarget::Node(id) = &to.value
            {
                self.node_id(id, &to.span, &ids, &owner);
            }
        }
    }

    /// Resolve the addresses one node names.
    fn node(&mut self, node: &Node) {
        match &node.kind {
            NodeKind::Agent(agent) => self.address(agent, &[Namespace::Agent]),
            NodeKind::Function(tool) => self.address(tool, &[Namespace::Tool]),
            NodeKind::Flow(flow) => self.address(&flow.flow, &[Namespace::Flow]),
            NodeKind::Store(store) => self.address(&store.store, &[Namespace::Store]),
            NodeKind::Map(map) => match &map.dispatch {
                MapDispatch::Homogeneous { node } => self.address(node, COMPONENTS),
                MapDispatch::Routed {
                    routes, default, ..
                } => {
                    for route in routes.iter().chain(default.as_deref()) {
                        self.route(route);
                    }
                }
                MapDispatch::Invalid => {}
            },
            NodeKind::Exec(_) | NodeKind::Http(_) | NodeKind::Human(_) | NodeKind::Invalid => {}
        }
    }

    /// Resolve one `map` route's dispatch target.
    fn route(&mut self, route: &MapRoute) {
        if let Some(node) = route.node.as_ref() {
            self.address(node, COMPONENTS);
        }
    }

    /// Resolve everything one store definition names.
    fn store(&mut self, store: &StoreDef) {
        if let Some(embed) = store.embed.as_ref()
            && let Some(provider) = embed.provider.as_ref()
        {
            self.address(provider, &[Namespace::Provider]);
        }
    }

    /// Resolve a control-transfer target: a node of this flow, or `end`
    /// (grammar 2.4, 9.2, 8.7).
    fn control_target(
        &mut self,
        target: &Spanned<ControlTarget>,
        ids: &BTreeSet<&str>,
        owner: &str,
    ) {
        if let ControlTarget::Node(id) = &target.value {
            self.node_id(id, &target.span, ids, owner);
        }
    }

    /// Resolve one flow-local node id.
    fn node_id(&mut self, id: &Ident, span: &Span, ids: &BTreeSet<&str>, owner: &str) {
        if ids.contains(id.as_str()) {
            return;
        }
        let candidates: Vec<&str> = ids.iter().copied().collect();
        let help = suggest(id.as_str(), &candidates)
            .map(|candidate| format!("did you mean `{candidate}`?"))
            .unwrap_or_else(|| {
                format!(
                    "node ids are scoped to their flow, and `{owner}` declares {}",
                    if candidates.is_empty() {
                        "none".to_string()
                    } else {
                        list(&candidates)
                    }
                )
            });
        self.diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::UndefinedReference,
                span.clone(),
                format!("`{id}` is not a node of `{owner}`"),
            )
            .with_help(help),
        );
    }
}

/// How a position's accepted namespaces are named in a diagnostic:
/// ``​`tool.*` or `flow.*` ``.
fn namespace_list(accepts: &[Namespace]) -> String {
    let names: Vec<String> = accepts
        .iter()
        .map(|namespace| format!("`{namespace}.*`"))
        .collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    }
}

/// The same list with its article, for grammar 2.3's own error form:
/// "expected an `agent.*` reference, found `tool.web_search`".
fn expected_namespaces(accepts: &[Namespace]) -> String {
    let article = match accepts.first() {
        Some(Namespace::Agent) => "an",
        _ => "a",
    };
    format!("{article} {}", namespace_list(accepts))
}

#[cfg(test)]
mod tests {
    use super::{expected_namespaces, namespace_list};
    use crate::ast::common::Namespace;

    #[test]
    fn accepted_namespaces_read_as_a_list() {
        assert_eq!(namespace_list(&[Namespace::Model]), "`model.*`");
        assert_eq!(
            namespace_list(&[Namespace::Tool, Namespace::Flow]),
            "`tool.*` or `flow.*`"
        );
        assert_eq!(
            namespace_list(&[Namespace::Agent, Namespace::Tool, Namespace::Flow]),
            "`agent.*`, `tool.*`, or `flow.*`"
        );
        assert_eq!(
            expected_namespaces(&[Namespace::Agent, Namespace::Tool, Namespace::Flow]),
            "an `agent.*`, `tool.*`, or `flow.*`"
        );
        assert_eq!(expected_namespaces(&[Namespace::Model]), "a `model.*`");
    }
}
