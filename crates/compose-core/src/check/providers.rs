//! Provider settings and the three capability checks (grammar 11.2, 12).
//!
//! # Settings
//!
//! `settings:` is the one open object in the logical layer: the editor schema
//! types the common keys and permits the rest, and **the compiler checks the
//! block against the provider plugin's published schema** (grammar 12.2,
//! Decision D40) — which is why `thinking:` on an OpenAI provider is an error
//! at the model definition and not at the first call.
//!
//! v0 ships no plugin registry, so the published schemas are the table below:
//! one row per provider kind, holding the keys that kind accepts and the shape
//! of each. It is the v0 stand-in for what a plugin will publish, it lives in
//! exactly one place so that a registry replaces it rather than joins it, and
//! it is closed in the same sense grammar 12.1's key row is (Decision D106): a
//! key no row names is a key the plugin will not read.
//!
//! # Capabilities
//!
//! Three rules read one capability table (grammar 12.2, 11.2):
//!
//! * an **agent's model** must come from a provider declaring structured output
//!   — agents are structured-output calls, so a model that cannot honor the
//!   contract is a compile error (PRD 5.2);
//! * a **route's members** must be capability-equivalent, so failover cannot
//!   silently break structured output;
//! * a **vector store's `embed.provider`** must serve embeddings (Decision
//!   D116).
//!
//! Every v0 chat provider serves structured output and tool use, so the first
//! two rules cannot fire on a v0 composition and have no negative fixture; they
//! are table-driven all the same, because the table is what a new kind — or a
//! new capability — is added to. The third does fire: `anthropic` publishes no
//! embeddings API, which is why `examples/triage-fanout` embeds through its
//! `openai_compatible` provider rather than its `anthropic` one.

use crate::ast::common::{Address, Literal};
use crate::ast::definition::ProviderKind;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::definition::{DefinitionBody, Model};

use super::Ctx;

/// What a provider kind can serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Capabilities {
    /// Constrained decoding against a declared schema — what an agent needs.
    pub(crate) structured_output: bool,
    /// Tool calling.
    pub(crate) tool_use: bool,
    /// An embeddings endpoint — what a `vector` store's `embed:` needs.
    pub(crate) embeddings: bool,
}

/// The v0 capability table (grammar 12.2, 11.2).
pub(crate) const fn capabilities(kind: ProviderKind) -> Capabilities {
    match kind {
        // Anthropic's API has no embeddings endpoint; embedding through an
        // Anthropic connection is not something a plugin could implement.
        ProviderKind::Anthropic => Capabilities {
            structured_output: true,
            tool_use: true,
            embeddings: false,
        },
        ProviderKind::OpenAi
        | ProviderKind::OpenAiCompatible
        | ProviderKind::AzureOpenAi
        | ProviderKind::Bedrock
        | ProviderKind::Vertex => Capabilities {
            structured_output: true,
            tool_use: true,
            embeddings: true,
        },
    }
}

/// The shape of one setting value.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// A number, inclusive bounds.
    Number(f64, f64),
    /// An integer, inclusive bounds.
    Integer(i64, i64),
    /// A boolean.
    Boolean,
    /// One of a closed set of strings.
    Choice(&'static [&'static str]),
    /// An array of strings.
    Strings,
    /// A nested object with its own keys.
    Object(&'static [(&'static str, Shape)]),
}

/// The keys every provider kind publishes (grammar 12.2).
const COMMON: &[(&str, Shape)] = &[
    ("temperature", Shape::Number(0.0, 2.0)),
    ("top_p", Shape::Number(0.0, 1.0)),
    ("max_tokens", Shape::Integer(1, i64::MAX)),
    ("stop", Shape::Strings),
    ("seed", Shape::Integer(i64::MIN, i64::MAX)),
    ("parallel_tool_calls", Shape::Boolean),
];

