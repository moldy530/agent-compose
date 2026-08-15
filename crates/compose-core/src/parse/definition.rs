//! The six definition namespaces (grammar 5, 6, 7, 11, 12).

use crate::ast::common::{Address, Namespace};
use crate::ast::definition::{
    AgentAccess, DefinitionBody, DirectModel, EmbedBlock, ModelDef, ProviderDef, ProviderKind,
    RouteCondition, RouteModel, Settings, StoreDef, StoreKind, StoreScope, ToolDef,
    ToolImplementation,
};
use crate::ast::schema::{FieldMap, Surface};
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::yaml::Node;

use super::binding;
use super::flow;
use super::lexical;
use super::reader::{Cx, Fields, expect_mapping, expect_sequence, list, suggest};
use super::schema;
use super::schema::literal;

/// Read a definition body, dispatching on the namespace its key named.
pub(crate) fn definition_body(
    address: &Spanned<Address>,
    node: &Node,
    cx: &mut Cx,
) -> Option<DefinitionBody> {
    let kind = match address.value.namespace {
        Namespace::Agent => "agent",
        Namespace::Tool => "tool",
        Namespace::Flow => "flow",
        Namespace::Store => "store",
        Namespace::Provider => "provider",
        Namespace::Model => "model",
    };
    let subject = format!("{kind} definition `{}`", address.value);
    let mapping = expect_mapping(node, &subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), &subject);

    let body = match address.value.namespace {
        Namespace::Agent => DefinitionBody::Agent(agent(&mut fields, &subject, cx)),
        Namespace::Tool => DefinitionBody::Tool(tool(&mut fields, &subject, cx)),
        Namespace::Flow => DefinitionBody::Flow(flow::flow_def(&mut fields, &subject, cx)),
        Namespace::Store => DefinitionBody::Store(store(&mut fields, &subject, cx)),
        Namespace::Provider => DefinitionBody::Provider(provider(&mut fields, &subject, cx)),
        Namespace::Model => DefinitionBody::Model(model(&mut fields, &subject, cx)),
    };
    fields.finish(cx);
    Some(body)
}

fn agent(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> crate::ast::definition::AgentDef {
    let model = fields
        .require("model", cx)
        .and_then(|node| lexical::reference(node, "`model`", &[Namespace::Model], cx));
    let prompt = fields
        .require("prompt", cx)
        .and_then(|node| lexical::non_empty_text(node, "`prompt`", cx));
    let output = fields.require("output", cx).and_then(|node| {
        let map = schema::field_map(node, &format!("`output` of {subject}"), Surface::Result, cx)?;
        if map.is_empty() {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    map.span.clone(),
                    format!("`output` of {subject} must declare at least one property"),
                )
                .with_help(
                    "an agent's structured output is what routing reads and what makes its edges serializable (grammar 5.1)",
                ),
            );
        }
        Some(map)
    });
    let input = fields.take("input").and_then(|node| {
        let map = schema::field_map(node, &format!("`input` of {subject}"), Surface::Input, cx)?;
        if map.is_empty() {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    map.span.clone(),
                    format!("`input` of {subject} must declare at least one field"),
                )
                .with_help(
                    "omitting `input:` is how an agent takes a single unnamed string; `{}` is neither contract (grammar 5.3)",
                ),
            );
        }
        Some(map)
    });
    let tools = reference_list(
        fields,
        "tools",
        &[Namespace::Tool, Namespace::Flow],
        subject,
        cx,
    )
    .values;
    let stores = reference_list(fields, "stores", &[Namespace::Store], subject, cx).values;
    let description = description(fields, cx);
    let max_tool_iterations = fields
        .integer("max_tool_iterations", cx)
        .filter(|value| super::reader::in_range(value, "`max_tool_iterations`", 1..=50, cx));

    crate::ast::definition::AgentDef {
        model,
        prompt,
        output,
        input,
        tools,
        stores,
        description,
        max_tool_iterations,
    }
}

