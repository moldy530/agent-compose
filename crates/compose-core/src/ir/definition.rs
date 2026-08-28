//! The six definition namespaces in the IR (grammar 5, 6, 7, 11, 12).
//!
//! Every definition is inlined under its typed address, which is the key it
//! occupies in [`Ir::definitions`](super::Ir::definitions). Required keys are
//! required here: a definition that reached the artifact declared everything
//! grammar 5, 6, 7, 11, and 12 ask of it, because a missing one is a diagnostic
//! and a composition with a diagnostic produces no artifact.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::ast::common::{Address, Duration, Ident, Interpolated, Literal};
use crate::ast::definition::{
    AgentAccess, Builtin, ProviderKind, RouteCondition, StoreKind, StoreScope,
};
use crate::diag::{Span, Spanned};

use super::binding::InterpolatedEntry;
use super::flow::{Flow, ToolImplementation};
use super::schema::FieldMap;

/// One definition of the composition: its address, its span, and its body.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Definition {
    /// The definition key, repeated here with the span of the key itself — the
    /// map this sits in is keyed by the same address, so a value pulled out of
    /// the artifact on its own still names what it defines.
    pub address: Spanned<Address>,
    /// The whole entry's span, key and body together.
    pub span: Span,
    /// The body, tagged by the namespace that selects it.
    #[serde(flatten)]
    pub body: DefinitionBody,
}

/// The body of a definition, one variant per namespace (grammar 2.2).
///
/// The variants differ in size because the constructs do — a provider's key row
/// is long, a model's is short. Each is built once per definition and then only
/// read or written out, so boxing would add indirection to every consumer and
/// buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "namespace", rename_all = "snake_case")]
pub enum DefinitionBody {
    /// An `agent.*` definition.
    Agent(Agent),
    /// A `tool.*` definition.
    Tool(Tool),
    /// A `flow.*` definition.
    Flow(Flow),
    /// A `store.*` definition.
    Store(Store),
    /// A `provider.*` definition.
    Provider(Provider),
    /// A `model.*` definition.
    Model(Model),
}

/// An `agent.*` definition: one LLM call with structured output (grammar 5).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Agent {
    /// `model:` — the model binding or route this agent calls.
    pub model: Spanned<Address>,
    /// `prompt:` — literal text, no interpolation (Decision D13).
    pub prompt: Spanned<String>,
    /// `output:` — at least one property; what routing reads (PRD 5.2, 5.3).
    pub output: FieldMap,
    /// `input:` — absent means the string-in default (grammar 5.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<FieldMap>,
    /// `tools:` — `tool.*` and `flow.*` addresses, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Spanned<Address>>,
    /// `tools:` — the `builtin.*` entries of the same list, in declaration
    /// order (grammar 5.5, Decision D123).
    ///
    /// Omitted from the artifact when empty, which is what keeps a composition
    /// that attaches none byte-identical to one written before the key existed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub builtins: Vec<BuiltinTool>,
    /// `stores:` — `store.*` addresses, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stores: Vec<Spanned<Address>>,
    /// `description:` — documentation only; agents are not tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// `max_tool_iterations:` — absent means the default, `8` (Decision D51).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<i64>,
}

/// One `builtin.*` entry of an agent's `tools:` list, with the bounds it
/// declared (grammar 5.5, Decision D123, PRD resolved q31).
///
/// The bounds are not defaults and not optional here: `root:` is required of
/// every built-in and `timeout:` of `builtin.bash`, so an artifact that carries
/// one carries what bounds it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BuiltinTool {
    /// Which built-in, and the span of the entry key that named it.
    pub tool: Spanned<Builtin>,
    /// `root:` — the directory every path this tool touches must resolve
    /// inside, and `builtin.bash`'s working directory.
    pub root: Spanned<Interpolated>,
    /// `timeout:` — how long `builtin.bash`'s command may run. Absent on the
    /// file tools, which run no command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Spanned<Duration>>,
    /// The whole entry, key and bounds together.
    pub span: Span,
}

/// A `tool.*` definition: one implementation, two usage surfaces (grammar 6).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Tool {
    /// `description:` — the LLM's selection signal (PRD 5.5).
    pub description: Spanned<String>,
    /// `input:` — the tool's parameters; `{}` for a no-argument tool.
    pub input: FieldMap,
    /// `output:` — the result schema; `{}` for a tool with no result.
    pub output: FieldMap,
    /// Exactly one of `exec:`, `http:`, `function:`.
    #[serde(flatten)]
    pub implementation: ToolImplementation,
}

/// A `store.*` definition (grammar 11.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Store {
    /// `kind:` — `kv`, `vector`, or `blob`.
    pub kind: StoreKind,
    /// `scope:` — the lifetime (Decision D35).
    pub scope: StoreScope,
    /// `value_schema:` — `kv` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_schema: Option<FieldMap>,
    /// `metadata_schema:` — `vector` only (Decision D113). Absent is not the
    /// same as `{}`: a store that declares none has no metadata at all, and a
    /// `search`'s matches carry no `metadata` field (Decision D114).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata_schema: Option<FieldMap>,
    /// `embed:` — `vector` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed: Option<Embed>,
    /// `backend:` — an abstract alias the active target resolves (grammar 11.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<Spanned<Ident>>,
    /// `description:` — LLM-facing for agent-attached stores.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// `agent_access:` — absent means the default, `read_write`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_access: Option<AgentAccess>,
}

