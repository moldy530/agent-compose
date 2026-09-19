//! Flows, nodes, and edges in the IR (grammar 7, 8).
//!
//! Every node id an edge, a fallback, or a `human.on_timeout` names has been
//! resolved against the flow that declares it, and every typed address against
//! the composition — so the graph analyses that read this (SCC termination,
//! reachability, exhaustiveness, convergence) start from a graph whose vertices
//! and edges all exist.

use serde::Serialize;

use crate::ast::common::{
    Address, Cel, ControlTarget, Duration, EdgeSource, EdgeTarget, Ident, Literal, PathExpr,
};
use crate::ast::flow::{
    CoderWorkspace, FlowContext, Harness, PermissionMode, StoreOp, WorkspaceAccess,
};
use crate::diag::{Span, Spanned};

use super::binding::{Bindings, Exec, Http, InterpolatedEntry, NodeInput, Writes};
use super::policy::{Policy, Retry};
use super::schema::FieldMap;

/// A `flow.*` definition: a subgraph module with a declared I/O surface
/// (grammar 7).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Flow {
    /// `description:` — required only when the flow is used as an agent tool
    /// (Decision D26).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// `inputs:` — the module's parameter surface; absent means no inputs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inputs: Option<FieldMap>,
    /// `outputs:` — the module's result surface, read from the state channels
    /// of the same names at quiescence (grammar 7.5).
    pub outputs: FieldMap,
    /// `nodes:` — in declaration order.
    pub nodes: Vec<Node>,
    /// `edges:` — in declaration order, **which is semantic**: grammar 7.3
    /// evaluates a node's outgoing edges in it.
    pub edges: Vec<Edge>,
}

/// One node of a flow (grammar 7.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Node {
    /// The flow-local node id.
    pub id: Spanned<Ident>,
    /// `input:` — bindings; never present on a `map` node (grammar 8.6 rule 9).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<NodeInput>,
    /// `writes:` — write remap; never present on a `map` node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writes: Option<Writes>,
    /// `retry`/`timeout`/`on_error` for this node — level 2 of grammar 9.3.
    ///
    /// Written out as the three keys themselves, because that is how a node
    /// writes them (grammar 7.1) and because `policy` means something else on a
    /// node: a `flow:` node's `policy:` is the level-1 override for the nodes
    /// *inside* the instance, a different field of a different level
    /// (Decision D60). One key, one meaning.
    #[serde(flatten)]
    pub policy: Policy,
    /// `description:` — documentation only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// The node object's own span.
    pub span: Span,
    /// The kind key and its configuration.
    #[serde(flatten)]
    pub kind: NodeKind,
}

/// The nine node kinds (grammar 7.1, Decisions D23, D136), tagged by `kind`.
///
/// The variants differ in size because the constructs do — a `map:` block
/// carries a dispatch form, per-item bindings, and a policy, while an `agent:`
/// node is one address. Each is built once and then only read or written out,
/// so boxing the large ones would add indirection for no gain.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NodeKind {
    /// `agent: agent.*`
    Agent {
        /// The agent this node runs.
        agent: Spanned<Address>,
    },
    /// `coder: { ... }` — one run of a coding-agent harness (grammar 8.9).
    Coder {
        /// The block.
        coder: Coder,
    },
    /// `exec: { ... }` — an inline subprocess step (grammar 8.2).
    Exec {
        /// The block.
        exec: Exec,
    },
    /// `http: { ... }` — an inline request (grammar 8.3).
    Http {
        /// The block.
        http: Http,
    },
    /// `function: tool.*` — the graph-invoked use of a tool definition.
    Function {
        /// The tool this node invokes.
        function: Spanned<Address>,
    },
    /// `flow: flow.*` — subgraph instantiation (grammar 8.5).
    Flow {
        /// The flow this node instantiates.
        flow: Spanned<Address>,
        /// `context:` — absent means the default, `isolated`.
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<FlowContext>,
        /// `policy:` — level 1 of grammar 9.3 for every node *inside* the
        /// instance, distinct from this node's own policy (Decision D60).
        #[serde(skip_serializing_if = "Option::is_none")]
        policy: Option<Policy>,
    },
    /// `map: { ... }` — fan-out (grammar 8.6).
    Map {
        /// The block.
        map: Map,
    },
    /// `human: { ... }` — a human-in-the-loop pause (grammar 8.7).
    Human {
        /// The block.
        human: Human,
    },
    /// `store: store.*` — a store operation (grammar 8.8).
    Store {
        /// The store this node operates on.
        store: Spanned<Address>,
        /// The operation.
        op: StoreOp,
        /// The operation's parameters — exactly the op's row in grammar 11.4.
        params: StoreParams,
    },
}

