//! Input bindings, name-based wiring, and the two `input:` forms (grammar 8.0).
//!
//! Grammar 8.0 states two resolution chains and this module is both of them.
//! For an **in-flow target** — `agent:`, `exec:`, `http:`, `function:`,
//! `human:` — an input field resolves to an explicit binding, else the
//! same-named state channel, else the same-named flow input, else its own
//! `default:`, else a compile error. For a **module boundary** — a `flow:` node
//! and a `map` dispatch — only the first and fourth steps exist: nothing
//! crosses implicitly (Decision D68).
//!
//! The two steps in the middle are where two *declarations* meet with no
//! expression between them, so they are typed by [`satisfies`](super::model)
//! rather than by a result type: the channel, or the flow input, must be no
//! wider than the field it lands in (Decision D111).
//!
//! The **form** rules are here too. The parser refuses a scalar `input:` on the
//! kinds where an unnamed value names nothing (Decision D88); what it cannot
//! see is whether the *agent* a node names is string-in, because that is
//! another file. So the pairing — scalar into a string-in agent, field map into
//! a declared object — is decided here (grammar 5.3, 8.6 rule 12, Decisions
//! D14, D75).

use crate::ast::common::Namespace;
use crate::ast::definition::{AgentAccess, StoreKind};
use crate::cel::Scope;
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::binding::{Bindings, Http, NodeInput};
use crate::ir::definition::{Agent, Store, Tool};
use crate::ir::flow::{Node, NodeKind};
use crate::ir::schema::{Field, FieldMap};

use super::model::{default_of, satisfies};
use super::{Ctx, FlowCx, InputContract, expr, field_names, text};

/// Check one node's `input:` bindings and the fields they leave unbound.
pub(crate) fn node<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, node: &'a Node) {
    let contract = ctx.node_input(node);
    let id = text(&node.id).to_string();
    let subject = format!("node `{id}`");
    let scope = expr::flow_scope(ctx, cx, format!("the `input:` binding of node `{id}`"));

    // A form the target cannot take is one mistake: the fields it leaves
    // unbound are a consequence of it, not a second finding.
    let mut form_refused = false;
    match (&node.input, contract) {
        (Some(NodeInput::Scalar { value }), InputContract::StringIn | InputContract::AdHoc) => {
            // A string-in agent takes one unnamed string; an inline `exec:`
            // node passes it on the child's stdin (grammar 5.3, 6.1, 8.2).
            let analysis = expr::analyze(ctx, value, &scope);
            expr::expect(
                ctx,
                value,
                &analysis,
                &Type::String,
                &format!("the scalar `input:` of {subject}"),
            );
        }
        (Some(NodeInput::Scalar { value }), _) => {
            form_refused = true;
            let contract_note = contract.declared().map_or_else(
                || "the target declares no input fields".to_string(),
                |fields| {
                    format!(
                        "the target declares {}",
                        crate::parse::reader::list(field_names(fields))
                    )
                },
            );
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    value.span.clone(),
                    format!("{subject} binds one unnamed value, but its target declares named input fields"),
                )
                .with_help(format!(
                    "{contract_note}: bind them by name with `input: {{ <field>: <CEL> }}`; the scalar form binds a string-in agent, which declares no `input:` at all (grammar 5.3, Decision D14)"
                )),
            );
        }
        (Some(NodeInput::Fields { bindings }), InputContract::StringIn) => {
            form_refused = true;
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    bindings.span.clone(),
                    format!("{subject} binds input fields by name, but its agent is string-in"),
                )
                .with_help(
                    "an agent that declares no `input:` takes a single unnamed string: bind it with the scalar form, `input: \"<CEL>\"` (grammar 5.3, Decision D14)",
                ),
            );
        }
        (Some(NodeInput::Fields { bindings }), contract) => {
            field_bindings(ctx, bindings, &scope, contract, &subject);
        }
        (None, _) => {}
    }

    if form_refused {
        return;
    }
    match contract {
        InputContract::Fields(fields) => {
            let bound = bound_names(node.input.as_ref());
            let module_boundary = matches!(node.kind, NodeKind::Flow { .. });
            for field in &fields.fields {
                if bound.contains(&text(&field.name)) {
                    continue;
                }
                if module_boundary {
                    // Nothing falls through a module boundary (Decision D68).
                    if default_of(&field.ty).is_none() {
                        unbound(ctx, &node.span, field, &subject, true);
                    }
                } else {
                    name_based(ctx, cx, &node.span, field, &subject);
                }
            }
        }
        InputContract::StringIn => {
            if node.input.is_none() {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::MissingBinding,
                        node.span.clone(),
                        format!("{subject} runs a string-in agent and must bind its input"),
                    )
                    .with_help(
                        "an agent that declares no `input:` takes a single unnamed string, bound at the node as `input: \"<CEL>\"` (grammar 5.3, Decision D14)",
                    ),
                );
            }
        }
        InputContract::Empty | InputContract::AdHoc | InputContract::Unknown => {}
    }
}

