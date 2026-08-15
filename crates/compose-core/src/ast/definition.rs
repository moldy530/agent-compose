//! The six definition namespaces (grammar 5, 6, 7, 11, 12).

use crate::diag::{Span, Spanned};

use super::binding::{ExecBlock, FunctionBinding, HttpBlock, InterpolatedEntry};
use super::common::{Address, Ident, Interpolated, LiteralEntry};
use super::flow::FlowDef;
use super::schema::FieldMap;

/// One top-level definition: its address and its body.
#[derive(Clone, Debug, PartialEq)]
pub struct Definition {
    /// The definition key, e.g. `agent.reviewer`.
    pub address: Spanned<Address>,
    /// The definition's body.
    pub body: DefinitionBody,
    /// The whole entry's span, key and value together — what a diagnostic about
    /// the definition as a whole underlines. [`Self::address`] carries the key
    /// alone, for the diagnostics that are about the name.
    pub span: Span,
}

/// The body of a definition, one variant per namespace.
///
/// The variants differ in size because the constructs do — a provider's key
/// table is long, a model's is short. Each is built once per definition and
/// matched by reference from then on, so boxing would add indirection to every
/// consumer and buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum DefinitionBody {
    /// An `agent.*` definition.
    Agent(AgentDef),
    /// A `tool.*` definition.
    Tool(ToolDef),
    /// A `flow.*` definition.
    Flow(FlowDef),
    /// A `store.*` definition.
    Store(StoreDef),
    /// A `provider.*` definition.
    Provider(ProviderDef),
    /// A `model.*` definition.
    Model(ModelDef),
}

/// An `agent.*` definition: one LLM call with structured output (grammar 5).
#[derive(Clone, Debug, PartialEq)]
pub struct AgentDef {
    /// `model:` — required; a `model.*` reference.
    pub model: Option<Spanned<Address>>,
    /// `prompt:` — required; literal text, no interpolation (Decision D13).
    pub prompt: Option<Spanned<String>>,
    /// `output:` — required; at least one property.
    pub output: Option<FieldMap>,
    /// `input:` — optional; omitting it selects the string-in default.
    pub input: Option<FieldMap>,
    /// `tools:` — `tool.*` and `flow.*` references.
    pub tools: Vec<Spanned<Address>>,
    /// `stores:` — `store.*` references.
    pub stores: Vec<Spanned<Address>>,
    /// `description:` — documentation only; agents are not tools.
    pub description: Option<Spanned<String>>,
    /// `max_tool_iterations:` — 1..=50, defaults to 8 (Decision D51).
    pub max_tool_iterations: Option<Spanned<i64>>,
}

/// A `tool.*` definition: one implementation, two usage surfaces (grammar 6).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDef {
    /// `description:` — required; the LLM's selection signal.
    pub description: Option<Spanned<String>>,
    /// `input:` — required; `{}` for a no-argument tool.
    pub input: Option<FieldMap>,
    /// `output:` — required; `{}` for a tool with no result.
    pub output: Option<FieldMap>,
    /// Exactly one of `exec:`, `http:`, `function:`.
    pub implementation: Option<ToolImplementation>,
}

/// A tool's implementation binding (grammar 6.1).
#[derive(Clone, Debug, PartialEq)]
pub enum ToolImplementation {
    /// A subprocess.
    Exec(ExecBlock),
    /// An HTTP request.
    Http(HttpBlock),
    /// A host-registered function (the escape hatch).
    Function(FunctionBinding),
}

/// The store kinds (grammar 11.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    /// Key-value.
    Kv,
    /// Vector index.
    Vector,
    /// Blob storage.
    Blob,
}

impl StoreKind {
    /// Every kind, in the order grammar 11.1 lists them.
    pub const ALL: &'static [Self] = &[Self::Kv, Self::Vector, Self::Blob];

    /// The keyword that names this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kv => "kv",
            Self::Vector => "vector",
            Self::Blob => "blob",
        }
    }

    /// The ops this kind serves (grammar 11.4).
    #[must_use]
    pub const fn operations(self) -> &'static [&'static str] {
        match self {
            Self::Kv => &["get", "set", "delete", "list"],
            Self::Vector => &["search", "upsert", "delete"],
            Self::Blob => &["put", "get", "delete", "list"],
        }
    }
}

/// A store's lifetime (grammar 11.1, Decision D35).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreScope {
    /// Dies with the execution.
    Execution,
    /// Persists across executions sharing a session key.
    Session,
    /// Persists globally.
    Global,
}

