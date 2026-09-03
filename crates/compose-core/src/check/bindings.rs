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

use crate::ast::common::{Address, Namespace};
use crate::ast::definition::{AgentAccess, ProviderKind, StoreKind};
use crate::cel::Scope;
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::binding::{Bindings, Http, NodeInput};
use crate::ir::definition::{Agent, Store, Tool};
use crate::ir::flow::{MapDispatch, Node, NodeKind, ToolImplementation};
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
    attached_tool_collisions(ctx, address, agent);
    builtin_tool_collisions(ctx, address, agent);
    store_tool_collisions(ctx, address, agent);
    server_tool_collisions(ctx, address, agent);
}

/// The name an attached `tool.*`/`flow.*` is offered to the model under.
///
/// Its address's local name, except where the tool's implementation is a
/// `builtin:` binding: those go out as the provider-defined tool types, each of
/// which carries a name the provider dictates, so `tool.sandbox` with
/// `builtin: bash` is `bash` on the wire whatever its definition key says
/// (grammar 5.4, 6.1, Decision D135). Every collision rule below compares this
/// name, because it is the only one the model ever sees.
pub(crate) fn wire_name(ctx: &Ctx, address: &Address) -> String {
    if let Some(tool) = ctx.tool(address)
        && let ToolImplementation::Builtin { builtin } = &tool.implementation
    {
        return builtin.builtin.value.as_str().to_string();
    }
    address.name.as_str().to_string()
}

/// A built-in's name is on the wire beside the agent's other tools, so a
/// `tool.*` or `flow.*` offered under `bash` or `str_replace_based_edit_tool`
/// collides with the shorthand of that name (grammar 5.5, 11.5,
/// Decision D135).
///
/// The same rule as [`attached_tool_collisions`], reached from the entry that
/// carries no address to compare: a built-in's name is fixed by this compiler
/// rather than by an author's definition key, so the repair is on the *other*
/// side — rename the definition, or drop one of the two attachments. Which entry
/// the report underlines is the shorthand's, because that is the entry a reader
/// can see the name in without opening another file.
fn builtin_tool_collisions(ctx: &mut Ctx, address: &str, agent: &Agent) {
    for builtin in &agent.builtins {
        let local = builtin.value.as_str();
        let Some(attached) = agent
            .tools
            .iter()
            .find(|tool| wire_name(ctx, &tool.value) == local)
        else {
            continue;
        };
        let attached_span = attached.span.clone();
        let attached_address = attached.value.to_string();
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::ToolNameCollision,
                builtin.span.clone(),
                format!(
                    "`{address}` attaches `{}` and `{attached_address}`, which are one `{local}` tool on the model's side",
                    builtin.value.address(),
                ),
            )
            .with_label(attached_span, "the other is attached here")
            .with_help(
                "an attached tool's name is its address's local name — or, for a `builtin:` tool, the name the provider fixes — and a shorthand built-in's is the compiler's (grammar 5.4, 5.5, 6.1): rename the definition, change what it binds, or drop one of the two attachments",
            ),
        );
    }
}

/// Two entries of one `tools:` list whose **local names** are equal are one tool
/// on the model's side, which is a compile error.
///
/// Grammar 11.5 states this rule for the pair it is easiest to hit — a
/// synthesized store tool against an attached one — and states it in exactly
/// these terms ("a synthesized tool name that collides with an attached
/// `tool.*`/`flow.*` tool name"), so the *reason* is written down: an
/// attachment's name on the wire is its address's local name, and one name is
/// one tool. What no sentence and no Decision entry says is that the same holds
/// between two **declared** attachments — `tool.condense` beside `flow.condense`
/// — and the published schema accepts it. That is a doc defect in §5.4, whose
/// duplicate rule is about entries rather than names; the check is kept
/// meanwhile because the alternative is worse than a missing sentence. Both
/// attachments are emitted (a dropped one is the silent-tool bug flow-as-tool
/// codegen exists to fix), so the request carries two tools of one name — which
/// the provider surfaces refuse outright — and if one were dropped instead, the
/// model would be given a contract the composition did not attach.
fn attached_tool_collisions(ctx: &mut Ctx, address: &str, agent: &Agent) {
    for (position, attached) in agent.tools.iter().enumerate() {
        let local = wire_name(ctx, &attached.value);
        let Some(first) = agent.tools[..position]
            .iter()
            .find(|earlier| wire_name(ctx, &earlier.value) == local)
        else {
            continue;
        };
        let (first_address, first_span) = (first.value.to_string(), first.span.clone());
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::ToolNameCollision,
                attached.span.clone(),
                format!(
                    "`{address}` attaches `{first_address}` and `{}`, which are one `{local}` tool on the model's side",
                    attached.value
                ),
            )
            .with_label(first_span, "the first is attached here")
            .with_help(
                "an attached tool's name is its address's local name — or, for a `builtin:` tool, the name the provider fixes — and it is the only name it has on the model's side (grammar 5.4, 6.1, 11.5): rename one of the two definitions, or drop one of the attachments",
            ),
        );
    }
}