/// Check a field-map `input:` against the target's declared fields.
pub(crate) fn field_bindings(
    ctx: &mut Ctx,
    bindings: &Bindings,
    scope: &Scope,
    contract: InputContract<'_>,
    subject: &str,
) {
    for binding in &bindings.entries {
        let name = binding.name.value.as_str();
        let field = match contract {
            InputContract::Fields(fields) => {
                let field = fields.field(name);
                if field.is_none() {
                    unknown_field(ctx, &binding.name, fields, subject);
                }
                field
            }
            InputContract::Empty => {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownField,
                        binding.name.span.clone(),
                        format!("`{name}` is not an input field of {subject}'s target"),
                    )
                    .with_help("the target declares no input fields"),
                );
                None
            }
            // An inline node builds an ad-hoc object out of its bindings, so
            // any identifier is a legal name (grammar 8.2, 8.3).
            InputContract::AdHoc | InputContract::StringIn | InputContract::Unknown => None,
        };
        let analysis = expr::analyze(ctx, &binding.value, scope);
        if let Some(field) = field {
            expr::expect_field(
                ctx,
                &binding.value,
                &analysis,
                &field.ty,
                &format!("the binding for `{name}`"),
            );
        }
    }
}

/// Resolve an unbound field through grammar 8.0's steps 2 to 4.
fn name_based(ctx: &mut Ctx, cx: &FlowCx, at: &Span, field: &Field, subject: &str) {
    let name = text(&field.name);
    if let Some(channel) = ctx.channel(name) {
        if let Err(mismatch) = satisfies(&channel.ty, &field.ty) {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    at.clone(),
                    format!(
                        "the state channel `{name}` cannot supply `{name}` of {subject}: {}",
                        mismatch.describe()
                    ),
                )
                .with_label(channel.span.clone(), "the channel is declared here")
                .with_label(field.ty.span.clone(), "the field is declared here")
                .with_help(
                    "a name-based read puts no expression between the two declarations, so the channel must satisfy the field it lands in (grammar 8.0, Decision D111)",
                ),
            );
        }
        return;
    }
    if let Some(input) = cx
        .flow
        .inputs
        .as_ref()
        .and_then(|inputs| inputs.field(name))
    {
        if let Err(mismatch) = satisfies(&input.ty, &field.ty) {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    at.clone(),
                    format!(
                        "the flow input `{name}` cannot supply `{name}` of {subject}: {}",
                        mismatch.describe()
                    ),
                )
                .with_label(input.ty.span.clone(), "the flow input is declared here")
                .with_label(field.ty.span.clone(), "the field is declared here")
                .with_help(
                    "a name-based read puts no expression between the two declarations, so the source must satisfy the field it lands in (grammar 8.0, Decision D111)",
                ),
            );
        }
        return;
    }
    if default_of(&field.ty).is_some() {
        return;
    }
    unbound(ctx, at, field, subject, false);
}

fn unbound(ctx: &mut Ctx, at: &Span, field: &Field, subject: &str, module_boundary: bool) {
    let name = text(&field.name);
    let help = if module_boundary {
        "a module boundary binds every input explicitly: the state channel and the flow input of the same name do not cross it (grammar 8.0, Decision D68)".to_string()
    } else {
        format!(
            "bind it with `input: {{ {name}: <CEL> }}`, declare a state channel or a flow input of the same name, or give the field a `default:` (grammar 8.0)"
        )
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::MissingBinding,
            at.clone(),
            format!("`{name}` of {subject} is not bound"),
        )
        .with_label(field.name.span.clone(), "the input field is declared here")
        .with_help(help),
    );
}

pub(crate) fn unknown_field(
    ctx: &mut Ctx,
    name: &Spanned<String>,
    fields: &FieldMap,
    subject: &str,
) {
    let known = field_names(fields);
    let help = crate::parse::reader::suggest(name.value.as_str(), &known).map_or_else(
        || {
            if known.is_empty() {
                "the target declares no input fields".to_string()
            } else {
                format!(
                    "the declared fields are {}",
                    crate::parse::reader::list(&known)
                )
            }
        },
        |field| format!("did you mean `{field}`?"),
    );
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnknownField,
            name.span.clone(),
            format!(
                "`{}` is not an input field of {subject}'s target",
                name.value
            ),
        )
        .with_label(fields.span.clone(), "the input schema is declared here")
        .with_help(help),
    );
}

/// The names a node's `input:` binds, if it takes the field-map form.
fn bound_names(input: Option<&NodeInput>) -> Vec<&str> {
    match input {
        Some(NodeInput::Fields { bindings }) => bindings
            .entries
            .iter()
            .map(|binding| binding.name.value.as_str())
            .collect(),
        _ => Vec::new(),
    }
}