fn tool(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> ToolDef {
    let description = fields
        .require("description", cx)
        .and_then(|node| lexical::non_empty_text(node, "`description`", cx));
    let input = fields.require("input", cx).and_then(|node| {
        schema::field_map(node, &format!("`input` of {subject}"), Surface::Input, cx)
    });
    let output = fields.require("output", cx).and_then(|node| {
        schema::field_map(node, &format!("`output` of {subject}"), Surface::Result, cx)
    });

    const BINDINGS: &[&str] = &["exec", "http", "function"];
    fields.note_known(BINDINGS);
    let declared: Vec<&str> = BINDINGS
        .iter()
        .copied()
        .filter(|key| fields.contains(key))
        .collect();

    let implementation = match declared.as_slice() {
        [] => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingKey,
                    fields.span.clone(),
                    format!(
                        "{subject} declares no implementation: a tool carries exactly one of {}",
                        list(BINDINGS)
                    ),
                )
                .with_help("`exec` runs a subprocess, `http` calls an endpoint, `function` names a host-registered function"),
            );
            None
        }
        [single] => implementation(fields, single, subject, cx),
        [first, rest @ ..] => {
            for key in rest {
                let span = fields
                    .take_entry(key)
                    .map_or_else(|| fields.span.clone(), |entry| entry.key.span.clone());
                cx.error(
                    DiagnosticCode::ConflictingKeys,
                    &span,
                    format!(
                        "{subject} declares both `{first}` and `{key}`; a tool carries exactly one implementation"
                    ),
                );
            }
            implementation(fields, first, subject, cx)
        }
    };

    // The bound input object arrives as environment variables, so an `env:` key
    // of the same upper-snake-cased name would silently win (Decision D66).
    if let (Some(ToolImplementation::Exec(exec)), Some(input)) =
        (implementation.as_ref(), input.as_ref())
    {
        let names: Vec<Spanned<String>> = input
            .fields
            .iter()
            .map(|field| {
                Spanned::new(
                    field.name.value.as_str().to_owned(),
                    field.name.span.clone(),
                )
            })
            .collect();
        binding::reject_env_collisions(exec, names.iter(), cx);
    }

    ToolDef {
        description,
        input,
        output,
        implementation,
    }
}

fn implementation(
    fields: &mut Fields<'_>,
    key: &str,
    subject: &str,
    cx: &mut Cx,
) -> Option<ToolImplementation> {
    let node = match key {
        "exec" => fields.take_entry("exec"),
        "http" => fields.take_entry("http"),
        _ => fields.take_entry("function"),
    }?;
    let context = format!("the `{key}` binding of {subject}");
    match key {
        "exec" => {
            binding::exec_block(&node.value, &context, false, cx).map(ToolImplementation::Exec)
        }
        "http" => {
            binding::http_block(&node.value, &context, false, cx).map(ToolImplementation::Http)
        }
        _ => binding::function_binding(&node.value, &context, cx).map(ToolImplementation::Function),
    }
}

const STORE_KINDS: &[(&str, StoreKind)] = &[
    ("kv", StoreKind::Kv),
    ("vector", StoreKind::Vector),
    ("blob", StoreKind::Blob),
];

const STORE_SCOPES: &[(&str, StoreScope)] = &[
    ("execution", StoreScope::Execution),
    ("session", StoreScope::Session),
    ("global", StoreScope::Global),
];

const AGENT_ACCESS: &[(&str, AgentAccess)] = &[
    ("read", AgentAccess::Read),
    ("read_write", AgentAccess::ReadWrite),
];

