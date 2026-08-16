//! The deploy layer's sections (grammar 14).

use crate::ast::common::Namespace;
use crate::ast::definition::StoreKind;
use crate::ast::deploy::{
    BackendAlias, BackendConfig, BackendDefault, BackendProvider, ConnectionField, EventSource,
    EventSourceKind, EventSourcesSection, Network, Placement, PlacementsSection, PluginEntry,
    PluginValue, Runtime, SECRET_FIELDS, StorageBackendsSection,
};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::{Mapping, Node, Yaml};

use super::definition::description;
use super::lexical;
use super::reader::{Cx, Fields, expect_mapping, list};

const RUNTIMES: &[(&str, Runtime)] = &[
    ("isolated", Runtime::Isolated),
    ("colocated", Runtime::Colocated),
];

const NETWORKS: &[(&str, Network)] = &[
    ("none", Network::None),
    ("egress", Network::Egress),
    ("all", Network::All),
];

/// Read the `placements:` section (grammar 14.1).
pub(crate) fn placements(node: &Node, cx: &mut Cx) -> Option<PlacementsSection> {
    let mapping = expect_mapping(node, "`placements`", cx)?;
    let mut placements = Vec::new();
    for entry in mapping.entries() {
        let Some(address) = lexical::address(
            &entry.key,
            "a placement key",
            &[Namespace::Agent, Namespace::Tool, Namespace::Flow],
            cx,
        ) else {
            continue;
        };
        let subject = format!("placement `{}`", address.value);
        let Some(body) = expect_mapping(&entry.value, &subject, cx) else {
            continue;
        };
        let mut fields = Fields::new(body, entry.value.span.clone(), &subject);
        let runtime = fields
            .require("runtime", cx)
            .and_then(|node| lexical::keyword(node, "`runtime`", RUNTIMES, cx));
        let network = fields
            .take("network")
            .and_then(|node| lexical::keyword(node, "`network`", NETWORKS, cx));
        let description = description(&mut fields, cx);
        fields.finish(cx);

        placements.push(Placement {
            address,
            runtime,
            network,
            description,
            span: entry.key.span.joined(&entry.value.span),
        });
    }
    Some(PlacementsSection {
        placements,
        span: node.span.clone(),
    })
}

const STORE_KINDS: &[(&str, StoreKind)] = &[
    ("kv", StoreKind::Kv),
    ("vector", StoreKind::Vector),
    ("blob", StoreKind::Blob),
];

/// Read the `storage_backends:` section (grammar 14.2).
pub(crate) fn storage_backends(node: &Node, cx: &mut Cx) -> Option<StorageBackendsSection> {
    let mapping = expect_mapping(node, "`storage_backends`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "`storage_backends`");

    let mut defaults = Vec::new();
    if let Some(node) = fields.take("defaults")
        && let Some(body) = expect_mapping(node, "`storage_backends.defaults`", cx)
    {
        for entry in body.entries() {
            let Some(kind) = lexical::keyword(
                &Node {
                    value: crate::yaml::Yaml::String(entry.key.value.clone()),
                    span: entry.key.span.clone(),
                },
                "store kind",
                STORE_KINDS,
                cx,
            ) else {
                continue;
            };
            let subject = format!("the `{}` backend default", kind.value.as_str());
            let Some(config) = backend_config(&entry.value, &subject, Some(kind.value), cx) else {
                continue;
            };
            defaults.push(BackendDefault { kind, config });
        }
    }

    let mut aliases = Vec::new();
    if let Some(node) = fields.take("aliases")
        && let Some(body) = expect_mapping(node, "`storage_backends.aliases`", cx)
    {
        for entry in body.entries() {
            let Some(name) = lexical::key_identifier(&entry.key, "backend alias", cx) else {
                continue;
            };
            let subject = format!("backend alias `{}`", name.value);
            let Some(config) = backend_config(&entry.value, &subject, None, cx) else {
                continue;
            };
            aliases.push(BackendAlias { name, config });
        }
    }
    fields.finish(cx);

    Some(StorageBackendsSection {
        defaults,
        aliases,
        span: node.span.clone(),
    })
}

fn backend_config(
    node: &Node,
    subject: &str,
    kind: Option<StoreKind>,
    cx: &mut Cx,
) -> Option<BackendConfig> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    let providers: Vec<(&str, BackendProvider)> = BackendProvider::ALL
        .iter()
        .map(|provider| (provider.as_str(), *provider))
        .collect();
    let provider = fields
        .require("provider", cx)
        .and_then(|node| lexical::keyword(node, "storage `provider`", &providers, cx))
        .filter(|provider| match kind {
            Some(kind) if provider.value.kind() != kind => {
                let accepted: Vec<&str> = BackendProvider::ALL
                    .iter()
                    .filter(|candidate| candidate.kind() == kind)
                    .map(|candidate| candidate.as_str())
                    .collect();
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        provider.span.clone(),
                        format!(
                            "`{}` is not a `{}` storage provider",
                            provider.value.as_str(),
                            kind.as_str()
                        ),
                    )
                    .with_help(format!(
                        "the `{}` providers are {}",
                        kind.as_str(),
                        list(&accepted)
                    )),
                );
                false
            }
            _ => true,
        });

    let (connection, extra) = plugin_config(mapping, &["provider"], subject, cx);
    Some(BackendConfig {
        provider,
        connection,
        extra,
        span: node.span.clone(),
    })
}

