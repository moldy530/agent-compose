//! The six definition namespaces (grammar 5, 6, 7, 11, 12).

use crate::ast::common::{Address, Namespace};
use crate::ast::definition::{
    AgentAccess, Builtin, BuiltinAttachment, DefinitionBody, DirectModel, EmbedBlock, ModelDef,
    ProviderDef, ProviderKind, RouteCondition, RouteModel, Settings, StoreDef, StoreKind,
    StoreScope, ToolDef, ToolImplementation,
};
use crate::ast::schema::{FieldMap, Surface};
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::yaml::{Node, Yaml};

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
    // A body that is not a mapping has no key to read, and the wrong-type
    // diagnostic `expect_mapping` just pushed says so. The *address* is still
    // one the author declared, so the definition is kept with an `Invalid` body
    // rather than dropped: the resolver's index is a table of names, and a name
    // missing from it makes every reference to it undefined — one diagnostic
    // per reference site, for the one mistake already reported here.
    let Some(mapping) = expect_mapping(node, &subject, cx) else {
        return Some(DefinitionBody::Invalid);
    };
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
    let (tools, builtins) = agent_tools(fields, subject, cx);
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
        builtins,
        stores,
        description,
        max_tool_iterations,
    }
}

/// Read an agent's `tools:` list, which carries two kinds of entry (grammar 5.4,
/// 5.5, Decision D123).
///
/// A **scalar** entry is a `tool.*` or `flow.*` address, exactly as it always
/// was. A **mapping** entry attaches one of the four runtime built-ins, under
/// its own name, with the bounds that name requires beside it:
///
/// ```yaml
/// tools:
///   - tool.repo_grep
///   - builtin.read_file: { root: "${WORKSPACE}" }
///   - builtin.bash:      { root: "${WORKSPACE}", timeout: 30s }
/// ```
///
/// One entry attaches one tool, which is why a mapping carrying two keys is
/// refused rather than read as two attachments: PRD resolved q31 makes the
/// opt-in "one tool name at a time … never a single switch that grants the set",
/// and a shape that let one entry grant two would be the beginning of that
/// switch.
fn agent_tools(
    fields: &mut Fields<'_>,
    subject: &str,
    cx: &mut Cx,
) -> (Vec<Spanned<Address>>, Vec<BuiltinAttachment>) {
    let mut references: Vec<Spanned<Address>> = Vec::new();
    let mut builtins: Vec<BuiltinAttachment> = Vec::new();
    let Some(node) = fields.take("tools") else {
        return (references, builtins);
    };
    let Some(items) = expect_sequence(node, &format!("`tools` in {subject}"), cx) else {
        return (references, builtins);
    };
    for item in items {
        if item.as_mapping().is_some() {
            if let Some(attachment) = builtin_attachment(item, subject, cx) {
                if let Some(first) = builtins
                    .iter()
                    .find(|other| other.tool.value == attachment.tool.value)
                {
                    cx.push(
                        Diagnostic::error(
                            DiagnosticCode::InvalidValue,
                            attachment.tool.span.clone(),
                            format!("`tools` lists `{}` twice", attachment.tool.value.address()),
                        )
                        .with_label(first.tool.span.clone(), "first listed here")
                        .with_help(format!(
                            "one entry attaches one built-in under one set of bounds; a second \
                             entry for the same name would offer the model two `{}` tools \
                             (grammar 5.5, 11.5)",
                            attachment.tool.value.as_str()
                        )),
                    );
                    continue;
                }
                builtins.push(attachment);
            }
            continue;
        }
        // A built-in written as a bare address: the name is right and the shape
        // is not, so the repair is the shape rather than the namespace list a
        // reference diagnostic would print.
        if let Yaml::String(text) = &item.value
            && let Some(tool) = Builtin::from_address(text)
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidReference,
                    item.span.clone(),
                    format!("`{text}` is a built-in tool, not a definition to reference"),
                )
                .with_help(format!(
                    "a built-in is attached as a mapping carrying its bounds: \
                     `- {text}: {{ {} }}` (grammar 5.5)",
                    if tool.runs_a_command() {
                        "root: <directory>, timeout: 30s"
                    } else {
                        "root: <directory>"
                    }
                )),
            );
            continue;
        }
        let Some(reference) = lexical::reference(
            item,
            "each entry of `tools`",
            &[Namespace::Tool, Namespace::Flow],
            cx,
        ) else {
            continue;
        };
        if let Some(first) = references
            .iter()
            .find(|other| other.value == reference.value)
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    reference.span.clone(),
                    format!("`tools` lists `{}` twice", reference.value),
                )
                .with_label(first.span.clone(), "first listed here"),
            );
            continue;
        }
        references.push(reference);
    }
    (references, builtins)
}