/// An attached store synthesizes LLM-facing tools, and one of those names
/// colliding with an attached `tool.*`/`flow.*` is a compile error
/// (grammar 11.5).
///
/// An attached tool's name is its address's local name — the only name it has
/// on the model's side — so `tool.prefs_get` and the `prefs_get` a `kv`
/// `store.prefs` synthesizes are two tools with one name.
///
/// The **built-ins** are deliberately not compared here, and that is a fact
/// about the two name shapes rather than an omission: a synthesized name is
/// `<store's local name>_<op>` over grammar 11.4's closed op list, and none of
/// `bash`, `read_file`, `write_file` or `list` has that shape for a non-empty
/// local name. `the_builtin_names_are_not_shapes_a_store_can_synthesize` is that
/// premise, held where it can fail if either list moves (grammar 5.5, 11.5).
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
                .find(|tool| wire_name(ctx, &tool.value) == synthesized)
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

/// A tool the agent's **connection** offers may not take a name the agent's own
/// tools take (grammar 12.1, 11.5, Decision D122).
///
/// A provider's `server_tools:` suite is appended to the `tools` of every
/// request that connection serves, after the agent's own — one array, one
/// namespace. So `tool.web_search` on an agent whose provider declares
/// `web_search_20250305` is the rule §11.5 already states, reached from the
/// other side: the model is offered two different things under one name, and
/// the Messages API answers such a request 400.
///
/// **The Messages wire only**, and that is the rule rather than a gap — which
/// is why the loop below is keyed on [`ProviderKind::Anthropic`] rather than on
/// whether a name is knowable. It is the one wire where a server tool and a
/// client tool sit under the same key: a Responses built-in is addressed by its
/// `type:` while a function tool carries a `name:`, two different keys that
/// cannot collide. Chat Completions is the same shape one level down — a
/// function tool's name lives at `tools[i].function.name` while a suite entry's
/// `name:` is the entry's own key — and above that, no table could say what a
/// gateway keys its vocabulary on, so `openai_compatible` is out for both
/// reasons at once (`check::providers`'s `Slot`, grammar 12.1, D122).
///
/// A suite entry's `name:` is still compared against the **rest of its own
/// suite** on every kind, which is a different claim and stays where it is
/// (`check::providers`'s `suite_collisions`): two entries of one array under one
/// key are two tools under one identity by the author's own reckoning, whatever
/// the wire keys on.
///
/// Which providers the agent might reach is the ladder's whole width — a route's
/// members each declare their own suite, and any of them may serve the call.
fn server_tool_collisions(ctx: &mut Ctx, address: &str, agent: &Agent) {
    /// One name this agent puts on the wire, and how a report talks about it.
    struct Offered {
        /// The name the model is offered it under.
        local: String,
        /// The entry that offers it — what the report underlines.
        at: Span,
        /// What the agent did, as the message's middle clause.
        subject: String,
        /// The repair on *this* side of the pair. It differs by kind: an
        /// attachment can be renamed by renaming its definition, and a built-in
        /// cannot — its name is the compiler's — so the only move left there is
        /// to drop the entry (PRD G3).
        repair: &'static str,
    }

    let mut offered: Vec<Offered> = agent
        .tools
        .iter()
        .map(|tool| Offered {
            local: wire_name(ctx, &tool.value),
            at: tool.span.clone(),
            subject: format!("attaches `{}`, whose name collides with", tool.value),
            repair: "rename the attachment",
        })
        .collect();
    // A built-in reaches the same `tools` array under the same key, so a suite
    // declaring `bash` and an agent holding `builtin.bash` is the same 400 as
    // any other pair (grammar 5.5, Decision D135).
    for builtin in &agent.builtins {
        offered.push(Offered {
            local: builtin.value.as_str().to_string(),
            at: builtin.span.clone(),
            subject: format!(
                "attaches `{}`, whose name collides with",
                builtin.value.address()
            ),
            repair: "drop the built-in, whose name is not an author's to change",
        });
    }
    for attached in &agent.stores {
        let Some(store) = ctx.store(&attached.value) else {
            continue;
        };
        let local = attached.value.name.as_str();
        for suffix in synthesized_tools(store) {
            let synthesized = format!("{local}_{suffix}");
            offered.push(Offered {
                local: synthesized.clone(),
                at: attached.span.clone(),
                subject: format!(
                    "attaches `{}`, whose synthesized `{synthesized}` tool collides with",
                    attached.value
                ),
                repair: "rename the store",
            });
        }
    }
    if offered.is_empty() {
        return;
    }
    let providers = super::providers::providers_of(ctx, &agent.model.value);
    for (provider_address, provider) in providers {
        if provider.kind != ProviderKind::Anthropic {
            continue;
        }
        for server in &provider.config.server_tools {
            let Some(name) = super::providers::wire_name(provider.kind, server) else {
                continue;
            };
            for entry in offered.iter().filter(|entry| entry.local == name) {
                let local = &entry.local;
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::ToolNameCollision,
                        entry.at.clone(),
                        format!(
                            "`{address}` {} the `{local}` server tool `{provider_address}` declares",
                            entry.subject
                        ),
                    )
                    .with_label(server.span.clone(), "the server tool is declared here")
                    .with_help(format!(
                        "a server tool is appended to the `tools` of every request that \
                         connection serves, and the wire refuses an array carrying `{local}` \
                         twice: {}, or declare the suite on a provider this agent's model does \
                         not reach (grammar 12.1, 11.5, Decision D122)",
                        entry.repair
                    )),
                );
            }
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

/// The **one** call site a built-in has: an agent's `tools:` list (grammar 5.5,
/// 6.1, Decision D135).
///
/// A `tool.*` binding a built-in is a tool by address like any other, so a
/// `function:` node and a `map` dispatch can both name one — and neither can
/// call it. The reason is the same one that makes `input:`/`output:` compile
/// errors on such a tool: those two surfaces pass a **composition's** arguments
/// and read a declared result, and a built-in has neither. The model writes the
/// program, and only a model's loop has one to write.
///
/// Without this the mistake reaches the emitter, which skips a built-in when it
/// writes the project's tool functions and then emits a node calling the
/// function it did not write — a project that does not type-check, from a spec
/// `validate` accepted. The invariant `codegen::graph` states is enforced here.
pub(crate) fn builtin_node_targets<'a>(ctx: &mut Ctx<'a>, node: &'a Node) {
    let id = text(&node.id).to_string();
    match &node.kind {
        NodeKind::Function { function } => {
            builtin_call_site(ctx, &format!("node `{id}`"), function);
        }
        NodeKind::Map { map } => {
            let subject = format!("the `map` of node `{id}`");
            match &map.dispatch {
                MapDispatch::Homogeneous { node: target, .. } => {
                    builtin_call_site(ctx, &subject, target);
                }
                MapDispatch::Routed {
                    routes, default, ..
                } => {
                    for route in routes.iter().chain(default.as_deref()) {
                        builtin_call_site(ctx, &subject, &route.node);
                    }
                }
            }
        }
        _ => {}
    }
}

