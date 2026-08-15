//! Lowering the parsed composition into the flat IR.
//!
//! This runs **only when nothing has been rejected**, and that is what makes it
//! total. The parser records a missing required key, an unreadable value, and an
//! unrecognised form as a diagnostic and leaves an `Option::None` or an
//! `Invalid` variant behind; the resolver does the same for a name that does not
//! resolve. So on the path that reaches here, every required key is present and
//! every form is one the grammar defines, and the lowering can turn the AST's
//! "not read yet" shapes into the artifact's "this is what the composition
//! says" shapes.
//!
//! Every step is nevertheless written as a total function returning
//! [`Option`]: nothing here may panic on a shape it did not expect, and a `None`
//! that escapes to [`composition`] means no artifact rather than a partial one.
//! [`super::resolve_with_target`] asserts in debug builds that this never
//! happens without a diagnostic to explain it.

use std::collections::BTreeMap;

use crate::ast::binding as ast_binding;
use crate::ast::common::LiteralEntry;
use crate::ast::definition as ast_def;
use crate::ast::deploy as ast_deploy;
use crate::ast::flow as ast_flow;
use crate::ast::policy as ast_policy;
use crate::ast::schema as ast_schema;
use crate::ast::trigger as ast_trigger;
use crate::diag::Spanned;
use crate::ir;

use super::files::Composition;
use super::index::Index;

/// Lower an optional construct, keeping "there was none" and "it could not be
/// lowered" apart: the first is `Some(None)`, the second is `None` and takes the
/// whole artifact with it.
fn optional<T, U>(value: Option<T>, lower: impl FnOnce(T) -> Option<U>) -> Option<Option<U>> {
    match value {
        None => Some(None),
        Some(value) => lower(value).map(Some),
    }
}

/// Lower one resolved composition into its artifact.
pub(crate) fn composition(source: &Composition, index: &Index<'_>) -> Option<ir::Ir> {
    let mut definitions = BTreeMap::new();
    for (address, declared) in &index.definitions {
        definitions.insert(address.clone(), definition(declared.definition)?);
    }

    let mut state = BTreeMap::new();
    for section in index.state.iter() {
        for channel in &section.value.channels {
            state.insert(
                channel.name.value.as_str().to_string(),
                ir::Channel {
                    name: channel.name.clone(),
                    ty: type_node(&channel.ty)?,
                    reduce: channel.reduce.as_ref().map(|reduce| reduce.value),
                    span: channel.span.clone(),
                },
            );
        }
    }

    let mut triggers = BTreeMap::new();
    for section in index.triggers.iter() {
        for declared in &section.value.triggers {
            triggers.insert(declared.name.value.as_str().to_string(), trigger(declared)?);
        }
    }

    let mut sources: Vec<ir::Source> = source
        .files
        .iter()
        .map(|file| ir::Source {
            path: file.name.clone(),
            role: file.role,
        })
        .collect();
    if let Some(deploy) = source.deploy.as_ref() {
        sources.push(ir::Source {
            path: deploy.name.clone(),
            role: ir::SourceRole::Deploy,
        });
    }

    Some(ir::Ir {
        ir_version: ir::IR_VERSION,
        spec_version: index.version.clone()?,
        entrypoint: source.entrypoint.clone(),
        target: source.target.clone(),
        sources,
        defaults: optional(index.defaults.as_ref(), |section| {
            policy(&section.value.value)
        })?,
        state,
        triggers,
        definitions,
        deploy: deploy(source)?,
    })
}

// --- definitions ----------------------------------------------------------

fn definition(source: &ast_def::Definition) -> Option<ir::Definition> {
    let body = match &source.body {
        ast_def::DefinitionBody::Agent(def) => ir::definition::DefinitionBody::Agent(agent(def)?),
        ast_def::DefinitionBody::Tool(def) => ir::definition::DefinitionBody::Tool(tool(def)?),
        ast_def::DefinitionBody::Flow(def) => ir::definition::DefinitionBody::Flow(flow(def)?),
        ast_def::DefinitionBody::Store(def) => ir::definition::DefinitionBody::Store(store(def)?),
        ast_def::DefinitionBody::Provider(def) => {
            ir::definition::DefinitionBody::Provider(provider(def)?)
        }
        ast_def::DefinitionBody::Model(def) => ir::definition::DefinitionBody::Model(model(def)?),
        ast_def::DefinitionBody::Invalid => return None,
    };
    Some(ir::Definition {
        address: source.address.clone(),
        span: source.span.clone(),
        body,
    })
}