/// How much of a store's surface an attached agent gets (grammar 11.1,
/// Decision D37).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentAccess {
    /// Read-only tools.
    Read,
    /// Read and write tools (the default).
    ReadWrite,
}

/// A `store.*` definition (grammar 11.1).
#[derive(Clone, Debug, PartialEq)]
pub struct StoreDef {
    /// `kind:` — required.
    pub kind: Option<Spanned<StoreKind>>,
    /// `scope:` — required.
    pub scope: Option<Spanned<StoreScope>>,
    /// `value_schema:` — required for `kv`, illegal otherwise.
    pub value_schema: Option<FieldMap>,
    /// `metadata_schema:` — legal for `vector` and `blob`.
    pub metadata_schema: Option<FieldMap>,
    /// `embed:` — required for `vector`, illegal otherwise.
    pub embed: Option<EmbedBlock>,
    /// `backend:` — an abstract alias, resolved per target (grammar 11.3).
    pub backend: Option<Spanned<Ident>>,
    /// `description:` — LLM-facing for agent-attached stores.
    pub description: Option<Spanned<String>>,
    /// `agent_access:` — defaults to `read_write`.
    pub agent_access: Option<Spanned<AgentAccess>>,
}

/// A vector store's `embed:` block (grammar 11.2, Decision D36).
#[derive(Clone, Debug, PartialEq)]
pub struct EmbedBlock {
    /// `model:` — a provider-native embedding model id, not a `model.*` ref.
    pub model: Option<Spanned<String>>,
    /// `provider:` — which connection serves it.
    pub provider: Option<Spanned<Address>>,
    /// `dimensions:` — asserted against the backend's index.
    pub dimensions: Option<Spanned<i64>>,
    /// The block's own span.
    pub span: Span,
}

/// The v0 provider kinds (grammar 12.1, Decision D38).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKind {
    /// `anthropic`
    Anthropic,
    /// `openai`
    OpenAi,
    /// `openai_compatible`
    OpenAiCompatible,
    /// `azure_openai`
    AzureOpenAi,
    /// `bedrock`
    Bedrock,
    /// `vertex`
    Vertex,
}

impl ProviderKind {
    /// Every kind, in the order grammar 12.1 lists them.
    pub const ALL: &'static [Self] = &[
        Self::Anthropic,
        Self::OpenAi,
        Self::OpenAiCompatible,
        Self::AzureOpenAi,
        Self::Bedrock,
        Self::Vertex,
    ];

    /// The keyword that names this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::OpenAiCompatible => "openai_compatible",
            Self::AzureOpenAi => "azure_openai",
            Self::Bedrock => "bedrock",
            Self::Vertex => "vertex",
        }
    }

    /// The keys this kind requires (grammar 12.1).
    #[must_use]
    pub const fn required_keys(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic | Self::OpenAi => &["api_key"],
            Self::OpenAiCompatible => &["base_url"],
            Self::AzureOpenAi => &["base_url", "api_key", "api_version"],
            Self::Bedrock => &["region"],
            Self::Vertex => &["project", "location"],
        }
    }

    /// Every key this kind accepts, required ones included (grammar 12.1).
    ///
    /// `kind`, `headers`, and `description` belong to every provider
    /// definition: the first table of grammar 12.1 lists them without naming a
    /// kind, and neither is connection config. Everything else comes from the
    /// kind's own row of the second table, so a `region:` pasted onto an
    /// `anthropic` provider is a diagnostic rather than a key that builds and
    /// then does nothing (Decision D50).
    #[must_use]
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &["kind", "api_key", "base_url", "headers", "description"],
            Self::OpenAi => &[
                "kind",
                "api_key",
                "base_url",
                "organization",
                "headers",
                "description",
            ],
            Self::OpenAiCompatible => &["kind", "base_url", "api_key", "headers", "description"],
            Self::AzureOpenAi => &[
                "kind",
                "base_url",
                "api_key",
                "api_version",
                "headers",
                "description",
            ],
            Self::Bedrock => &[
                "kind",
                "region",
                "access_key_id",
                "secret_access_key",
                "session_token",
                "profile",
                "headers",
                "description",
            ],
            Self::Vertex => &[
                "kind",
                "project",
                "location",
                "credentials_json",
                "headers",
                "description",
            ],
        }
    }
}

#[cfg(test)]
mod provider_kind_tests {
    use super::ProviderKind;