fn store(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> StoreDef {
    let kind = fields
        .require("kind", cx)
        .and_then(|node| lexical::keyword(node, "store `kind`", STORE_KINDS, cx));
    let scope = fields
        .require("scope", cx)
        .and_then(|node| lexical::keyword(node, "store `scope`", STORE_SCOPES, cx));

    let value_schema = schema_key(fields, "value_schema", subject, cx);
    let metadata_schema = schema_key(fields, "metadata_schema", subject, cx);
    let embed = fields
        .take("embed")
        .and_then(|node| embed_block(node, subject, cx));

    if let Some(kind) = kind.as_ref() {
        let (required, illegal): (&[&str], &[&str]) = match kind.value {
            StoreKind::Kv => (&["value_schema"], &["metadata_schema", "embed"]),
            StoreKind::Vector => (&["embed"], &["value_schema"]),
            StoreKind::Blob => (&[], &["value_schema", "embed"]),
        };
        for key in required {
            if !fields.contains(key) {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::MissingKey,
                        fields.span.clone(),
                        format!(
                            "missing required key `{key}` in {subject}: a `{}` store declares it",
                            kind.value.as_str()
                        ),
                    )
                    .with_label(kind.span.clone(), "the kind is declared here"),
                );
            }
        }
        for key in illegal {
            let Some(entry) = fields.take_entry(key) else {
                continue;
            };
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    entry.key.span.clone(),
                    format!("`{key}` is not legal on a `{}` store", kind.value.as_str()),
                )
                .with_label(kind.span.clone(), "the kind is declared here"),
            );
        }
    }

    let backend = fields
        .string("backend", cx)
        .and_then(|text| lexical::identifier(&text, "backend alias", cx));
    let description = description(fields, cx);
    let agent_access = fields
        .take("agent_access")
        .and_then(|node| lexical::keyword(node, "`agent_access`", AGENT_ACCESS, cx));

    StoreDef {
        kind,
        scope,
        value_schema,
        metadata_schema,
        embed,
        backend,
        description,
        agent_access,
    }
}

fn schema_key(
    fields: &mut Fields<'_>,
    key: &'static str,
    subject: &str,
    cx: &mut Cx,
) -> Option<FieldMap> {
    let node = fields.take(key)?;
    schema::field_map(node, &format!("`{key}` of {subject}"), Surface::Result, cx)
}

fn embed_block(node: &Node, subject: &str, cx: &mut Cx) -> Option<EmbedBlock> {
    let context = format!("the `embed` block of {subject}");
    let mapping = expect_mapping(node, &context, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), &context);
    let model = fields
        .require("model", cx)
        .and_then(|node| lexical::non_empty_text(node, "`model`", cx));
    let provider = fields
        .take("provider")
        .and_then(|node| lexical::reference(node, "`provider`", &[Namespace::Provider], cx));
    let dimensions = fields
        .integer("dimensions", cx)
        .filter(|value| super::reader::in_range(value, "`dimensions`", 1..=i64::MAX, cx));
    fields.finish(cx);
    Some(EmbedBlock {
        model,
        provider,
        dimensions,
        span: node.span.clone(),
    })
}

const PROVIDER_KINDS: &[(&str, ProviderKind)] = &[
    ("anthropic", ProviderKind::Anthropic),
    ("openai", ProviderKind::OpenAi),
    ("openai_compatible", ProviderKind::OpenAiCompatible),
    ("azure_openai", ProviderKind::AzureOpenAi),
    ("bedrock", ProviderKind::Bedrock),
    ("vertex", ProviderKind::Vertex),
];