/// Read one `builtin.*` entry of a `tools:` list (grammar 5.5, Decision D123).
fn builtin_attachment(
    item: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<crate::ast::definition::BuiltinAttachment> {
    let entries = expect_mapping(item, "each entry of `tools`", cx)?.entries();
    let names = || list(Builtin::ALL.iter().map(|tool| tool.address()));
    let [entry] = entries else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                item.span.clone(),
                format!(
                    "each entry of `tools` attaches one tool, and this one declares {}",
                    if entries.is_empty() {
                        "none".to_string()
                    } else {
                        format!("{}", entries.len())
                    }
                ),
            )
            .with_help(format!(
                "a built-in is attached one name at a time — `- builtin.bash: {{ root: …, \
                 timeout: 30s }}` — so that what an agent holds is readable off the entry that \
                 holds it; the built-ins are {} (grammar 5.5)",
                names()
            )),
        );
        return None;
    };
    let key = &entry.key;
    let Some(tool) = Builtin::from_address(&key.value) else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownVariant,
                key.span.clone(),
                format!("`{}` is not a built-in tool", key.value),
            )
            .with_optional_help(
                // Suggested on the **local** names, with the shared
                // `builtin.` prefix taken off both sides. Left on, every pair
                // of names is eight characters closer than it is, and the
                // distance budget — a third of the length — is eight characters
                // wider: `builtin.grep` comes back as "did you mean
                // `builtin.bash`?", which is a nudge toward the one built-in
                // nobody should be nudged toward by accident (PRD G3).
                suggest(
                    key.value.strip_prefix("builtin.").unwrap_or(&key.value),
                    &Builtin::ALL
                        .iter()
                        .map(|tool| tool.as_str())
                        .collect::<Vec<_>>(),
                )
                .map(|name| format!("did you mean `builtin.{name}`?"))
                .or_else(|| {
                    Some(format!(
                        "the built-ins are {}; a `tool.*` or `flow.*` is attached as a bare \
                         address instead (grammar 5.4, 5.5)",
                        names()
                    ))
                }),
            ),
        );
        return None;
    };
    let context = format!("`{}` in {subject}", tool.address());
    let mapping = expect_mapping(&entry.value, &context, cx)?;
    let mut fields = Fields::new(mapping, entry.value.span.clone(), &context);
    let root = fields
        .require("root", cx)
        .and_then(|node| lexical::interpolated(node, "`root`", cx));
    let timeout = if tool.runs_a_command() {
        fields
            .require("timeout", cx)
            .and_then(|node| lexical::duration(node, "`timeout`", cx))
    } else {
        // Taken so `finish` does not report it as a plain unknown key: the
        // sentence a reader needs here is why this tool has no timeout, not a
        // list of the keys it does take.
        if let Some(node) = fields.take("timeout") {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownKey,
                    node.span.clone(),
                    format!("unknown key `timeout` in {context}"),
                )
                .with_help(
                    "`timeout:` bounds the command `builtin.bash` runs; a file tool has no \
                     command to bound, and a node-level `timeout:` bounds the whole agent node \
                     (grammar 5.5, 9.2)",
                ),
            );
        }
        None
    };
    fields.finish(cx);
    Some(crate::ast::definition::BuiltinAttachment {
        tool: Spanned::new(tool, key.span.clone()),
        root,
        timeout,
        span: item.span.clone(),
    })
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
        // `metadata_schema:` is a `vector` key alone: no `kv` or `blob` op takes
        // a `metadata` or a `filter` parameter, and none of the tools a `blob`
        // attachment synthesizes carries one, so declared elsewhere it would be
        // a key every write and every read ignores (Decision D113).
        let (required, illegal): (&[&str], &[&str]) = match kind.value {
            StoreKind::Kv => (&["value_schema"], &["metadata_schema", "embed"]),
            StoreKind::Vector => (&["embed"], &["value_schema"]),
            StoreKind::Blob => (&[], &["value_schema", "metadata_schema", "embed"]),
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
    // `provider:` is REQUIRED: a storage backend never computes vectors, and
    // there is nothing for an omission to resolve to — §14.2's storage
    // vocabulary publishes no embedding capability, and under `--target local`
    // no alias and no per-kind default is consulted at all (Decision D116).
    let provider = fields
        .require("provider", cx)
        .and_then(|node| lexical::reference(node, "`provider`", &[Namespace::Provider], cx));
    let dimensions = fields
        .integer("dimensions", cx)
        .filter(|value| super::reader::at_least(value, "`dimensions`", 1, cx));
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
    let kind_ref = kind.as_ref();

    let api_key = secret_field(fields, "api_key", kind_ref, subject, cx);
    let base_url = secret_field(fields, "base_url", kind_ref, subject, cx);
    let access_key_id = secret_field(fields, "access_key_id", kind_ref, subject, cx);
    let secret_access_key = secret_field(fields, "secret_access_key", kind_ref, subject, cx);
    let session_token = secret_field(fields, "session_token", kind_ref, subject, cx);
    let credentials_json = secret_field(fields, "credentials_json", kind_ref, subject, cx);

    let api_version = plain_field(fields, "api_version", kind_ref, subject, cx);
    let organization = plain_field(fields, "organization", kind_ref, subject, cx);
    let region = plain_field(fields, "region", kind_ref, subject, cx);
    let location = plain_field(fields, "location", kind_ref, subject, cx);
    let project = plain_field(fields, "project", kind_ref, subject, cx);
    let profile = plain_field(fields, "profile", kind_ref, subject, cx);

    let headers = provider_key(fields, "headers", kind_ref, subject, cx)
        .map(|node| binding::interpolated_map(node, "`headers`", binding::NameForm::HeaderLike, cx))
        .unwrap_or_default();
    let server_tools = server_tools(fields, kind_ref, subject, cx);
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
        credential(fields, kind, subject, cx);
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
        server_tools,
        description,
    }
}