const TOP_K: (&str, Shape) = ("top_k", Shape::Integer(1, i64::MAX));
const THINKING: (&str, Shape) = (
    "thinking",
    Shape::Object(&[("budget_tokens", Shape::Integer(1, i64::MAX))]),
);
const REASONING_EFFORT: (&str, Shape) = (
    "reasoning_effort",
    Shape::Choice(&["minimal", "low", "medium", "high"]),
);

/// The keys one kind publishes beyond [`COMMON`].
const fn extra(kind: ProviderKind) -> &'static [(&'static str, Shape)] {
    match kind {
        ProviderKind::Anthropic => &[TOP_K, THINKING],
        ProviderKind::OpenAi | ProviderKind::AzureOpenAi => &[REASONING_EFFORT],
        ProviderKind::OpenAiCompatible | ProviderKind::Vertex => &[TOP_K],
        // Bedrock serves Anthropic's models among others, so its published
        // schema carries their two knobs.
        ProviderKind::Bedrock => &[TOP_K, THINKING],
    }
}

fn published(kind: ProviderKind) -> Vec<(&'static str, Shape)> {
    COMMON
        .iter()
        .copied()
        .chain(extra(kind).iter().copied())
        .collect()
}

/// Check every model's settings and every capability requirement.
pub(crate) fn check(ctx: &mut Ctx) {
    let ir = ctx.ir;
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Model(Model::Direct(model)) => {
                let Some(provider) = ctx.provider(&model.provider.value) else {
                    continue;
                };
                let kind = provider.kind;
                for (key, value) in &model.settings {
                    setting(ctx, key, value, kind, address, &model.provider);
                }
            }
            DefinitionBody::Model(Model::Route(route)) => equivalence(ctx, address, route),
            DefinitionBody::Agent(agent) => {
                structured_output(ctx, &agent.model, address);
            }
            _ => {}
        }
    }
}

/// One `settings:` entry against the provider kind's published schema.
fn setting(
    ctx: &mut Ctx,
    key: &str,
    value: &Spanned<Literal>,
    kind: ProviderKind,
    model: &str,
    provider: &Spanned<Address>,
) {
    let schema = published(kind);
    let Some((_, shape)) = schema.iter().find(|(name, _)| *name == key) else {
        let known: Vec<&str> = schema.iter().map(|(name, _)| *name).collect();
        let help = crate::parse::reader::suggest(key, &known).map_or_else(
            || {
                format!(
                    "the `{}` provider plugin publishes {}",
                    kind.as_str(),
                    crate::parse::reader::list(&known)
                )
            },
            |name| format!("did you mean `{name}`?"),
        );
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownKey,
                value.span.clone(),
                format!(
                    "`{key}` is not a setting the `{}` provider plugin publishes",
                    kind.as_str()
                ),
            )
            .with_label(provider.span.clone(), "the provider is named here")
            .with_help(help),
        );
        return;
    };
    literal(ctx, &format!("`{key}` of `{model}`"), value, *shape);
}