/// One node target, refused where the tool it names binds a built-in.
fn builtin_call_site(ctx: &mut Ctx, subject: &str, target: &Spanned<Address>) {
    let Some(tool) = ctx.tool(&target.value) else {
        return;
    };
    let ToolImplementation::Builtin { builtin } = &tool.implementation else {
        return;
    };
    let name = builtin.builtin.value;
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            target.span.clone(),
            format!(
                "{subject} invokes `{}`, which binds the built-in `{}`",
                target.value,
                name.keyword()
            ),
        )
        .with_label(builtin.builtin.span.clone(), "the built-in is bound here")
        .with_help(format!(
            "a built-in hands the **model** the program, so an agent's `tools:` list is its only \
             call site: it declares no `input:` for a node to bind and no `output:` for one to \
             read (grammar 5.5, 6.1, Decision D135). Attach `{}` to an agent and let the agent \
             call it, or bind this tool with `exec:` to run a command this composition chose",
            target.value
        )),
    );
}

#[cfg(test)]
mod tests {
    /// Every code one composition reports, in the order the report is sorted
    /// into. The `version:` line is prepended so a case is only its own shape.
    fn codes(body: &str) -> Vec<String> {
        let ir = crate::codegen::test_support::ir_of(&format!("version: \"0.1\"\n{body}"));
        crate::check::check(&ir)
            .iter()
            .map(|diagnostic| diagnostic.code.to_string())
            .collect()
    }