/// A `coder:` block: one run of a coding-agent harness (grammar 8.9,
/// Decision D136, PRD resolved q57).
///
/// Everything a run needs is here because a coder node has no definition to
/// carry it: the harness that serves it, the model registry address the adapter
/// maps down, the workspace the run is contained by, its declared surfaces, the
/// environment its children see, and the harness config Decision D140 holds on
/// two tiers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Coder {
    /// `harness:` — which harness runs it.
    pub harness: Spanned<Harness>,
    /// `model:` — the registry address, resolved to an id by the adapter.
    pub model: Spanned<Address>,
    /// `workspace:` — the root the run works inside, per dispatch: an
    /// expression this node's input scope is evaluated against, or `fresh`
    /// (grammar 8.9, Decision D147, PRD resolved q61 ruling a).
    pub workspace: Spanned<CoderWorkspace>,
    /// `access:` — absent means the default, `workspace_write`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access: Option<WorkspaceAccess>,
    /// `permission_mode:` — the approval mode inside that containment; absent
    /// means the mode `access:` derives (Decision D146, PRD resolved q60 ruling
    /// a).
    ///
    /// Spanned, unlike `access:`, because the two refusals this key earns are
    /// both about the key rather than about the block: a harness with no
    /// approval axis, and a mode outside what the node's `access:` level admits.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_mode: Option<Spanned<PermissionMode>>,
    /// `prompt:` — literal text; there is no interpolation (grammar 5.2).
    pub prompt: Spanned<String>,
    /// `input:` — the declared surface; absent means string-in (grammar 5.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<FieldMap>,
    /// `output:` — the structured answer the output gate parses in full.
    pub output: FieldMap,
    /// `allow_tools:` — the harness tool names the run may use, in declaration
    /// order. Empty means the harness's own default set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allow_tools: Vec<Spanned<String>>,
    /// `env:` — the declared environment, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<InterpolatedEntry>,
    /// `inherit_env:` — absent means `false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inherit_env: Option<bool>,
    /// `settings:` — harness config by key, in **declaration order**: the block
    /// is a set of settings rather than a sequence, and a harness reads it by
    /// name (Decision D140).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<CoderSetting>,
    /// The block's own span.
    pub span: Span,
}

/// One entry of a coder node's `settings:` (Decision D140).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CoderSetting {
    /// The key, as the author wrote it.
    pub key: Spanned<String>,
    /// The value, as the author wrote it.
    pub value: Spanned<Literal>,
}

/// A `human:` block (grammar 8.7, Decision D52).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Human {
    /// `input:` — what the human is shown.
    pub input: FieldMap,
    /// `output:` — what they return; routable structured output.
    pub output: FieldMap,
    /// `timeout:` — jointly optional and jointly required with `on_timeout`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Spanned<Duration>>,
    /// `on_timeout:` — the route taken on expiry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_timeout: Option<Spanned<ControlTarget>>,
    /// The block's own span.
    pub span: Span,
}

/// The parameters of a store op (grammar 11.4).
///
/// Six of the nine are CEL; `top_k`, `limit`, and `content_type` are literals
/// (grammar 8.8), so a bound written here is readable without running the graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct StoreParams {
    /// `key:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<Spanned<Cel>>,
    /// `value:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<StoreValue>,
    /// `query:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<Spanned<Cel>>,
    /// `prefix:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prefix: Option<Spanned<Cel>>,
    /// `top_k:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<i64>,
    /// `limit:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<i64>,
    /// `filter:` — metadata field to CEL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<Bindings>,
    /// `metadata:` — metadata field to CEL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Bindings>,
    /// `content_type:` — a literal media type.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<Spanned<String>>,
}

/// A store op's `value:`: one expression on a `vector upsert` and a `blob put`,
/// a field map of expressions on a `kv set` (grammar 11.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum StoreValue {
    /// One CEL expression — the text or the content.
    Expression {
        /// The expression.
        value: Spanned<Cel>,
    },
    /// Field to CEL, matching the store's `value_schema`.
    Fields {
        /// The bindings.
        bindings: Bindings,
    },
}

/// A `map:` block: agent-controlled cardinality with deterministic dispatch
/// (grammar 8.6).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Map {
    /// `over:` — a path expression that must resolve to an array with
    /// `max_items`, produced by a node dominating this one (rules 1, 11).
    pub over: Spanned<PathExpr>,
    /// `as:` — the per-item binding name; absent means the default, `item`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_binding: Option<Spanned<Ident>>,
    /// `max_concurrency:` — the node-wide bound (Decision D28).
    pub max_concurrency: i64,
    /// `on_item_error:` — absent means the default, `fail`. The one per-item
    /// key that stays map-wide (grammar 8.6 rule 7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_item_error: Option<ItemError>,
    /// The block's own span.
    pub span: Span,
    /// The dispatch form: homogeneous, or discriminator-routed.
    #[serde(flatten)]
    pub dispatch: MapDispatch,
}