fn literal(ctx: &mut Ctx, subject: &str, value: &Spanned<Literal>, shape: Shape) {
    match (shape, &value.value) {
        (Shape::Number(low, high), Literal::Int(number)) => {
            range(ctx, subject, &value.span, *number as f64, low, high);
        }
        (Shape::Number(low, high), Literal::Float(number)) => {
            range(ctx, subject, &value.span, *number, low, high);
        }
        (Shape::Integer(low, high), Literal::Int(number)) => {
            if *number < low || *number > high {
                ctx.error(
                    DiagnosticCode::ValueOutOfRange,
                    &value.span,
                    format!(
                        "{subject} is {number}, outside {}",
                        bounds(low as f64, high as f64)
                    ),
                );
            }
        }
        (Shape::Boolean, Literal::Bool(_)) => {}
        (Shape::Choice(choices), Literal::String(text)) => {
            if !choices.contains(&text.as_str()) {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        value.span.clone(),
                        format!("{subject} is `{text}`, which the provider plugin does not accept"),
                    )
                    .with_help(format!(
                        "the accepted values are {}",
                        crate::parse::reader::list(choices)
                    )),
                );
            }
        }
        (Shape::Strings, Literal::Sequence(items)) => {
            for item in items {
                if !matches!(item.value, Literal::String(_)) {
                    ctx.error(
                        DiagnosticCode::TypeMismatch,
                        &item.span,
                        format!(
                            "{subject} takes an array of strings, and this entry is {}",
                            item.value.description()
                        ),
                    );
                }
            }
        }
        (Shape::Object(keys), Literal::Mapping(entries)) => {
            for entry in entries {
                let Some((_, shape)) = keys.iter().find(|(name, _)| *name == entry.key.value)
                else {
                    let known: Vec<&str> = keys.iter().map(|(name, _)| *name).collect();
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::UnknownKey,
                            entry.key.span.clone(),
                            format!("`{}` is not a key of {subject}", entry.key.value),
                        )
                        .with_help(format!("it takes {}", crate::parse::reader::list(&known))),
                    );
                    continue;
                };
                literal(
                    ctx,
                    &format!("`{}` of {subject}", entry.key.value),
                    &entry.value,
                    *shape,
                );
            }
        }
        (shape, found) => {
            ctx.error(
                DiagnosticCode::TypeMismatch,
                &value.span,
                format!(
                    "{subject} takes {}, found {}",
                    describe(shape),
                    found.description()
                ),
            );
        }
    }
}

fn range(ctx: &mut Ctx, subject: &str, at: &Span, value: f64, low: f64, high: f64) {
    if value < low || value > high {
        ctx.error(
            DiagnosticCode::ValueOutOfRange,
            at,
            format!("{subject} is {value}, outside {}", bounds(low, high)),
        );
    }
}

fn bounds(low: f64, high: f64) -> String {
    match (low, high) {
        (low, high) if high == i64::MAX as f64 => format!("the range {low} and above"),
        (low, high) if low == i64::MIN as f64 => format!("the range up to {high}"),
        (low, high) => format!("the range {low} to {high}"),
    }
}

const fn describe(shape: Shape) -> &'static str {
    match shape {
        Shape::Number(..) => "a number",
        Shape::Integer(..) => "an integer",
        Shape::Boolean => "a boolean",
        Shape::Choice(_) => "a string",
        Shape::Strings => "an array of strings",
        Shape::Object(_) => "a mapping",
    }
}

/// An agent's model resolves to providers that can serve structured output
/// (grammar 12.2, PRD 5.2).
fn structured_output(ctx: &mut Ctx, model: &Spanned<Address>, agent: &str) {
    for (address, kind) in providers_of(ctx, &model.value) {
        if capabilities(kind).structured_output {
            continue;
        }
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::MissingCapability,
                model.span.clone(),
                format!(
                    "`{agent}` needs structured output, and `{}` is served by `{address}`, which does not publish it",
                    model.value
                ),
            )
            .with_help(
                "an agent is one LLM call with structured output, so every model it references must come from a provider that can honor the contract (grammar 12.2, PRD 5.2)",
            ),
        );
    }
}

/// A route's members are capability-equivalent (grammar 12.2).
fn equivalence(ctx: &mut Ctx, address: &str, route: &crate::ir::definition::RouteModel) {
    let mut baseline: Option<(String, Capabilities)> = None;
    for member in &route.route {
        let Some((provider, kind)) = providers_of(ctx, &member.value).into_iter().next() else {
            continue;
        };
        let capabilities = capabilities(kind);
        match &baseline {
            None => baseline = Some((provider, capabilities)),
            Some((first, want)) => {
                if inference(*want) != inference(capabilities) {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::MissingCapability,
                            member.span.clone(),
                            format!(
                                "`{address}` fails over to `{}`, which is not capability-equivalent to `{first}`",
                                member.value
                            ),
                        )
                        .with_help(
                            "every member of a route must publish the same inference capabilities, so a failover cannot silently break structured output (grammar 12.2)",
                        ),
                    );
                }
            }
        }
    }
}