/// An inline `http:` node's `query:`/`body:` values, which are *flow*-scoped
/// unlike the same keys inside a `tool.*` binding (grammar 8.3, 6.1).
///
/// The values themselves are untyped: what a request parameter or a JSON body
/// member has to be is the endpoint's business, and this grammar declares
/// nothing about it. What is checked is the expression — its roots, its paths,
/// and its constructs.
pub(crate) fn inline_http<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, node: &'a Node, http: &Http) {
    let id = text(&node.id).to_string();
    let scope = expr::flow_scope(ctx, cx, format!("the `http` block of node `{id}`"));
    for bindings in [http.query.as_ref(), http.body.as_ref()]
        .into_iter()
        .flatten()
    {
        for binding in &bindings.entries {
            expr::analyze(ctx, &binding.value, &scope);
        }
    }
}

/// What an agent's two attachment lists produce together (grammar 5.4, 11.5).
///
/// Both rules here are the validator's because neither's two halves are ever in
/// one file: a `flow.*` used as a tool needs the `description:` the *flow*
/// declares, and a store's synthesized tool names are decided by the *store*'s
/// kind and `agent_access:` while the name they may not collide with is in the
/// agent's own `tools:` list.
pub(crate) fn agent_tools(ctx: &mut Ctx, address: &str, agent: &Agent) {
    for tool in &agent.tools {
        if tool.value.namespace != Namespace::Flow {
            continue;
        }
        let Some(flow) = ctx.flow(&tool.value) else {
            continue;
        };
        if flow.description.is_some() {
            continue;
        }
        let at = ctx
            .ir
            .definitions
            .get(&tool.value.to_string())
            .map_or_else(|| tool.span.clone(), |definition| definition.span.clone());
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                tool.span.clone(),
                format!(
                    "`{address}` attaches `{}` as a tool, and it declares no `description:`",
                    tool.value
                ),
            )
            .with_label(at, "the flow is defined here")
            .with_help(
                "a flow used as an agent tool takes a tool's contract: its `inputs`/`outputs` become the parameter and result schemas, and the description is what the model selects on (grammar 5.4, 7.5)",
            ),
        );
    }
    store_tool_collisions(ctx, address, agent);
}

/// An attached store synthesizes LLM-facing tools, and one of those names
/// colliding with an attached `tool.*`/`flow.*` is a compile error
/// (grammar 11.5).
///
/// An attached tool's name is its address's local name — the only name it has
/// on the model's side — so `tool.prefs_get` and the `prefs_get` a `kv`
/// `store.prefs` synthesizes are two tools with one name.
fn store_tool_collisions(ctx: &mut Ctx, address: &str, agent: &Agent) {
    if agent.tools.is_empty() || agent.stores.is_empty() {
        return;
    }
    for attached in &agent.stores {
        let Some(store) = ctx.store(&attached.value) else {
            continue;
        };
        let local = attached.value.name.as_str();
        for suffix in synthesized_tools(store) {
            let synthesized = format!("{local}_{suffix}");
            let Some(tool) = agent
                .tools
                .iter()
                .find(|tool| tool.value.name.as_str() == synthesized)
            else {
                continue;
            };
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::ToolNameCollision,
                    attached.span.clone(),
                    format!(
                        "`{address}` attaches `{}`, whose synthesized `{synthesized}` tool collides with the attached `{}`",
                        attached.value, tool.value
                    ),
                )
                .with_label(tool.span.clone(), "the colliding tool is attached here")
                .with_help(format!(
                    "a `{}` store with `agent_access: {}` synthesizes {} (grammar 11.5): rename the store, rename the tool, or drop one of the two attachments",
                    store.kind.as_str(),
                    store
                        .agent_access
                        .unwrap_or(AgentAccess::ReadWrite)
                        .as_str(),
                    crate::parse::reader::list(
                        synthesized_tools(store)
                            .iter()
                            .map(|suffix| format!("{local}_{suffix}"))
                    )
                )),
            );
        }
    }
}

/// The tool-name suffixes an attached store synthesizes, in the order grammar
/// 11.5's table lists them.
fn synthesized_tools(store: &Store) -> &'static [&'static str] {
    let read_only = store.agent_access == Some(AgentAccess::Read);
    match (store.kind, read_only) {
        (StoreKind::Kv, true) => &["get"],
        (StoreKind::Kv, false) => &["get", "set"],
        (StoreKind::Vector, true) => &["search"],
        (StoreKind::Vector, false) => &["search", "upsert"],
        (StoreKind::Blob, true) => &["get", "list"],
        (StoreKind::Blob, false) => &["get", "list", "put"],
    }
}

/// A `tool.*` implementation binding's CEL, which sees exactly one root — the
/// tool's own declared `input:` (grammar 6.1, Decision D65).
pub(crate) fn tool_implementation(ctx: &mut Ctx, address: &str, tool: &Tool) {
    let Some(http) = super::tool_http(tool) else {
        return;
    };
    let scope = expr::tool_scope(&tool.input, address);
    for bindings in [http.query.as_ref(), http.body.as_ref()]
        .into_iter()
        .flatten()
    {
        for binding in &bindings.entries {
            expr::analyze(ctx, &binding.value, &scope);
        }
    }
}