fn agent(source: &ast_def::AgentDef) -> Option<ir::definition::Agent> {
    Some(ir::definition::Agent {
        model: source.model.clone()?,
        prompt: source.prompt.clone()?,
        output: field_map(source.output.as_ref()?)?,
        input: optional(source.input.as_ref(), field_map)?,
        tools: source.tools.clone(),
        stores: source.stores.clone(),
        description: source.description.clone(),
        max_tool_iterations: source.max_tool_iterations.as_ref().map(|value| value.value),
    })
}

fn tool(source: &ast_def::ToolDef) -> Option<ir::definition::Tool> {
    let implementation = match source.implementation.as_ref()? {
        ast_def::ToolImplementation::Exec(block) => {
            ir::flow::ToolImplementation::Exec { exec: exec(block)? }
        }
        ast_def::ToolImplementation::Http(block) => {
            ir::flow::ToolImplementation::Http { http: http(block)? }
        }
        ast_def::ToolImplementation::Function(binding) => ir::flow::ToolImplementation::Function {
            function: ir::binding::FunctionBinding {
                name: binding.name.clone()?,
                span: binding.span.clone(),
            },
        },
    };
    Some(ir::definition::Tool {
        description: source.description.clone()?,
        input: field_map(source.input.as_ref()?)?,
        output: field_map(source.output.as_ref()?)?,
        implementation,
    })
}

fn store(source: &ast_def::StoreDef) -> Option<ir::definition::Store> {
    Some(ir::definition::Store {
        kind: source.kind.as_ref()?.value,
        scope: source.scope.as_ref()?.value,
        value_schema: optional(source.value_schema.as_ref(), field_map)?,
        metadata_schema: optional(source.metadata_schema.as_ref(), field_map)?,
        embed: optional(source.embed.as_ref(), |block| {
            Some(ir::definition::Embed {
                model: block.model.clone()?,
                provider: block.provider.clone()?,
                dimensions: block.dimensions.as_ref().map(|value| value.value),
                span: block.span.clone(),
            })
        })?,
        backend: source.backend.clone(),
        description: source.description.clone(),
        agent_access: source.agent_access.as_ref().map(|access| access.value),
    })
}

fn provider(source: &ast_def::ProviderDef) -> Option<ir::definition::Provider> {
    Some(ir::definition::Provider {
        kind: source.kind.as_ref()?.value,
        config: ir::definition::ProviderConfig {
            api_key: source.api_key.clone(),
            base_url: source.base_url.clone(),
            access_key_id: source.access_key_id.clone(),
            secret_access_key: source.secret_access_key.clone(),
            session_token: source.session_token.clone(),
            credentials_json: source.credentials_json.clone(),
            api_version: source.api_version.clone(),
            organization: source.organization.clone(),
            region: source.region.clone(),
            location: source.location.clone(),
            project: source.project.clone(),
            profile: source.profile.clone(),
            headers: source.headers.iter().map(interpolated_entry).collect(),
        },
        description: source.description.clone(),
    })
}

fn model(source: &ast_def::ModelDef) -> Option<ir::definition::Model> {
    Some(match source {
        ast_def::ModelDef::Direct(direct) => {
            ir::definition::Model::Direct(ir::definition::DirectModel {
                provider: direct.provider.clone()?,
                id: direct.id.clone()?,
                settings: direct
                    .settings
                    .as_ref()
                    .map(|settings| settings.entries.iter().map(literal_entry).collect())
                    .unwrap_or_default(),
                description: direct.description.clone(),
            })
        }
        ast_def::ModelDef::Route(route) => {
            ir::definition::Model::Route(ir::definition::RouteModel {
                route: route.route.clone(),
                route_on: route.route_on.clone(),
                description: route.description.clone(),
            })
        }
    })
}

/// One `settings:` entry, keyed by its name with the whole entry's span.
fn literal_entry(entry: &LiteralEntry) -> (String, Spanned<crate::ast::common::Literal>) {
    (
        entry.key.value.clone(),
        Spanned::new(
            entry.value.value.clone(),
            entry.key.span.joined(&entry.value.span),
        ),
    )
}

// --- flows ----------------------------------------------------------------

fn flow(source: &ast_flow::FlowDef) -> Option<ir::flow::Flow> {
    Some(ir::flow::Flow {
        description: source.description.clone(),
        inputs: optional(source.inputs.as_ref(), field_map)?,
        outputs: field_map(source.outputs.as_ref()?)?,
        nodes: source.nodes.iter().map(node).collect::<Option<Vec<_>>>()?,
        edges: source.edges.iter().map(edge).collect::<Option<Vec<_>>>()?,
    })
}