/// Read `server_tools:` — the array of wire config objects a provider appends
/// to every request it serves (grammar 12.1, Decision D122).
///
/// The kind gate is here rather than in [`provider_key`] because the refusal is
/// a different sentence: a `region:` on an `anthropic` provider is a key from
/// another kind's row, while a `server_tools:` on a `bedrock` one is a key this
/// **release** has not taught that kind's wire, and the repair is not "move it"
/// but "wait, or reach the same models through a kind whose wire carries it".
/// A reader who is told the wrong one goes looking for a typo.
///
/// Only the two things the compiler must decide are decided here: that every
/// entry is a mapping, and that each one names a plain-string `type:`. The
/// **contents** are the validator's (`check/providers.rs`), because which fields
/// a tool has depends on the provider `kind:` — one literal away in the same
/// mapping, but read through a table that also has to answer the second tier's
/// "this one is not in the table at all", which is a warning rather than an
/// error and belongs where the other provider-plugin checks are.
fn server_tools(
    fields: &mut Fields<'_>,
    kind: Option<&Spanned<ProviderKind>>,
    subject: &str,
    cx: &mut Cx,
) -> Vec<crate::ast::definition::ServerToolDef> {
    let Some(entry) = fields.take_entry("server_tools") else {
        return Vec::new();
    };
    if let Some(kind) = kind
        && !kind.value.serves_server_tools()
    {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::UnsupportedServerTools,
                entry.key.span.clone(),
                format!(
                    "{subject} declares `kind: {}`, whose wire this compiler release does not \
                     carry server tools on",
                    kind.value.as_str()
                ),
            )
            .with_label(kind.span.clone(), "the kind is declared here")
            .with_help(format!(
                "server tools launched on {} — a tool that runs on the provider's side rides the \
                 request that provider serves, and the other wires have not been taught the \
                 shape, so a config declared here would never reach one (grammar 12.1, \
                 Decision D122)",
                list(
                    ProviderKind::ALL
                        .iter()
                        .filter(|kind| kind.serves_server_tools())
                        .map(|kind| format!("kind: {}", kind.as_str()))
                        .collect::<Vec<_>>()
                )
            )),
        );
        return Vec::new();
    }
    let Some(items) = expect_sequence(&entry.value, "`server_tools`", cx) else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| server_tool(item, index, cx))
        .collect()
}