/// The capabilities a `model.*` reference needs of its provider: what a chat
/// call uses, never what an embedding call does.
const fn inference(capabilities: Capabilities) -> (bool, bool) {
    (capabilities.structured_output, capabilities.tool_use)
}

/// A `vector` store's embedding connection must serve embeddings (grammar
/// 11.2, Decision D116).
pub(crate) fn require_embeddings(ctx: &mut Ctx, provider: &Spanned<Address>, store: &str) {
    let Some(definition) = ctx.provider(&provider.value) else {
        return;
    };
    if capabilities(definition.kind).embeddings {
        return;
    }
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::MissingCapability,
            provider.span.clone(),
            format!(
                "`{store}` embeds through `{}`, whose `{}` plugin publishes no embeddings",
                provider.value,
                definition.kind.as_str()
            ),
        )
        .with_help(
            "`embed.provider` names the connection that turns text into a vector, and the storage backend never computes them: name a provider whose kind serves embeddings (grammar 11.2, Decision D116)",
        ),
    );
}

/// The providers a `model.*` reference resolves to: one for a direct model,
/// one per member for a route.
fn providers_of(ctx: &Ctx, model: &Address) -> Vec<(String, ProviderKind)> {
    match ctx.model(model) {
        Some(Model::Direct(direct)) => ctx
            .provider(&direct.provider.value)
            .map(|provider| (direct.provider.value.to_string(), provider.kind))
            .into_iter()
            .collect(),
        Some(Model::Route(route)) => route
            .route
            .iter()
            .filter_map(|member| match ctx.model(&member.value) {
                Some(Model::Direct(direct)) => ctx
                    .provider(&direct.provider.value)
                    .map(|provider| (direct.provider.value.to_string(), provider.kind)),
                _ => None,
            })
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind v0 admits, so the two tests below read the whole table.
    const V0: &[ProviderKind] = &[
        ProviderKind::Anthropic,
        ProviderKind::OpenAi,
        ProviderKind::OpenAiCompatible,
        ProviderKind::AzureOpenAi,
        ProviderKind::Bedrock,
        ProviderKind::Vertex,
    ];

    #[test]
    fn every_v0_kind_serves_structured_output() {
        for kind in V0 {
            assert!(
                capabilities(*kind).structured_output,
                "{} must serve structured output for agents to be bindable to it",
                kind.as_str()
            );
        }
    }

    /// Why [`equivalence`] has nothing to reject on a v0 composition, stated
    /// over the table rather than left as a remark: every kind publishes the
    /// same pair of inference capabilities, so every route is equivalent by
    /// construction and no legal spec can reach that diagnostic. It is the
    /// evidence `tests/m0_inventory.rs` carries for that rule in place of a
    /// negative fixture, and the day a kind arrives that lacks one of the two,
    /// this test fails and the fixture becomes writable (grammar 12.2).
    #[test]
    fn every_v0_kind_publishes_the_same_inference_capabilities() {
        let baseline = inference(capabilities(ProviderKind::Anthropic));
        assert_eq!(baseline, (true, true));
        for kind in V0 {
            assert_eq!(
                inference(capabilities(*kind)),
                baseline,
                "{} differs, so a route mixing it with another kind is now rejectable",
                kind.as_str()
            );
        }
    }

    /// The one capability a v0 kind lacks, and the reason Decision D116's check
    /// has anything to reject.
    #[test]
    fn anthropic_publishes_no_embeddings() {
        assert!(!capabilities(ProviderKind::Anthropic).embeddings);
        assert!(capabilities(ProviderKind::OpenAiCompatible).embeddings);
    }

    #[test]
    fn thinking_is_published_by_the_kinds_that_serve_it() {
        let anthropic: Vec<&str> = published(ProviderKind::Anthropic)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(anthropic.contains(&"thinking"));
        assert!(anthropic.contains(&"max_tokens"));
        let openai: Vec<&str> = published(ProviderKind::OpenAi)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(!openai.contains(&"thinking"));
        assert!(openai.contains(&"reasoning_effort"));
    }
}