fn node(source: &ast_flow::Node) -> Option<ir::flow::Node> {
    let kind = match &source.kind {
        ast_flow::NodeKind::Agent(address) => ir::flow::NodeKind::Agent {
            agent: address.clone(),
        },
        ast_flow::NodeKind::Exec(block) => ir::flow::NodeKind::Exec { exec: exec(block)? },
        ast_flow::NodeKind::Http(block) => ir::flow::NodeKind::Http { http: http(block)? },
        ast_flow::NodeKind::Function(address) => ir::flow::NodeKind::Function {
            function: address.clone(),
        },
        ast_flow::NodeKind::Flow(flow_node) => ir::flow::NodeKind::Flow {
            flow: flow_node.flow.clone(),
            context: flow_node.context.as_ref().map(|context| context.value),
            policy: optional(flow_node.policy.as_ref(), |block| policy(&block.value))?,
        },
        ast_flow::NodeKind::Map(block) => ir::flow::NodeKind::Map { map: map(block)? },
        ast_flow::NodeKind::Human(block) => ir::flow::NodeKind::Human {
            human: ir::flow::Human {
                input: field_map(block.input.as_ref()?)?,
                output: field_map(block.output.as_ref()?)?,
                timeout: block.timeout.clone(),
                on_timeout: block.on_timeout.clone(),
                span: block.span.clone(),
            },
        },
        ast_flow::NodeKind::Store(store_node) => ir::flow::NodeKind::Store {
            store: store_node.store.clone(),
            op: store_node.op.as_ref()?.value,
            params: store_params(&store_node.params)?,
        },
        ast_flow::NodeKind::Invalid => return None,
    };
    Some(ir::flow::Node {
        id: source.id.clone(),
        input: optional(source.input.as_ref(), node_input)?,
        writes: source.writes.as_ref().map(writes),
        policy: policy(&source.policy)?,
        description: source.description.clone(),
        span: source.span.clone(),
        kind,
    })
}

fn edge(source: &ast_flow::Edge) -> Option<ir::flow::Edge> {
    Some(ir::flow::Edge {
        from: source.from.clone()?,
        to: source.to.clone()?,
        when: source.when.clone(),
        else_edge: source
            .else_edge
            .as_ref()
            .map(|span| Spanned::new(true, span.clone())),
        max_iterations: source.max_iterations.as_ref().map(|value| value.value),
        span: source.span.clone(),
    })
}

/// Lower a `map:` block, moving the three per-target keys onto the dispatch
/// form that may declare them.
///
/// `input:`, `writes:`, and `detach:` describe a dispatch *target*, so grammar
/// 8.6 rule 7 admits them as map-block keys on the homogeneous form and on an
/// individual route, never on a routed map's block (Decisions D31, D85). The
/// parser refuses the illegal combination, so folding them into the homogeneous
/// variant here loses nothing — and makes the shape the grammar forbids
/// unrepresentable in the artifact.
fn map(source: &ast_flow::MapBlock) -> Option<ir::flow::Map> {
    let dispatch = match &source.dispatch {
        ast_flow::MapDispatch::Homogeneous { node } => ir::flow::MapDispatch::Homogeneous {
            node: node.clone(),
            input: optional(source.input.as_ref(), node_input)?,
            writes: source.writes.as_ref().map(writes),
            detach: source.detach.clone(),
        },
        ast_flow::MapDispatch::Routed {
            route_by,
            routes,
            default,
        } => ir::flow::MapDispatch::Routed {
            route_by: route_by.clone(),
            routes: routes.iter().map(map_route).collect::<Option<Vec<_>>>()?,
            default: optional(default.as_ref(), |route| map_route(route).map(Box::new))?,
        },
        ast_flow::MapDispatch::Invalid => return None,
    };
    Some(ir::flow::Map {
        over: source.over.clone()?,
        item_binding: source.item_binding.clone(),
        max_concurrency: source.max_concurrency.as_ref()?.value,
        on_item_error: optional(source.on_item_error.as_ref(), |strategy| {
            Some(match &strategy.value {
                ast_flow::ItemError::Fail => ir::flow::ItemError::Fail {
                    span: strategy.span.clone(),
                },
                ast_flow::ItemError::Skip => ir::flow::ItemError::Skip {
                    span: strategy.span.clone(),
                },
                ast_flow::ItemError::Retry(block) => ir::flow::ItemError::Retry {
                    retry: retry(block)?,
                    span: strategy.span.clone(),
                },
            })
        })?,
        span: source.span.clone(),
        dispatch,
    })
}