/// One entry of `server_tools:`.
fn server_tool(node: &Node, index: usize, cx: &mut Cx) -> crate::ast::definition::ServerToolDef {
    let context = format!("`server_tools[{index}]`");
    let empty = crate::ast::definition::ServerToolDef {
        type_name: None,
        config: Vec::new(),
        span: node.span.clone(),
    };
    let Some(mapping) = expect_mapping(node, &context, cx) else {
        return empty;
    };
    let type_name = match mapping.get("type") {
        Some(node) => lexical::text(node, &format!("`type` of {context}"), cx),
        None => {
            cx.error(
                DiagnosticCode::MissingKey,
                &node.span,
                format!(
                    "missing required key `type` in {context}: a server tool is named by the \
                     `type:` its provider's wire takes"
                ),
            );
            None
        }
    };
    // Everything else is the provider's vocabulary and travels verbatim: an
    // unknown key here is not a mistake but the whole point of the key
    // (Decision D50's plugin-config exception, grammar 12.1).
    let config = mapping
        .entries()
        .iter()
        .filter(|entry| entry.key.value != "type")
        .map(|entry| {
            lexical::reject_env_refs(&entry.key, &format!("a config key of {context}"), cx);
            crate::ast::deploy::PluginEntry {
                key: entry.key.clone(),
                value: super::deploy::plugin_value(
                    &entry.value,
                    &format!("`{}` of {context}", entry.key.value),
                    cx,
                ),
            }
        })
        .collect();
    crate::ast::definition::ServerToolDef {
        type_name,
        config,
        span: node.span.clone(),
    }
}

/// A connection pointed at a vendor's own endpoint declares a key (grammar
/// 12.1, Decision D120).
///
/// The one *conditional* required key, which is why it is not a row of
/// [`ProviderKind::required_keys`]: on the two kinds with a default endpoint,
/// omitting `base_url:` is not an omission at all — the connection resolves to
/// `https://api.anthropic.com` or `https://api.openai.com`, where nothing but
/// `api_key:` authenticates — while declaring one says the traffic goes to a
/// gateway that supplies the credential server-side. So what is refused is the
/// **pair** being absent together, and the message names both repairs because
/// either one alone is a complete fix.
///
/// Decided from `contains` rather than from the parsed values, exactly as the
/// unconditional rows above are: a declared `api_key:` holding a literal is an
/// `invalid-env-ref` about the value, and piling a second report about the key
/// being *absent* on top of it would be a report about a key the author wrote.
fn credential(fields: &Fields<'_>, kind: &Spanned<ProviderKind>, subject: &str, cx: &mut Cx) {
    let Some(endpoint) = kind.value.default_endpoint() else {
        return;
    };
    if fields.contains("api_key") || fields.contains("base_url") {
        return;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::MissingCredential,
            fields.span.clone(),
            format!(
                "{subject} declares `kind: {}` and neither `api_key:` nor `base_url:`",
                kind.value.as_str()
            ),
        )
        .with_label(kind.span.clone(), "the kind is declared here")
        .with_help(format!(
            "with no `base_url:` this connection reaches `{endpoint}`, where only a key \
             authenticates: declare `api_key:`, or name the gateway that supplies one with \
             `base_url:` — a provider carrying a `base_url:` and no key sends no authentication \
             header at all (grammar 12.1, Decision D120)"
        )),
    );
}