const EVENT_SOURCE_KINDS: &[(&str, EventSourceKind)] = &[
    ("redis_streams", EventSourceKind::RedisStreams),
    ("sqs", EventSourceKind::Sqs),
    ("nats", EventSourceKind::Nats),
];

/// Read the `event_sources:` section (grammar 14.3).
pub(crate) fn event_sources(node: &Node, cx: &mut Cx) -> Option<EventSourcesSection> {
    let mapping = expect_mapping(node, "`event_sources`", cx)?;
    let mut sources = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = lexical::key_identifier(&entry.key, "event source name", cx) else {
            continue;
        };
        let subject = format!("event source `{}`", name.value);
        let Some(body) = expect_mapping(&entry.value, &subject, cx) else {
            continue;
        };
        let mut fields = Fields::new(body, entry.value.span.clone(), &subject);
        let kind = fields
            .require("kind", cx)
            .and_then(|node| lexical::keyword(node, "event source `kind`", EVENT_SOURCE_KINDS, cx));
        let (connection, extra) = plugin_config(body, &["kind"], &subject, cx);
        sources.push(EventSource {
            name,
            kind,
            connection,
            extra,
            span: entry.key.span.joined(&entry.value.span),
        });
    }
    Some(EventSourcesSection {
        sources,
        span: node.span.clone(),
    })
}

/// Split an open plugin-config object into the connection fields grammar 4.3
/// closes over — which must be `${ENV}` value-form references — and everything
/// else, which the plugin's own published schema checks (Decision D50).
///
/// "Everything else" is not unchecked. Grammar 4.3 puts "non-secret
/// `storage_backends` and `event_sources` config values" in class 2 alongside
/// the `exec:` block and provider `headers`, so each one is read as an
/// interpolable string on the way past: a malformed `${…}` token is reported
/// here, and a well-formed one has its name recorded so the reference survives
/// unresolved into the IR for `build`/`serve`/`run` to check for presence.
/// *Which* keys a plugin admits is still the plugin's schema's business.
fn plugin_config(
    mapping: &Mapping,
    typed: &[&str],
    subject: &str,
    cx: &mut Cx,
) -> (Vec<ConnectionField>, Vec<PluginEntry>) {
    let mut connection = Vec::new();
    let mut extra = Vec::new();
    for entry in mapping.entries() {
        if typed.contains(&entry.key.value.as_str()) {
            continue;
        }
        let context = format!("`{}` in {subject}", entry.key.value);
        if SECRET_FIELDS.contains(&entry.key.value.as_str()) {
            let Some(value) = lexical::env_ref(&entry.value, &context, cx) else {
                continue;
            };
            connection.push(ConnectionField {
                name: entry.key.clone(),
                value,
            });
            continue;
        }
        // A plugin option is *named* by its key, not substituted into it. Class
        // 2 covers the config values, so Decision D92's totality rule leaves the
        // keys in class 3 — the same split `settings:` already draws, where the
        // key is rejected for a token as readily as the value. An unescaped one
        // here would reach the plugin as the characters the author did not
        // intend.
        lexical::reject_env_refs(&entry.key, &format!("a config key of {subject}"), cx);
        extra.push(PluginEntry {
            key: entry.key.clone(),
            value: plugin_value(&entry.value, &context, cx),
        });
    }
    (connection, extra)
}

/// Read one value of an open plugin-config object (grammar 4.3 class 2).
///
/// A plugin object carries arbitrary YAML, so the walk is recursive: a token
/// can sit inside a nested mapping or a sequence as easily as at the top, which
/// is the same reason `settings:` is walked to its leaves for the class-3 rule.
fn plugin_value(node: &Node, subject: &str, cx: &mut Cx) -> Spanned<PluginValue> {
    let value = match &node.value {
        Yaml::Null => PluginValue::Null,
        Yaml::Bool(value) => PluginValue::Bool(*value),
        Yaml::Int(value) => PluginValue::Int(*value),
        Yaml::Float(value) => PluginValue::Float(*value),
        Yaml::String(text) => {
            let text = Spanned::new(text.clone(), node.span.clone());
            PluginValue::Text(lexical::interpolate(text, subject, cx).value)
        }
        Yaml::Sequence(items) => PluginValue::Sequence(
            items
                .iter()
                .map(|item| plugin_value(item, subject, cx))
                .collect(),
        ),
        Yaml::Mapping(mapping) => PluginValue::Mapping(
            mapping
                .entries()
                .iter()
                .map(|entry| {
                    lexical::reject_env_refs(
                        &entry.key,
                        &format!("a nested config key of {subject}"),
                        cx,
                    );
                    PluginEntry {
                        key: entry.key.clone(),
                        value: plugin_value(&entry.value, subject, cx),
                    }
                })
                .collect(),
        ),
    };
    Spanned::new(value, node.span.clone())
}