fn map_route(source: &ast_flow::MapRoute) -> Option<ir::flow::MapRoute> {
    Some(ir::flow::MapRoute {
        tag: source.tag.clone(),
        node: source.node.clone()?,
        max_concurrency: source.max_concurrency.as_ref().map(|value| value.value),
        input: optional(source.input.as_ref(), node_input)?,
        writes: source.writes.as_ref().map(writes),
        detach: source.detach.clone(),
        span: source.span.clone(),
    })
}

fn store_params(source: &ast_flow::StoreOpParams) -> Option<ir::flow::StoreParams> {
    Some(ir::flow::StoreParams {
        key: source.key.clone(),
        value: source.value.as_ref().map(|value| match value {
            ast_flow::StoreValue::Expression(cel) => {
                ir::flow::StoreValue::Expression { value: cel.clone() }
            }
            ast_flow::StoreValue::Fields(fields) => ir::flow::StoreValue::Fields {
                bindings: bindings(fields),
            },
        }),
        query: source.query.clone(),
        prefix: source.prefix.clone(),
        top_k: source.top_k.as_ref().map(|value| value.value),
        limit: source.limit.as_ref().map(|value| value.value),
        filter: source.filter.as_ref().map(bindings),
        metadata: source.metadata.as_ref().map(bindings),
        content_type: source.content_type.clone(),
    })
}

// --- bindings and blocks --------------------------------------------------

fn node_input(source: &ast_binding::NodeInput) -> Option<ir::binding::NodeInput> {
    Some(match source {
        ast_binding::NodeInput::Scalar(cel) => {
            ir::binding::NodeInput::Scalar { value: cel.clone() }
        }
        ast_binding::NodeInput::Fields(fields) => ir::binding::NodeInput::Fields {
            bindings: bindings(fields),
        },
    })
}

fn bindings(source: &ast_binding::Bindings) -> ir::binding::Bindings {
    ir::binding::Bindings {
        entries: source
            .entries
            .iter()
            .map(|entry| ir::binding::Binding {
                name: entry.name.clone(),
                value: entry.value.clone(),
            })
            .collect(),
        span: source.span.clone(),
    }
}

fn writes(source: &ast_binding::Writes) -> ir::binding::Writes {
    ir::binding::Writes {
        entries: source
            .entries
            .iter()
            .map(|entry| ir::binding::WriteEntry {
                field: entry.field.clone(),
                channel: entry.channel.clone(),
            })
            .collect(),
        span: source.span.clone(),
    }
}

fn interpolated_entry(source: &ast_binding::InterpolatedEntry) -> ir::binding::InterpolatedEntry {
    ir::binding::InterpolatedEntry {
        name: source.name.clone(),
        value: source.value.clone(),
    }
}

fn exec(source: &ast_binding::ExecBlock) -> Option<ir::binding::Exec> {
    Some(ir::binding::Exec {
        command: source.command.clone()?,
        args: source.args.clone(),
        cwd: source.cwd.clone(),
        env: source.env.iter().map(interpolated_entry).collect(),
        expect_exit: source.expect_exit.iter().map(|value| value.value).collect(),
        output: optional(source.output.as_ref(), field_map)?,
        span: source.span.clone(),
    })
}

fn http(source: &ast_binding::HttpBlock) -> Option<ir::binding::Http> {
    Some(ir::binding::Http {
        method: source.method.clone()?,
        url: source.url.clone()?,
        headers: source.headers.iter().map(interpolated_entry).collect(),
        query: source.query.as_ref().map(bindings),
        body: source.body.as_ref().map(bindings),
        expect_status: source
            .expect_status
            .iter()
            .map(|value| value.value)
            .collect(),
        output: optional(source.output.as_ref(), field_map)?,
        span: source.span.clone(),
    })
}

// --- policy ---------------------------------------------------------------

fn policy(source: &ast_policy::PolicyBlock) -> Option<ir::Policy> {
    Some(ir::Policy {
        retry: optional(source.retry.as_ref(), retry)?,
        timeout: source.timeout.clone(),
        on_error: source
            .on_error
            .as_ref()
            .map(|strategy| match &strategy.value {
                ast_policy::OnError::Fail => ir::policy::OnError::Fail {
                    span: strategy.span.clone(),
                },
                ast_policy::OnError::Skip => ir::policy::OnError::Skip {
                    span: strategy.span.clone(),
                },
                ast_policy::OnError::Fallback(target) => ir::policy::OnError::Fallback {
                    target: target.clone(),
                    span: strategy.span.clone(),
                },
            }),
    })
}