/// Read one provider key, rejecting it when the declared kind has no such key
/// (grammar 12.1).
///
/// Without a legible `kind:` every key is read as written: one unreadable kind
/// should not cascade into a dozen reports about keys that may well be right.
fn provider_key<'a>(
    fields: &mut Fields<'a>,
    key: &'static str,
    kind: Option<&Spanned<ProviderKind>>,
    subject: &str,
    cx: &mut Cx,
) -> Option<&'a Node> {
    let entry = fields.take_entry(key)?;
    let Some(kind) = kind else {
        return Some(&entry.value);
    };
    if kind.value.keys().contains(&key) {
        return Some(&entry.value);
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::UnknownKey,
            entry.key.span.clone(),
            format!(
                "{subject} declares `kind: {}`, which has no `{key}` key",
                kind.value.as_str()
            ),
        )
        .with_label(kind.span.clone(), "the kind is declared here")
        .with_help(format!(
            "the keys of `kind: {}` are {}",
            kind.value.as_str(),
            list(kind.value.keys())
        )),
    );
    None
}

/// A provider key that must hold an `${ENV}` value-form reference (grammar 4.3).
fn secret_field(
    fields: &mut Fields<'_>,
    key: &'static str,
    kind: Option<&Spanned<ProviderKind>>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<crate::ast::common::EnvRef>> {
    provider_key(fields, key, kind, subject, cx)
        .and_then(|node| lexical::env_ref(node, &format!("`{key}`"), cx))
}

/// A provider key that is a plain string and may interpolate (grammar 12.1).
fn plain_field(
    fields: &mut Fields<'_>,
    key: &'static str,
    kind: Option<&Spanned<ProviderKind>>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<crate::ast::common::Interpolated>> {
    provider_key(fields, key, kind, subject, cx)
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
        let entries: Vec<crate::ast::common::LiteralEntry> = mapping
            .entries()
            .iter()
            .map(|entry| crate::ast::common::LiteralEntry {
                key: entry.key.clone(),
                value: literal(&entry.value),
            })
            .collect();
        // Grammar 4.3 names "model `id` and every value inside `settings:`" in
        // one class-3 breath, for PRD 5.9's reason: every LLM configuration in
        // a project stays greppable in one file. `settings:` is open and
        // arbitrarily deep, so the token can sit in a nested mapping or an
        // array as easily as at the top — and with nothing downstream reading
        // it as a reference, an unescaped one would reach the IR as the
        // characters the author did not intend.
        let context = format!("`settings` of {subject}");
        for entry in &entries {
            lexical::reject_env_refs(&entry.key, &context, cx);
            schema::reject_env_refs_in_literal(&entry.value, &context, cx);
            // `settings:` is open, but it is still data the compiler lowers into
            // the artifact and from there into a provider call (grammar 3.8):
            // JSON has no notation for infinity or NaN, so one written here
            // would otherwise reach the artifact as `null` — a value the author
            // never wrote. Refused where it is written, and named by the key it
            // sits under, because a subtree is where it can hide.
            schema::reject_non_finite_in_literal(
                &entry.value,
                &format!("`{}` in {context}", entry.key.value),
                cx,
            );
        }
        Some(Settings {
            entries,
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