/// How a `map` dispatches (grammar 8.6 rule 2), tagged by `dispatch`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "dispatch", rename_all = "snake_case")]
pub enum MapDispatch {
    /// `node:` — one target for every item, with the per-target keys at map
    /// level (Decision D85).
    Homogeneous {
        /// The dispatch target: `agent.*`, `tool.*`, or `flow.*`.
        node: Spanned<Address>,
        /// `input:` — per-item bindings; absent means the whole item.
        #[serde(skip_serializing_if = "Option::is_none")]
        input: Option<NodeInput>,
        /// `writes:` — write remap; targets must be reduced channels.
        #[serde(skip_serializing_if = "Option::is_none")]
        writes: Option<Writes>,
        /// `detach:` — fire-and-forget dispatch. Kept spanned: whether it is
        /// legal depends on the active target (grammar 8.6 rule 7, D59).
        #[serde(skip_serializing_if = "Option::is_none")]
        detach: Option<Spanned<bool>>,
    },
    /// `route_by:` + `routes:` — one target per discriminator variant, each
    /// narrowed to its own variant's payload (rule 4).
    Routed {
        /// The literal discriminator field name, never CEL (rule 8).
        route_by: Spanned<Ident>,
        /// The named routes, in declaration order.
        routes: Vec<MapRoute>,
        /// The catch-all route, narrowed to the unrouted variants (D30).
        #[serde(skip_serializing_if = "Option::is_none")]
        default: Option<Box<MapRoute>>,
    },
}

/// One destination of a discriminator-routed map (grammar 8.6).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MapRoute {
    /// The variant tag this route serves; absent on the `default:` route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<Spanned<Ident>>,
    /// `node:` — the dispatch target.
    pub node: Spanned<Address>,
    /// `max_concurrency:` — at most the map's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<i64>,
    /// `input:` — per-item bindings for this route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<NodeInput>,
    /// `writes:` — write remap for this route.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub writes: Option<Writes>,
    /// `detach:` — fire-and-forget dispatch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detach: Option<Spanned<bool>>,
    /// The route object's own span.
    pub span: Span,
}

/// Per-item error strategy on a `map` node (grammar 8.6 rule 10, D73).
///
/// `Retry` is the large variant for the same reason `on_error:`'s `Fallback`
/// is: the strategy that takes a parameter carries it inline.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum ItemError {
    /// Fail the whole fan-out (the default).
    Fail {
        /// Where the keyword was written.
        span: Span,
    },
    /// Drop the failing item and carry on.
    Skip {
        /// Where the keyword was written.
        span: Span,
    },
    /// Retry the failing item under this policy, carried inline because there
    /// is no chain for a bare `retry` to inherit a bound from.
    Retry {
        /// The retry block.
        retry: Retry,
        /// Where the whole `on_item_error:` value was written.
        span: Span,
    },
}

/// One edge of a flow (grammar 7.2).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Edge {
    /// `from:` — a node id or `start`.
    pub from: Spanned<EdgeSource>,
    /// `to:` — a node id or `end`.
    pub to: Spanned<EdgeTarget>,
    /// `when:` — a CEL guard.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<Spanned<Cel>>,
    /// `else: true` — marks the default edge, taken iff no guarded sibling was
    /// (grammar 7.3 rule 4). Only `true` is legal (Decision D61).
    #[serde(rename = "else", skip_serializing_if = "Option::is_none")]
    pub else_edge: Option<Spanned<bool>>,
    /// `max_iterations:` — the cycle bound, legal only alongside `when:`
    /// (Decision D90).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<i64>,
    /// The edge object's own span.
    pub span: Span,
}

/// A `tool.*` implementation binding (grammar 6.1), tagged by `binding`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "binding", rename_all = "snake_case")]
pub enum ToolImplementation {
    /// A subprocess.
    Exec {
        /// The block.
        exec: Exec,
    },
    /// An HTTP request.
    Http {
        /// The block.
        http: Http,
    },
    /// A host-registered function — the escape hatch, and the one binding that
    /// makes a composition non-portable (grammar 6.1).
    Function {
        /// The registry lookup.
        function: super::binding::FunctionBinding,
    },
    /// A hand-authored TypeScript module inside the project — the first
    /// authored code the compiler holds to a contract (grammar 6.1, PRD
    /// resolved q48).
    Module {
        /// The file, the environment it may read, and what it imports.
        module: super::binding::Module,
    },
    /// One of the built-in tools, configured — the binding that hands the
    /// **model** the program (grammar 6.1, PRD resolved q54).
    Builtin {
        /// Which built-in, and the bounds it runs under.
        builtin: super::binding::Builtin,
    },
}