fn retry(source: &Spanned<ast_policy::Retry>) -> Option<ir::policy::Retry> {
    Some(ir::policy::Retry {
        max: source.value.max.as_ref()?.value,
        backoff: source.value.backoff.clone()?,
        multiplier: source.value.multiplier.as_ref().map(|value| value.value),
        max_backoff: source.value.max_backoff.clone(),
        jitter: source.value.jitter.as_ref().map(|value| value.value),
        span: source.span.clone(),
    })
}

// --- schemas --------------------------------------------------------------

fn field_map(source: &ast_schema::FieldMap) -> Option<ir::schema::FieldMap> {
    Some(ir::schema::FieldMap {
        surface: source.surface,
        fields: source
            .fields
            .iter()
            .map(|field| {
                Some(ir::schema::Field {
                    name: field.name.clone(),
                    ty: type_node(&field.ty)?,
                })
            })
            .collect::<Option<Vec<_>>>()?,
        span: source.span.clone(),
    })
}

fn type_node(source: &ast_schema::TypeNode) -> Option<ir::schema::TypeNode> {
    let form = match &source.form {
        ast_schema::TypeForm::Scalar(scalar) => ir::schema::TypeForm::Scalar(ir::schema::Scalar {
            kind: scalar.kind.value,
            min_length: scalar.min_length.as_ref().map(|value| value.value),
            max_length: scalar.max_length.as_ref().map(|value| value.value),
            pattern: scalar.pattern.as_ref().map(|value| value.value.clone()),
            format: scalar.format.as_ref().map(|value| value.value),
            minimum: scalar.minimum.as_ref().map(|value| value.value),
            maximum: scalar.maximum.as_ref().map(|value| value.value),
            exclusive_minimum: scalar.exclusive_minimum.as_ref().map(|value| value.value),
            exclusive_maximum: scalar.exclusive_maximum.as_ref().map(|value| value.value),
            multiple_of: scalar.multiple_of.as_ref().map(|value| value.value),
            default: scalar.default.clone(),
        }),
        ast_schema::TypeForm::Enum(variants) => ir::schema::TypeForm::Enum(ir::schema::EnumType {
            variants: variants.variants.clone(),
            default: variants.default.clone(),
        }),
        ast_schema::TypeForm::Object(object) => {
            ir::schema::TypeForm::Object(ir::schema::ObjectType {
                properties: field_map(&object.properties)?,
                optional: object.optional.clone(),
                default: object.default.clone(),
            })
        }
        ast_schema::TypeForm::Array(array) => ir::schema::TypeForm::Array(ir::schema::ArrayType {
            items: Box::new(type_node(&array.items)?),
            max_items: array.max_items.as_ref().map(|value| value.value),
            min_items: array.min_items.as_ref().map(|value| value.value),
            unique_items: array.unique_items.as_ref().map(|value| value.value),
            default: array.default.clone(),
        }),
        ast_schema::TypeForm::Union(union) => ir::schema::TypeForm::Union(ir::schema::UnionType {
            discriminator: union.discriminator.clone(),
            variants: union
                .variants
                .iter()
                .map(|variant| {
                    Some(ir::schema::UnionVariant {
                        tag: variant.tag.clone(),
                        fields: field_map(&variant.fields)?,
                    })
                })
                .collect::<Option<Vec<_>>>()?,
        }),
        ast_schema::TypeForm::Invalid => return None,
    };
    Some(ir::schema::TypeNode {
        description: source.description.clone(),
        span: source.span.clone(),
        form,
    })
}

// --- triggers -------------------------------------------------------------