    /// A route's members each declare their own suite, so the names an agent's
    /// tools may not take are the union over the whole ladder — not the first
    /// member's alone.
    ///
    /// The failure this pins is the least observable one there is: a
    /// composition that runs for months on its primary and 400s the first time
    /// the fallback answers.
    #[test]
    fn a_suite_a_later_ladder_member_declares_is_read_too() {
        assert_eq!(
            codes(
                r#"
provider.primary:
  kind: anthropic
  api_key: ${K}
provider.fallback:
  kind: anthropic
  api_key: ${K2}
  server_tools:
    - type: web_search_20250305
      name: web_search
model.primary:
  provider: provider.primary
  id: some-model
model.fallback:
  provider: provider.fallback
  id: another-model
model.m:
  route: [model.primary, model.fallback]
tool.web_search:
  description: Search the web the long way round.
  input:
    query: { type: string }
  output:
    value: { type: string }
  exec:
    command: search-the-web
agent.a:
  model: model.m
  prompt: Decide.
  tools: [tool.web_search]
  output:
    verdict: { type: string }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#
            ),
            ["mismatched-server-tools", "tool-name-collision"],
            "the second member's suite is the one that collides, and the members \
             declaring different suites at all is the other half of the same shape"
        );
    }

    /// The rule is the Messages wire's, and is stated over it rather than left
    /// unstated: a Responses built-in is addressed by its `type:` while a
    /// function tool carries a `name:`, two keys that cannot collide, so an
    /// `openai` connection declaring `web_search` beside a `tool.web_search` is
    /// a composition this release does not refuse.
    #[test]
    fn the_responses_wire_addresses_its_two_kinds_of_tool_by_different_keys() {
        assert_eq!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: web_search
model.m:
  provider: provider.o
  id: gpt-5
tool.web_search:
  description: Search the web the long way round.
  input:
    query: { type: string }
  output:
    value: { type: string }
  exec:
    command: search-the-web
agent.a:
  model: model.m
  prompt: Decide.
  tools: [tool.web_search]
  output:
    verdict: { type: string }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#
            ),
            Vec::<String>::new()
        );
    }

    /// …and the same over a **gateway**, which is the other half of the same
    /// sentence and the one a compiler is likeliest to get wrong by accident.
    ///
    /// `openai_compatible` is off this rule for two reasons at once, either of
    /// which is enough. No table could say what key a gateway addresses its
    /// vocabulary on — that is the whole reason every entry on the kind is
    /// second-tier — and the surface it rides is Chat Completions, where a
    /// client tool's name lives at `tools[i].function.name` while a suite entry
    /// carries its `name:` at the top of its own object: two keys, not one.
    ///
    /// The failure this pins is a legal composition refused with no workaround
    /// but renaming, against a published grammar (§12.1, D122) that tells the
    /// author it compiles. The entry's `unknown-server-tool` warning is the
    /// whole of what this release has to say about it.
    #[test]
    fn a_gateways_suite_is_not_compared_with_the_agents_own_tools() {
        assert_eq!(
            codes(
                r#"
provider.g:
  kind: openai_compatible
  base_url: ${GATEWAY_URL}
  server_tools:
    - type: retrieval
      name: search_docs
model.m:
  provider: provider.g
  id: qwen3-coder-30b
tool.search_docs:
  description: Search the docs the long way round.
  input:
    query: { type: string }
  output:
    value: { type: string }
  exec:
    command: search-the-docs
agent.a:
  model: model.m
  prompt: Decide.
  tools: [tool.search_docs]
  output:
    verdict: { type: string }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#
            ),
            ["unknown-server-tool"],
            "a gateway's suite is carried unchecked, and that includes not \
             deciding whose name space its `name:` lands in"
        );
    }

    /// A **store**'s synthesized tools reach the same rule by the same route, so
    /// the wire gate has to hold for them too: an `openai_compatible` connection
    /// whose suite names `notes_get` beside an attached `store.notes` is the
    /// same legal composition as the one above, spelled with the name the
    /// runtime makes rather than one the author wrote.
    #[test]
    fn a_gateways_suite_is_not_compared_with_a_synthesized_store_tool_either() {
        assert_eq!(
            codes(
                r#"
provider.g:
  kind: openai_compatible
  base_url: ${GATEWAY_URL}
  server_tools:
    - type: retrieval
      name: notes_get
model.m:
  provider: provider.g
  id: qwen3-coder-30b
store.notes:
  kind: kv
  scope: execution
  description: What the run has been told.
  agent_access: read
  value_schema:
    theme: { type: string }
agent.a:
  model: model.m
  prompt: Decide.
  stores: [store.notes]
  output:
    verdict: { type: string }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#
            ),
            ["unknown-server-tool"]
        );
    }

    /// No store can synthesize a tool called `bash` or
    /// `str_replace_based_edit_tool`, which is why
    /// [`super::store_tool_collisions`] does not compare the built-ins.
    ///
    /// The premise is about two closed lists — the built-in names and grammar
    /// 11.4's ops — so it is held here rather than asserted in a comment: a
    /// built-in whose name were `notes_get`, or an op called `file`, would make
    /// the collision writable, and this test is what would say so.
    #[test]
    fn the_builtin_names_are_not_shapes_a_store_can_synthesize() {
        use crate::ast::definition::{Builtin, StoreKind};
        for builtin in Builtin::ALL {
            for kind in StoreKind::ALL {
                for op in kind.operations() {
                    let suffix = format!("_{op}");
                    assert!(
                        !builtin.as_str().ends_with(&suffix)
                            || builtin.as_str().len() == suffix.len(),
                        "`{}` is `<local>_{op}` for a non-empty local name, so a store could \
                         synthesize it",
                        builtin.as_str()
                    );
                }
            }
        }
    }

    /// A **routed** `map` is the third way a node names a tool, and the one the
    /// negative corpus does not reach.
    ///
    /// Two fixtures pin the other two spellings against their exact messages —
    /// `a-builtin-tool-called-from-a-function-node` and
    /// `a-builtin-tool-dispatched-to-by-a-map` — and both go through one target
    /// address. A `routes:` ladder is a *list* of them, walked separately
    /// ([`super::builtin_node_targets`]), so a rule kept at the homogeneous
    /// dispatch is not thereby kept at a route: this is the arm where a built-in
    /// sits beside targets that are perfectly legal, which is also the shape an
    /// author most plausibly writes it in.
    ///
    /// Held here rather than as a fixture because what it is evidence for is the
    /// **arm**, not the wording: the message is the fixtures', and a routed
    /// composition minimal enough to fail nothing else needs a union, a
    /// discriminator and a second route to carry it.
    #[test]
    fn a_route_of_a_map_dispatching_to_a_builtin_is_refused_at_that_route() {
        assert_eq!(
            codes(
                r#"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.triage:
  model: model.m
  prompt: Triage.
  input:
    text: { type: string }
  output:
    jobs:
      type: array
      max_items: 10
      items:
        discriminator: kind
        variants:
          shell:
            command: { type: string }
          human:
            summary: { type: string }
agent.reviewer:
  model: model.m
  prompt: Review.
  input:
    summary: { type: string }
  output:
    ticket: { type: string }
tool.sandbox:
  builtin: bash
  workspace: ./work
flow.f:
  outputs: {}
  nodes:
    classify:
      agent: agent.triage
      input: { text: "'x'" }
    dispatch:
      map:
        over: "classify.output.jobs"
        as: job
        route_by: kind
        max_concurrency: 4
        routes:
          shell:
            node: tool.sandbox
            input: { command: "job.command" }
          human:
            node: agent.reviewer
            input: { summary: "job.summary" }
  edges:
    - { from: start, to: classify }
    - { from: classify, to: dispatch }
    - { from: dispatch, to: end }
"#
            ),
            ["invalid-value"],
            "the route names a built-in, which no node may call however it names it — and the \
             route beside it, which names an agent, is refused nothing"
        );
    }
}