    #[test]
    fn every_kind_accepts_the_keys_it_requires() {
        for kind in ProviderKind::ALL {
            assert!(
                kind.keys().contains(&"kind"),
                "`{}` does not accept `kind`",
                kind.as_str()
            );
            for key in kind.required_keys() {
                assert!(
                    kind.keys().contains(key),
                    "`{}` requires `{key}` but does not accept it",
                    kind.as_str()
                );
            }
        }
    }
}

/// A `provider.*` definition: an inference connection (grammar 12.1).
///
/// Credential and connection fields hold `${ENV}` value-form references only;
/// the plain-string fields may interpolate (grammar 4.3, Decision D41).
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderDef {
    /// `kind:` — required; selects the plugin and its required keys.
    pub kind: Option<Spanned<ProviderKind>>,
    /// `api_key:`
    pub api_key: Option<Spanned<super::common::EnvRef>>,
    /// `base_url:`
    pub base_url: Option<Spanned<super::common::EnvRef>>,
    /// `access_key_id:`
    pub access_key_id: Option<Spanned<super::common::EnvRef>>,
    /// `secret_access_key:`
    pub secret_access_key: Option<Spanned<super::common::EnvRef>>,
    /// `session_token:`
    pub session_token: Option<Spanned<super::common::EnvRef>>,
    /// `credentials_json:`
    pub credentials_json: Option<Spanned<super::common::EnvRef>>,
    /// `api_version:`
    pub api_version: Option<Spanned<Interpolated>>,
    /// `organization:`
    pub organization: Option<Spanned<Interpolated>>,
    /// `region:`
    pub region: Option<Spanned<Interpolated>>,
    /// `location:`
    pub location: Option<Spanned<Interpolated>>,
    /// `project:`
    pub project: Option<Spanned<Interpolated>>,
    /// `profile:`
    pub profile: Option<Spanned<Interpolated>>,
    /// `headers:`
    pub headers: Vec<InterpolatedEntry>,
    /// `description:`
    pub description: Option<Spanned<String>>,
}

/// The conditions a model route fails over on (grammar 12.2).
///
/// Infrastructure conditions only: content-based routing is what graph edges
/// are for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteCondition {
    /// The provider rate-limited the call.
    RateLimit,
    /// The provider reported itself overloaded.
    Overloaded,
    /// The call timed out.
    Timeout,
    /// The provider returned a server error.
    ServerError,
}

impl RouteCondition {
    /// Every condition, in the order grammar 12.2 lists them.
    pub const ALL: &'static [Self] = &[
        Self::RateLimit,
        Self::Overloaded,
        Self::Timeout,
        Self::ServerError,
    ];

    /// The keyword that names this condition.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RateLimit => "rate_limit",
            Self::Overloaded => "overloaded",
            Self::Timeout => "timeout",
            Self::ServerError => "server_error",
        }
    }
}

/// A `model.*` definition: a direct binding or a route, never both
/// (Decision D39).
#[derive(Clone, Debug, PartialEq)]
pub enum ModelDef {
    /// A direct provider binding.
    Direct(DirectModel),
    /// An ordered failover route.
    Route(RouteModel),
}

/// The direct form of a model definition (grammar 12.2).
#[derive(Clone, Debug, PartialEq)]
pub struct DirectModel {
    /// `provider:` — required.
    pub provider: Option<Spanned<Address>>,
    /// `id:` — required; the provider-native model id, no env refs.
    pub id: Option<Spanned<String>>,
    /// `settings:` — the one open object in the logical layer (Decision D40).
    pub settings: Option<Settings>,
    /// `description:`
    pub description: Option<Spanned<String>>,
}

/// The route form of a model definition (grammar 12.2).
#[derive(Clone, Debug, PartialEq)]
pub struct RouteModel {
    /// `route:` — at least two `model.*` members, in failover order.
    pub route: Vec<Spanned<Address>>,
    /// `route_on:` — defaults to `[rate_limit, overloaded, timeout]`.
    pub route_on: Option<Vec<Spanned<RouteCondition>>>,
    /// `description:`
    pub description: Option<Spanned<String>>,
}

/// A model's `settings:` block, kept as spanned literals.
///
/// This is the one open object in the logical layer: known keys are typed for
/// editors and plugin-specific keys are legal, so the compiler checks the block
/// against the provider plugin's published schema rather than against a closed
/// key list (Decision D40).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The entries, in declaration order.
    pub entries: Vec<LiteralEntry>,
    /// The mapping's own span.
    pub span: Span,
}
