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
    /// A definition whose body the parser could not read at all — it was not a
    /// mapping, so not one key of it is available.
    ///
    /// The address survives anyway, because it is the name the author declared
    /// and the resolver's index is a table of *names*: dropping the entry would
    /// make every reference to it undefined, and a composition that uses the
    /// definition ten times would answer one mistake with eleven diagnostics.
    Invalid,
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

impl StoreScope {
    /// The keyword that names this scope.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Execution => "execution",
            Self::Session => "session",
            Self::Global => "global",
        }
    }
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

impl AgentAccess {
    /// The keyword that names this level of access.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::ReadWrite => "read_write",
        }
    }
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
    /// `metadata_schema:` — a `vector` key alone. No `kv` or `blob` op takes a
    /// `metadata` or `filter` parameter and none of the tools a `blob`
    /// attachment synthesizes carries one, so the key would be inert there
    /// (grammar 11.1, Decision D113).
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
    /// `provider:` — REQUIRED; the connection that computes the vectors. A
    /// storage backend never embeds: `backend:` forks per target and says where
    /// the vectors live, while the embedding connection is logical-layer and
    /// does not (grammar 11.2, Decision D116).
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

    /// The keys this kind requires **unconditionally** (grammar 12.1).
    ///
    /// `anthropic` and `openai` name nothing here, and that is the rule rather
    /// than an omission: their `api_key:` is required only where no `base_url:`
    /// points the connection away from the vendor's own endpoint, which is a
    /// disjunction this list cannot state. [`default_endpoint`] is the other
    /// half, and the pair is decided one pass earlier — `parse/definition.rs`'s
    /// `credential`, because two literals in one mapping is one file's business
    /// (Decision D120, `docs/grammar.md` Appendix B).
    #[must_use]
    pub const fn required_keys(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic | Self::OpenAi => &[],
            Self::OpenAiCompatible => &["base_url"],
            Self::AzureOpenAi => &["base_url", "api_key", "api_version"],
            Self::Bedrock => &["region"],
            Self::Vertex => &["project", "location"],
        }
    }

    /// The endpoint a connection of this kind reaches when it declares no
    /// `base_url:` (grammar 12.1, Decision D120).
    ///
    /// `Some` for exactly the two kinds that have one, and `None` for every kind
    /// that does not: `openai_compatible` and `azure_openai` require `base_url:`
    /// outright, and the two SDK-reached kinds have no bare endpoint at all.
    /// This is what makes the conditional credential rule statable — "no
    /// `base_url:`" means "reaching the vendor" only where a default exists to
    /// fall back to — and the host is carried rather than merely the fact,
    /// because the diagnostic names it: the key is required *because* of a
    /// default the author cannot see in their own file. The emitted runtime
    /// falls back to the same two hosts, which
    /// `the_default_endpoints_are_the_ones_the_emitted_runtime_falls_back_to`
    /// holds.
    #[must_use]
    pub const fn default_endpoint(self) -> Option<&'static str> {
        match self {
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::OpenAi => Some("https://api.openai.com"),
            Self::OpenAiCompatible | Self::AzureOpenAi | Self::Bedrock | Self::Vertex => None,
        }
    }

    /// Whether this kind takes `server_tools:` (grammar 12.1, Decision D122).
    ///
    /// Three kinds do, and each for its own reason: `anthropic` carries the
    /// suite on the Messages wire it already speaks, `openai` carries it on the
    /// Responses wire a declared suite switches it to, and
    /// `openai_compatible` carries it because a gateway may honour any
    /// vocabulary at all and refusing the key would recreate the very support
    /// treadmill resolved q30 exists to avoid.
    ///
    /// The other three are refused **outright** rather than warned: their wires
    /// have not been taught the shape, so a config declared on one would be
    /// dropped on the floor — a silent no-op, which is what
    /// [D50](../../../docs/grammar.md) refuses everywhere else.
    #[must_use]
    pub const fn serves_server_tools(self) -> bool {
        matches!(
            self,
            Self::Anthropic | Self::OpenAi | Self::OpenAiCompatible
        )
    }

    /// Every key this kind accepts, required ones included (grammar 12.1).
    ///
    /// `kind` and `description` are the two keys grammar 12.1 states are legal
    /// on every provider; everything else — `headers` included — comes from the
    /// kind's own row of the second table, which is closed. A `region:` pasted
    /// onto an `anthropic` provider is therefore a diagnostic rather than a key
    /// that builds and then does nothing (Decision D106, Decision D50), and so
    /// is a `headers:` on one of the two SDK-reached kinds: `bedrock` and
    /// `vertex` take neither `base_url` nor `headers`, because a connection
    /// made through a cloud SDK has no bare endpoint to point at and no request
    /// the spec composes headers onto.
    #[must_use]
    pub const fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &[
                "kind",
                "api_key",
                "base_url",
                "headers",
                "server_tools",
                "description",
            ],
            Self::OpenAi => &[
                "kind",
                "api_key",
                "base_url",
                "organization",
                "headers",
                "server_tools",
                "description",
            ],
            Self::OpenAiCompatible => &[
                "kind",
                "base_url",
                "api_key",
                "headers",
                "server_tools",
                "description",
            ],
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
                "description",
            ],
            Self::Vertex => &[
                "kind",
                "project",
                "location",
                "credentials_json",
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

    /// Grammar 12.1 names the two SDK-reached kinds outright: `bedrock` and
    /// `vertex` take neither `base_url` nor `headers`, because a connection
    /// made through a cloud SDK has no bare endpoint to point at and no request
    /// the spec composes headers onto. The published schema refuses both keys
    /// on both kinds, so accepting one here would break Appendix B's
    /// one-directional invariant as well as D106's closed row.
    #[test]
    fn the_sdk_reached_kinds_take_neither_base_url_nor_headers() {
        for kind in [ProviderKind::Bedrock, ProviderKind::Vertex] {
            for key in ["base_url", "headers"] {
                assert!(
                    !kind.keys().contains(&key),
                    "`{}` must not accept `{key}` (grammar 12.1)",
                    kind.as_str()
                );
            }
        }
    }

    /// The two kinds the conditional credential rule is about, held to the two
    /// properties that make it statable: each accepts both credential-shaped
    /// keys, and neither key is required outright — while every kind *without* a
    /// default endpoint still names required keys the parser can enforce on its
    /// own (grammar 12.1, Decision D120).
    ///
    /// The day a seventh kind arrives with a vendor endpoint of its own, this is
    /// what fails until [`default_endpoint`](ProviderKind::default_endpoint) and
    /// [`keys`](ProviderKind::keys) above have been told about it — the two rows
    /// `parse/definition.rs`'s `credential` reads. The published schema states
    /// the same conditional a third time, hand-duplicated per kind
    /// (`schemas/agent-compose.schema.json`); it is held to this table rather
    /// than to a list of its own, because `schema_conformance.rs`'s
    /// `the_published_schema_accepts_a_keyless_provider_that_names_its_endpoint`
    /// derives the kinds it asserts over from `default_endpoint`.
    #[test]
    fn only_the_kinds_with_a_default_endpoint_leave_their_credential_conditional() {
        let mut defaulted = Vec::new();
        for kind in ProviderKind::ALL {
            let Some(endpoint) = kind.default_endpoint() else {
                assert!(
                    !kind.required_keys().is_empty(),
                    "`{}` reaches no endpoint of its own, so its row requires keys outright",
                    kind.as_str()
                );
                continue;
            };
            assert!(
                endpoint.starts_with("https://"),
                "`{}`'s default endpoint is a URL the diagnostic can quote",
                kind.as_str()
            );
            defaulted.push(kind.as_str());
            for key in ["api_key", "base_url"] {
                assert!(
                    kind.keys().contains(&key),
                    "`{}` must accept `{key}` for the conditional rule to have two repairs",
                    kind.as_str()
                );
            }
            assert!(
                kind.required_keys().is_empty(),
                "`{}`'s credential rule is conditional and the parser's, so nothing is required outright",
                kind.as_str()
            );
        }
        assert_eq!(defaulted, ["anthropic", "openai"]);
    }

    /// The other half of the same row rule: every kind reached over plain HTTP
    /// does take `headers:`, so the exclusion above stays a statement about two
    /// rows rather than a retreat from the key.
    #[test]
    fn every_http_reached_kind_takes_headers() {
        for kind in [
            ProviderKind::Anthropic,
            ProviderKind::OpenAi,
            ProviderKind::OpenAiCompatible,
            ProviderKind::AzureOpenAi,
        ] {
            assert!(
                kind.keys().contains(&"headers"),
                "`{}` must accept `headers` (grammar 12.1)",
                kind.as_str()
            );
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
    /// `server_tools:` — the tools this connection's provider runs on its own
    /// side, in that provider's wire vocabulary (grammar 12.1, Decision D122).
    pub server_tools: Vec<ServerToolDef>,
    /// `description:`
    pub description: Option<Spanned<String>>,
}

/// One entry of a provider's `server_tools:` array (grammar 12.1,
/// Decision D122).
///
/// The entry is a **wire object**, not a construct of this grammar: `type:` is
/// the only key the compiler requires, and everything beside it travels to the
/// provider verbatim. Values are grammar 4.3 class 2 — non-secret provider
/// config, so they may interpolate — which is why they are read as
/// [`PluginValue`](super::deploy::PluginValue) rather than as
/// [`Literal`](super::common::Literal).
#[derive(Clone, Debug, PartialEq)]
pub struct ServerToolDef {
    /// `type:` — required, and a plain string: it is the key the vendor's
    /// vocabulary is looked up under, so an `${ENV}` here would be a tool whose
    /// identity is not decidable at compile time.
    pub type_name: Option<Spanned<String>>,
    /// Every key of the entry beside `type:`, in declaration order.
    pub config: Vec<super::deploy::PluginEntry>,
    /// The entry's own span.
    pub span: Span,
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
