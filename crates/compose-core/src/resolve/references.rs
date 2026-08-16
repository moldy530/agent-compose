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
//! spelling. No corpus project can reach it — every position in grammar 2.3's
//! table refuses a wrong-namespace *spelling* at the parser — so it is pinned
//! by a unit test at the bottom of this module, the way `resolve::index` pins
//! the version-mismatch rule that is dormant for the same kind of reason.
//!
//! One position asks a second question once its address has resolved: a model
//! route's members MUST be **direct** models, never routes of their own
//! (grammar 2.3's position table, grammar 12.2). That is a property of the
//! *definition* the member names rather than of the reference, so the parser
//! cannot decide it the way it decides the route's other two member rules — and
//! it costs one index lookup rather than a graph or a type, so it does not wait
//! for the pass that reads the IR either.

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
                        cx.route_member(member);
                    }
                }
            },
            // A definition whose body the parser could not read has had its
            // say, and there is nothing left of it to resolve. It is in the
            // index for its *name*, so that references to it resolve rather
            // than turning one wrong-type diagnostic into one per use site.
            DefinitionBody::Invalid => {}
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
                )
                .with_help(format!(
                    "a position accepts a fixed set of namespaces whatever the composition happens to define: this one takes {} (grammar 2.3)",
                    namespace_list(accepts)
                )),
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

    /// Resolve one member of a model route: a `model.*` that is itself a
    /// **direct** model (grammar 2.3's position table, grammar 12.2).
    ///
    /// The parser owns the route's other two member rules — a repeated member,
    /// and a route of fewer than two — because both are decidable from the list
    /// as written. This one is not: whether a member is a route is a property
    /// of the *definition* it names, which is a lookup in this pass's index.
    /// It is still neither a graph nor a type, so it belongs here rather than
    /// with the checks that read the IR (see [`super`]).
    fn route_member(&mut self, member: &Spanned<Address>) {
        self.address(member, &[Namespace::Model]);
        // An address that did not resolve, or resolved to something that is not
        // a model, has already been reported by `address`.
        let Some(declared) = self.index.get(&member.value) else {
            return;
        };
        let DefinitionBody::Model(ModelDef::Route(_)) = &declared.definition.body else {
            return;
        };
        let written = member.value.to_string();
        self.diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                member.span.clone(),
                format!("`{written}` is a route, and a route's members are direct models"),
            )
            .with_label(
                declared.definition.address.span.clone(),
                format!("`{written}` is defined here, in `{}`", declared.file),
            )
            .with_help(
                "a route is an ordered fallback between direct bindings: nesting one inside another hides a second failover policy in the first, and a route that names itself falls back to the model that just failed (grammar 2.3, 12.2)",
            ),
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
    use super::{Cx, expected_namespaces, namespace_list};
    use crate::ast::common::{Address, Ident, Namespace};
    use crate::ast::document::Document;
    use crate::diag::{
        Diagnostic, DiagnosticCode, Diagnostics, Position, SourceName, Span, Spanned,
    };
    use crate::ir::SourceRole;
    use crate::resolve::files::{Composition, SpecSource};
    use crate::resolve::index;

    /// One source text as a complete, cleanly parsed one-file composition.
    fn one_file(source: &str) -> Composition {
        let parsed = crate::parse_str(source, "main.yml");
        assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
        let Some(Document::Spec(file)) = parsed.document else {
            panic!("the source is a spec file");
        };
        Composition {
            entrypoint: "main.yml".to_string(),
            target: crate::resolve::DEFAULT_TARGET.to_string(),
            files: vec![SpecSource {
                name: "main.yml".to_string(),
                role: SourceRole::Entrypoint,
                clean: true,
                file,
            }],
            deploy: None,
            complete: true,
        }
    }

    /// Resolve every reference in a one-file composition, the way
    /// [`super::check`] is called on a real one.
    fn resolve_all(source: &str) -> Vec<Diagnostic> {
        let composition = one_file(source);
        let mut diagnostics = Diagnostics::new();
        let index = index::build(&composition, &mut diagnostics);
        assert!(diagnostics.into_vec().is_empty());

        let mut diagnostics = Diagnostics::new();
        super::check(&index, &mut diagnostics);
        diagnostics.into_vec()
    }

    /// Resolve one reference against a one-file composition, whatever the
    /// parser would have made of that spelling in a real position.
    fn resolve_one(
        source: &str,
        reference: Address,
        accepts: &[Namespace],
    ) -> (Option<String>, Option<String>) {
        let composition = one_file(source);
        let mut diagnostics = Diagnostics::new();
        let index = index::build(&composition, &mut diagnostics);
        assert!(diagnostics.into_vec().is_empty());

        let mut diagnostics = Diagnostics::new();
        let span = Span::new(
            SourceName::new("main.yml"),
            0..0,
            Position::new(9, 3),
            Position::new(9, 9),
        );
        Cx {
            index: &index,
            diagnostics: &mut diagnostics,
        }
        .address(&Spanned::new(reference, span), accepts);
        let reported: Vec<Diagnostic> = diagnostics.into_vec();
        assert!(reported.len() <= 1, "{reported:?}");
        match reported.into_iter().next() {
            None => (None, None),
            Some(diagnostic) => {
                assert_eq!(diagnostic.code, DiagnosticCode::InvalidReference);
                (Some(diagnostic.message), diagnostic.help)
            }
        }
    }

    const PROVIDER: &str = "version: \"0.1\"\nprovider.p:\n  kind: anthropic\n  api_key: ${KEY}\n";

    /// One provider and two direct models, for the route cases to build on.
    const MODELS: &str = "version: \"0.1\"\nprovider.p:\n  kind: anthropic\n  api_key: ${KEY}\nmodel.a:\n  provider: provider.p\n  id: a\nmodel.b:\n  provider: provider.p\n  id: b\n";

    /// The wrong-namespace arm: the address *is* defined, in a namespace this
    /// position does not accept. Every position in grammar 2.3's table refuses
    /// that spelling at the parser, so no fixture project reaches this — the
    /// arm is the position table restated where resolution happens, and this is
    /// what keeps it honest.
    #[test]
    fn a_defined_address_in_a_namespace_the_position_refuses_is_reported_there() {
        let (message, help) = resolve_one(
            PROVIDER,
            Address::new(Namespace::Provider, Ident::new("p")),
            &[Namespace::Model],
        );
        assert_eq!(
            message.as_deref(),
            Some("expected a `model.*` reference, found `provider.p`")
        );
        assert_eq!(
            help.as_deref(),
            Some(
                "a position accepts a fixed set of namespaces whatever the composition happens to define: this one takes `model.*` (grammar 2.3)"
            )
        );
    }

    /// The same arm with a multi-namespace position, so the help's list is not
    /// pinned only in its one-element form.
    #[test]
    fn the_wrong_namespace_help_names_every_namespace_the_position_takes() {
        let (_, help) = resolve_one(
            PROVIDER,
            Address::new(Namespace::Provider, Ident::new("p")),
            &[Namespace::Tool, Namespace::Flow],
        );
        assert_eq!(
            help.as_deref(),
            Some(
                "a position accepts a fixed set of namespaces whatever the composition happens to define: this one takes `tool.*` or `flow.*` (grammar 2.3)"
            )
        );
    }

    /// The accepting case, so the test above is not passing because every
    /// reference is refused.
    #[test]
    fn a_defined_address_the_position_accepts_is_not_reported() {
        assert_eq!(
            resolve_one(
                PROVIDER,
                Address::new(Namespace::Provider, Ident::new("p")),
                &[Namespace::Provider],
            ),
            (None, None)
        );
    }

    /// Two direct models in fallback order — the legal shape. The rule below
    /// refuses a *nested* route, not routes in general, and over-rejection is
    /// the failure mode a negative corpus cannot catch.
    #[test]
    fn a_route_of_direct_models_is_not_reported() {
        let reported = resolve_all(&format!(
            "{MODELS}model.tier_one:\n  route: [model.a, model.b]\n"
        ));
        assert!(reported.is_empty(), "{reported:?}");
    }

    /// The degenerate nesting: a route whose second member is the route itself,
    /// so its fallback is the model that just failed. The corpus fixture
    /// `model-route-names-a-route` pins the spelling an author writes on
    /// purpose; this pins the one a reader might expect a special case for, and
    /// there is none — it is the same lookup and the same diagnostic.
    #[test]
    fn a_route_that_names_itself_is_reported_like_any_other_nested_route() {
        let reported = resolve_all(&format!("{MODELS}model.r:\n  route: [model.a, model.r]\n"));
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert_eq!(reported[0].code, DiagnosticCode::InvalidValue);
        assert_eq!(
            reported[0].message,
            "`model.r` is a route, and a route's members are direct models"
        );
        assert_eq!(reported[0].labels.len(), 1);
        assert_eq!(
            reported[0].labels[0].message,
            "`model.r` is defined here, in `main.yml`"
        );
    }

    /// A member that is not defined at all is reported once, as undefined —
    /// the route rule needs a definition to read and says nothing without one.
    #[test]
    fn an_undefined_route_member_is_reported_only_as_undefined() {
        let reported = resolve_all(&format!(
            "{MODELS}model.r:\n  route: [model.a, model.gone]\n"
        ));
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert_eq!(reported[0].code, DiagnosticCode::UndefinedReference);
        assert_eq!(
            reported[0].message,
            "`model.gone` is not defined in this composition"
        );
    }

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