fn provider(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> ProviderDef {
    let kind = fields
        .require("kind", cx)
        .and_then(|node| lexical::keyword(node, "provider `kind`", PROVIDER_KINDS, cx));

    let api_key = secret_field(fields, "api_key", cx);
    let base_url = secret_field(fields, "base_url", cx);
    let access_key_id = secret_field(fields, "access_key_id", cx);
    let secret_access_key = secret_field(fields, "secret_access_key", cx);
    let session_token = secret_field(fields, "session_token", cx);
    let credentials_json = secret_field(fields, "credentials_json", cx);

    let api_version = plain_field(fields, "api_version", cx);
    let organization = plain_field(fields, "organization", cx);
    let region = plain_field(fields, "region", cx);
    let location = plain_field(fields, "location", cx);
    let project = plain_field(fields, "project", cx);
    let profile = plain_field(fields, "profile", cx);

    let headers = fields
        .take("headers")
        .map(|node| binding::interpolated_map(node, "`headers`", binding::NameForm::HeaderLike, cx))
        .unwrap_or_default();
    let description = description(fields, cx);

    if let Some(kind) = kind.as_ref() {
        for key in kind.value.required_keys() {
            if !fields.contains(key) {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::MissingKey,
                        fields.span.clone(),
                        format!(
                            "missing required key `{key}` in {subject}: `{}` providers declare it",
                            kind.value.as_str()
                        ),
                    )
                    .with_label(kind.span.clone(), "the kind is declared here"),
                );
            }
        }
    }

    ProviderDef {
        kind,
        api_key,
        base_url,
        access_key_id,
        secret_access_key,
        session_token,
        credentials_json,
        api_version,
        organization,
        region,
        location,
        project,
        profile,
        headers,
        description,
    }
}

/// A provider key that must hold an `${ENV}` value-form reference (grammar 4.3).
fn secret_field(
    fields: &mut Fields<'_>,
    key: &'static str,
    cx: &mut Cx,
) -> Option<Spanned<crate::ast::common::EnvRef>> {
    fields
        .take(key)
        .and_then(|node| lexical::env_ref(node, &format!("`{key}`"), cx))
}

/// A provider key that is a plain string and may interpolate (grammar 12.1).
fn plain_field(
    fields: &mut Fields<'_>,
    key: &'static str,
    cx: &mut Cx,
) -> Option<Spanned<crate::ast::common::Interpolated>> {
    fields
        .take(key)
        .and_then(|node| lexical::interpolated(node, &format!("`{key}`"), cx))
}

const ROUTE_CONDITIONS: &[(&str, RouteCondition)] = &[
    ("rate_limit", RouteCondition::RateLimit),
    ("overloaded", RouteCondition::Overloaded),
    ("timeout", RouteCondition::Timeout),
    ("server_error", RouteCondition::ServerError),
];

fn model(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> ModelDef {
    if fields.contains("route") {
        let route = reference_list(fields, "route", &[Namespace::Model], subject, cx);
        // A route is an ordered fallback of at least two models (grammar 12.2):
        // one member has nothing to fail over to, and none is not a binding at
        // all.
        if let Some(declared) = route.declared
            && declared < 2
        {
            let span = route.span.clone().unwrap_or_else(|| fields.span.clone());
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    span,
                    format!(
                        "`route` of {subject} declares {declared} member{}; a route needs at least 2",
                        if declared == 1 { "" } else { "s" }
                    ),
                )
                .with_help(if declared == 1 {
                    "a one-member route is a direct model: write `provider:` and `id:` instead"
                } else {
                    "list the models to fail over to, in order, or write `provider:` and `id:` for a direct model"
                }),
            );
        }
        let route = route.values;
        let route_on = fields.take("route_on").map(|node| {
            let mut conditions: Vec<Spanned<RouteCondition>> = Vec::new();
            if let Some(items) = expect_sequence(node, "`route_on`", cx) {
                if items.is_empty() {
                    cx.error(
                        DiagnosticCode::InvalidValue,
                        &node.span,
                        "`route_on` must declare at least one condition",
                    );
                }
                for item in items {
                    let Some(condition) =
                        lexical::keyword(item, "route condition", ROUTE_CONDITIONS, cx)
                    else {
                        continue;
                    };
                    if let Some(first) = conditions
                        .iter()
                        .find(|other| other.value == condition.value)
                    {
                        cx.push(
                            Diagnostic::error(
                                DiagnosticCode::InvalidValue,
                                condition.span.clone(),
                                format!(
                                    "duplicate `route_on` condition `{}`",
                                    condition.value.as_str()
                                ),
                            )
                            .with_label(first.span.clone(), "first declared here"),
                        );
                        continue;
                    }
                    conditions.push(condition);
                }
            }
            conditions
        });
        let description = description(fields, cx);

        for key in ["provider", "id", "settings"] {
            let Some(entry) = fields.take_entry(key) else {
                continue;
            };
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    entry.key.span.clone(),
                    format!("`{key}` is not legal on a model route"),
                )
                .with_help(
                    "a route is an ordered list of direct models; `provider`, `id`, and `settings` belong to its members (grammar 12.2)",
                ),
            );
        }

        return ModelDef::Route(RouteModel {
            route,
            route_on,
            description,
        });
    }

    let provider = fields
        .require("provider", cx)
        .and_then(|node| lexical::reference(node, "`provider`", &[Namespace::Provider], cx));
    let id = fields
        .require("id", cx)
        .and_then(|node| lexical::non_empty_text(node, "model `id`", cx));
    let settings = fields.take("settings").and_then(|node| {
        let mapping = expect_mapping(node, "`settings`", cx)?;
        Some(Settings {
            entries: mapping
                .entries()
                .iter()
                .map(|entry| crate::ast::common::LiteralEntry {
                    key: entry.key.clone(),
                    value: literal(&entry.value),
                })
                .collect(),
            span: node.span.clone(),
        })
    });
    let description = description(fields, cx);

    ModelDef::Direct(DirectModel {
        provider,
        id,
        settings,
        description,
    })
}