/// A vector store's `embed:` block (grammar 11.2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Embed {
    /// `model:` — a provider-native embedding model id, not a `model.*` ref
    /// (Decision D36).
    pub model: Spanned<String>,
    /// `provider:` — the connection that computes the vectors. A storage
    /// backend never embeds (Decision D116).
    pub provider: Spanned<Address>,
    /// `dimensions:` — asserted against the backend's index.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dimensions: Option<i64>,
    /// The block's own span.
    pub span: Span,
}

/// A `provider.*` definition: an inference connection (grammar 12.1).
///
/// Credential and connection fields hold `${ENV}` value-form references, which
/// reach the artifact unresolved: `validate` checks their syntax, and
/// `build`/`serve`/`run` check presence (grammar 4.3, PRD 5.9).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Provider {
    /// `kind:` — selects the plugin and closes this definition's key row
    /// (Decision D106).
    pub kind: ProviderKind,
    /// The keys the kind's row admits, each written under its own name. Only
    /// the ones the definition declared are present.
    #[serde(flatten)]
    pub config: ProviderConfig,
    /// `description:` — documentation only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
}

/// The per-kind keys of a provider (grammar 12.1).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProviderConfig {
    /// `api_key:` — an environment reference, never a literal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<Spanned<crate::ast::common::EnvRef>>,
    /// `base_url:` — an environment reference, never a literal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<Spanned<crate::ast::common::EnvRef>>,
    /// `access_key_id:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_key_id: Option<Spanned<crate::ast::common::EnvRef>>,
    /// `secret_access_key:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret_access_key: Option<Spanned<crate::ast::common::EnvRef>>,
    /// `session_token:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_token: Option<Spanned<crate::ast::common::EnvRef>>,
    /// `credentials_json:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials_json: Option<Spanned<crate::ast::common::EnvRef>>,
    /// `api_version:` — interpolable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_version: Option<Spanned<Interpolated>>,
    /// `organization:` — interpolable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<Spanned<Interpolated>>,
    /// `region:` — interpolable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region: Option<Spanned<Interpolated>>,
    /// `location:` — interpolable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Spanned<Interpolated>>,
    /// `project:` — interpolable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<Spanned<Interpolated>>,
    /// `profile:` — interpolable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<Spanned<Interpolated>>,
    /// `headers:` — extra request headers, interpolable values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<InterpolatedEntry>,
    /// `server_tools:` — the provider-side tools appended to the `tools` of
    /// every request this connection serves, in declaration order
    /// (Decision D122).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<ServerTool>,
}

/// One `server_tools:` entry, as the artifact carries it (grammar 12.1,
/// Decision D122).
///
/// A wire object rather than a construct of this grammar: `type:` is the only
/// key the compiler reads, and `config` is everything beside it — checked
/// against the curated table when the type is in it, and carried untouched
/// either way. The values are grammar 4.3 class 2, so each string reaches the
/// artifact as an [`Interpolated`] node that records the references it embeds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ServerTool {
    /// `type:` — the key the provider's own vocabulary is looked up under.
    #[serde(rename = "type")]
    pub type_name: Spanned<String>,
    /// Everything beside `type:`, sorted by key: a wire object is a set of
    /// fields rather than a sequence, and the artifact is canonical.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub config: BTreeMap<String, Spanned<crate::ast::deploy::PluginValue>>,
    /// The entry's own span.
    pub span: Span,
}

/// A `model.*` definition: a direct binding or a route, never both
/// (Decision D39). Tagged by `form`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum Model {
    /// A direct provider binding.
    Direct(DirectModel),
    /// An ordered failover route.
    Route(RouteModel),
}

/// The direct form of a model definition (grammar 12.2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DirectModel {
    /// `provider:` — the connection.
    pub provider: Spanned<Address>,
    /// `id:` — the provider-native model id; no env refs (grammar 4.3).
    pub id: Spanned<String>,
    /// `settings:` — the one open object in the logical layer (Decision D40),
    /// checked against the provider plugin's published schema. Sorted by key:
    /// the block is a set of settings, not a sequence.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, Spanned<Literal>>,
    /// `description:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
}

/// The route form of a model definition (grammar 12.2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouteModel {
    /// `route:` — at least two distinct `model.*` members, **in failover
    /// order**, which is why this list is not canonicalized.
    pub route: Vec<Spanned<Address>>,
    /// `route_on:` — absent means the default,
    /// `[rate_limit, overloaded, timeout]`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_on: Option<Vec<Spanned<RouteCondition>>>,
    /// `description:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
}