fn trigger(source: &ast_trigger::Trigger) -> Option<ir::Trigger> {
    let kind = match source.kind.as_ref()? {
        ast_trigger::TriggerKind::Manual => ir::trigger::TriggerKind::Manual,
        ast_trigger::TriggerKind::Http(http) => {
            ir::trigger::TriggerKind::Http(ir::trigger::HttpTrigger {
                path: http.path.clone(),
                method: http.method.as_ref().map(|method| method.value),
                input: http.input.as_ref().map(bindings),
                respond: http.respond.as_ref().map(|respond| respond.value),
                timeout: http.timeout.clone(),
                callback: http.callback.clone(),
            })
        }
        ast_trigger::TriggerKind::Schedule(schedule) => {
            ir::trigger::TriggerKind::Schedule(ir::trigger::ScheduleTrigger {
                cron: schedule.cron.clone()?,
                timezone: schedule.timezone.clone(),
                input: schedule.input.as_ref().map(bindings),
            })
        }
        ast_trigger::TriggerKind::Event(event) => {
            ir::trigger::TriggerKind::Event(ir::trigger::EventTrigger {
                source: event.source.clone()?,
                input: event.input.as_ref().map(bindings),
                dedupe_key: event.dedupe_key.clone(),
            })
        }
    };
    Some(ir::Trigger {
        name: source.name.clone(),
        flow: source.flow.clone()?,
        session_key: source.session_key.clone(),
        description: source.description.clone(),
        span: source.span.clone(),
        kind,
    })
}

// --- the deploy layer -----------------------------------------------------

fn deploy(source: &Composition) -> Option<ir::Deploy> {
    let Some(file) = source.deploy.as_ref() else {
        return Some(ir::Deploy {
            target: source.target.clone(),
            source: None,
            placements: BTreeMap::new(),
            storage_backends: None,
            event_sources: BTreeMap::new(),
        });
    };

    let mut placements = BTreeMap::new();
    for section in file.file.placements.iter() {
        for placement in &section.placements {
            placements.insert(
                placement.address.value.to_string(),
                ir::deploy::Placement {
                    address: placement.address.clone(),
                    runtime: placement.runtime.as_ref()?.value,
                    network: placement.network.as_ref().map(|network| network.value),
                    description: placement.description.clone(),
                    span: placement.span.clone(),
                },
            );
        }
    }

    let storage_backends = optional(file.file.storage_backends.as_ref(), |section| {
        let mut defaults = BTreeMap::new();
        for entry in &section.defaults {
            defaults.insert(
                entry.kind.value.as_str().to_string(),
                backend(
                    Spanned::new(
                        entry.kind.value.as_str().to_string(),
                        entry.kind.span.clone(),
                    ),
                    Some(entry.kind.value),
                    &entry.config,
                )?,
            );
        }
        let mut aliases = BTreeMap::new();
        for entry in &section.aliases {
            aliases.insert(
                entry.name.value.as_str().to_string(),
                backend(
                    Spanned::new(
                        entry.name.value.as_str().to_string(),
                        entry.name.span.clone(),
                    ),
                    None,
                    &entry.config,
                )?,
            );
        }
        Some(ir::deploy::StorageBackends {
            defaults,
            aliases,
            span: section.span.clone(),
        })
    })?;

    let mut event_sources = BTreeMap::new();
    for section in file.file.event_sources.iter() {
        for entry in &section.sources {
            event_sources.insert(
                entry.name.value.as_str().to_string(),
                ir::deploy::EventSource {
                    name: entry.name.clone(),
                    kind: entry.kind.as_ref()?.value,
                    connection: connection(&entry.connection),
                    extra: plugin_entries(&entry.extra),
                    span: entry.span.clone(),
                },
            );
        }
    }

    Some(ir::Deploy {
        target: source.target.clone(),
        source: Some(file.name.clone()),
        placements,
        storage_backends,
        event_sources,
    })
}

fn backend(
    name: Spanned<String>,
    kind: Option<crate::ast::definition::StoreKind>,
    config: &ast_deploy::BackendConfig,
) -> Option<ir::deploy::BackendConfig> {
    Some(ir::deploy::BackendConfig {
        span: name.span.joined(&config.span),
        name,
        kind,
        provider: config.provider.as_ref()?.value,
        connection: connection(&config.connection),
        extra: plugin_entries(&config.extra),
    })
}

fn connection(
    source: &[ast_deploy::ConnectionField],
) -> BTreeMap<String, Spanned<crate::ast::common::EnvRef>> {
    source
        .iter()
        .map(|field| {
            (
                field.name.value.clone(),
                Spanned::new(
                    field.value.value.clone(),
                    field.name.span.joined(&field.value.span),
                ),
            )
        })
        .collect()
}

fn plugin_entries(
    source: &[ast_deploy::PluginEntry],
) -> BTreeMap<String, Spanned<ast_deploy::PluginValue>> {
    source
        .iter()
        .map(|entry| {
            (
                entry.key.value.clone(),
                Spanned::new(
                    entry.value.value.clone(),
                    entry.key.span.joined(&entry.value.span),
                ),
            )
        })
        .collect()
}