/// Read `description:`, legal on every definition and node (Decision D54).
pub(crate) fn description(fields: &mut Fields<'_>, cx: &mut Cx) -> Option<Spanned<String>> {
    fields
        .take("description")
        .and_then(|node| lexical::text(node, "`description`", cx))
}

/// A `<key>:` list of typed addresses, as read.
struct ReferenceList {
    /// The distinct references, in declaration order.
    values: Vec<Spanned<Address>>,
    /// How many entries the author wrote, duplicates and unreadable entries
    /// included. An arity rule is about the source text, so it counts these
    /// rather than [`Self::values`]: `route: [model.a, model.a]` declares two
    /// members and one of them is a duplicate, not a route with one member.
    ///
    /// `None` when the key was absent or held something that is not a sequence
    /// at all — there is no count to reason about, and the shape has already
    /// been reported.
    declared: Option<usize>,
    /// The sequence's own span, absent when the key was not declared at all.
    span: Option<Span>,
}

/// Read a sequence of typed addresses, rejecting duplicates (grammar 5.4).
fn reference_list(
    fields: &mut Fields<'_>,
    key: &'static str,
    allowed: &[Namespace],
    subject: &str,
    cx: &mut Cx,
) -> ReferenceList {
    let mut list = ReferenceList {
        values: Vec::new(),
        declared: None,
        span: None,
    };
    let Some(node) = fields.take(key) else {
        return list;
    };
    list.span = Some(node.span.clone());
    let Some(items) = expect_sequence(node, &format!("`{key}` in {subject}"), cx) else {
        return list;
    };
    list.declared = Some(items.len());
    for item in items {
        let Some(reference) =
            lexical::reference(item, &format!("each entry of `{key}`"), allowed, cx)
        else {
            continue;
        };
        if let Some(first) = list
            .values
            .iter()
            .find(|other| other.value == reference.value)
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    reference.span.clone(),
                    format!("`{key}` lists `{}` twice", reference.value),
                )
                .with_label(first.span.clone(), "first listed here"),
            );
            continue;
        }
        list.values.push(reference);
    }
    list
}

/// Suggest a definition namespace for a top-level key that looks like one.
pub(crate) fn suggest_namespace(prefix: &str) -> Option<&'static str> {
    let names: Vec<&str> = Namespace::ALL.iter().map(|n| n.as_str()).collect();
    suggest(prefix, &names)
}
