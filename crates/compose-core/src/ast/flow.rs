//! Flows, nodes, and edges (grammar 7, 8).

use crate::diag::{Span, Spanned};

use super::binding::{Bindings, ExecBlock, HttpBlock, NodeInput, Writes};
use super::common::{
    Address, Cel, ControlTarget, Duration, EdgeSource, EdgeTarget, Ident, PathExpr,
};
use super::policy::{PolicyBlock, Retry};
use super::schema::FieldMap;

/// A `flow.*` definition: a subgraph module with a declared I/O surface
/// (grammar 7).
#[derive(Clone, Debug, PartialEq)]
pub struct FlowDef {
    /// `description:` — required only when the flow is used as an agent tool
    /// (Decision D26).
    pub description: Option<Spanned<String>>,
    /// `inputs:` — the module's parameter surface.
    pub inputs: Option<FieldMap>,
    /// `outputs:` — required; the module's result surface.
    pub outputs: Option<FieldMap>,
    /// `nodes:` — at least one, in declaration order.
    pub nodes: Vec<Node>,
    /// `edges:` — at least one, in declaration order (which is semantic:
    /// grammar 7.3 evaluates them in it).
    pub edges: Vec<Edge>,
}

/// One node of a flow (grammar 7.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The flow-local node id.
    pub id: Spanned<Ident>,
    /// Exactly one kind key.
    pub kind: NodeKind,
    /// `input:` — bindings; illegal on `map` nodes (grammar 8.6 rule 9).
    pub input: Option<NodeInput>,
    /// `writes:` — write remap; illegal on `map` nodes.
    pub writes: Option<Writes>,
    /// `retry`/`timeout`/`on_error` for this node.
    pub policy: PolicyBlock,
    /// `description:` — documentation only.
    pub description: Option<Spanned<String>>,
    /// The node object's own span.
    pub span: Span,
}

/// The eight node kinds (grammar 7.1, Decision D23).
///
/// The variants differ in size because the constructs do — a `map:` block
/// carries a dispatch form, per-item bindings, and a policy, while an `agent:`
/// node is one reference. Each is built once per node and matched by reference
/// from then on, so boxing the large ones would add indirection to every
/// consumer and buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// `agent: agent.*`
    Agent(Spanned<Address>),
    /// `exec: { ... }`
    Exec(ExecBlock),
    /// `http: { ... }`
    Http(HttpBlock),
    /// `function: tool.*`
    Function(Spanned<Address>),
    /// `flow: flow.*`
    Flow(FlowNode),
    /// `map: { ... }`
    Map(MapBlock),
    /// `human: { ... }`
    Human(HumanBlock),
    /// `store: store.*`
    Store(StoreNode),
    /// No kind key, or more than one. A diagnostic was reported; the node is
    /// kept so the flow's other nodes and its edges still parse.
    Invalid,
}

impl NodeKind {
    /// The kind key that selects this node kind.
    #[must_use]
    pub const fn key(&self) -> &'static str {
        match self {
            Self::Agent(_) => "agent",
            Self::Exec(_) => "exec",
            Self::Http(_) => "http",
            Self::Function(_) => "function",
            Self::Flow(_) => "flow",
            Self::Map(_) => "map",
            Self::Human(_) => "human",
            Self::Store(_) => "store",
            Self::Invalid => "<none>",
        }
    }
}

/// Every node kind key, in the order grammar 7.1 lists them.
pub const NODE_KIND_KEYS: &[&str] = &[
    "agent", "exec", "http", "function", "flow", "map", "human", "store",
];

/// Conversation-history scoping on a `flow:` node (grammar 8.5, Decision D27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowContext {
    /// A fresh history for the subflow (the default).
    Isolated,
    /// Share the caller's history.
    Inherit,
}

/// A `flow:` node: subgraph instantiation (grammar 8.5).
#[derive(Clone, Debug, PartialEq)]
pub struct FlowNode {
    /// The flow to instantiate.
    pub flow: Spanned<Address>,
    /// `context:` — defaults to `isolated`.
    pub context: Option<Spanned<FlowContext>>,
    /// `policy:` — the override applied to every node *inside* the instance,
    /// distinct from this node's own policy (Decision D60).
    pub policy: Option<Spanned<PolicyBlock>>,
}

/// A `human:` block (grammar 8.7, Decision D52).
#[derive(Clone, Debug, PartialEq)]
pub struct HumanBlock {
    /// `input:` — what the human is shown.
    pub input: Option<FieldMap>,
    /// `output:` — what they return; routable structured output.
    pub output: Option<FieldMap>,
    /// `timeout:` — jointly required with `on_timeout`.
    pub timeout: Option<Spanned<Duration>>,
    /// `on_timeout:` — jointly required with `timeout`.
    pub on_timeout: Option<Spanned<ControlTarget>>,
    /// The block's own span.
    pub span: Span,
}

/// The store operations (grammar 11.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreOp {
    /// `kv`/`blob`: read one key.
    Get,
    /// `kv`: write one key.
    Set,
    /// `kv`/`vector`/`blob`: remove one key.
    Delete,
    /// `kv`/`blob`: enumerate keys under a prefix.
    List,
    /// `vector`: similarity search.
    Search,
    /// `vector`: write one document.
    Upsert,
    /// `blob`: write one object.
    Put,
}

impl StoreOp {
    /// Every op, in the order grammar 11.4 lists them.
    pub const ALL: &'static [Self] = &[
        Self::Get,
        Self::Set,
        Self::Delete,
        Self::List,
        Self::Search,
        Self::Upsert,
        Self::Put,
    ];

    /// The keyword that names this op.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Set => "set",
            Self::Delete => "delete",
            Self::List => "list",
            Self::Search => "search",
            Self::Upsert => "upsert",
            Self::Put => "put",
        }
    }

    /// The parameters this op takes, required ones first — the exact row of
    /// grammar 11.4's catalogue (Decision D34).
    #[must_use]
    pub const fn required_parameters(self) -> &'static [&'static str] {
        match self {
            Self::Get | Self::Delete => &["key"],
            Self::Set | Self::Upsert | Self::Put => &["key", "value"],
            Self::List => &["limit"],
            Self::Search => &["query", "top_k"],
        }
    }

    /// The optional parameters this op takes.
    #[must_use]
    pub const fn optional_parameters(self) -> &'static [&'static str] {
        match self {
            Self::Get | Self::Delete | Self::Set => &[],
            Self::List => &["prefix"],
            Self::Search => &["filter"],
            Self::Upsert => &["metadata"],
            Self::Put => &["content_type"],
        }
    }
}

/// The `value:` of a store op: a field map for `kv set`, a single expression
/// for `vector upsert` and `blob put` (grammar 11.4).
#[derive(Clone, Debug, PartialEq)]
pub enum StoreValue {
    /// One CEL expression — the text or the content.
    Expression(Spanned<Cel>),
    /// Field to CEL, matching the store's `value_schema`.
    Fields(Bindings),
}

/// A `store:` node: a deterministic, graph-invoked store operation
/// (grammar 8.8).
#[derive(Clone, Debug, PartialEq)]
pub struct StoreNode {
    /// The store to operate on.
    pub store: Spanned<Address>,
    /// The operation.
    pub op: Option<Spanned<StoreOp>>,
    /// The operation's parameters.
    pub params: StoreOpParams,
}

/// The parameters of a store op. Which of them are legal is fixed per op by
/// grammar 11.4; the parser enforces that catalogue and leaves the schema
/// checks (does `value` match `value_schema`?) to the validator.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoreOpParams {
    /// `key:`
    pub key: Option<Spanned<Cel>>,
    /// `value:`
    pub value: Option<StoreValue>,
    /// `query:`
    pub query: Option<Spanned<Cel>>,
    /// `prefix:`
    pub prefix: Option<Spanned<Cel>>,
    /// `top_k:`
    pub top_k: Option<Spanned<i64>>,
    /// `limit:`
    pub limit: Option<Spanned<i64>>,
    /// `filter:`
    pub filter: Option<Bindings>,
    /// `metadata:`
    pub metadata: Option<Bindings>,
    /// `content_type:`
    pub content_type: Option<Spanned<String>>,
}

/// Per-item error strategy on a `map` node (grammar 8.6 rule 10,
/// Decision D73).
///
/// The shape is the enum-or-single-key-object one `on_error:` already uses: a
/// retry carries its policy **inline**, because a bare `retry` would name a
/// behaviour with no count and no backoff and there is no chain for it to
/// inherit one from — `defaults:` applies to nodes, and a dispatched instance
/// is not a node of this flow.
///
/// `Retry` is the large variant for the same reason `on_error:`'s `Fallback`
/// is: the strategy that takes a parameter carries it inline. Boxing §9.1's
/// block here would put an allocation behind a value that is written once and
/// then only read.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum ItemError {
    /// Fail the whole fan-out (the default).
    Fail,
    /// Drop the failing item and carry on.
    Skip,
    /// Retry the failing item under this policy; when the retries are
    /// exhausted the item resolves as `fail` does.
    Retry(Spanned<Retry>),
}

/// A `map:` block: agent-controlled cardinality with deterministic dispatch
/// (grammar 8.6).
#[derive(Clone, Debug, PartialEq)]
pub struct MapBlock {
    /// `over:` — a path expression resolving to an array with `max_items`.
    pub over: Option<Spanned<PathExpr>>,
    /// `as:` — the per-item binding name; defaults to `item`.
    pub item_binding: Option<Spanned<Ident>>,
    /// The dispatch form: homogeneous, or discriminator-routed.
    pub dispatch: MapDispatch,
    /// `max_concurrency:` — required (Decision D28).
    pub max_concurrency: Option<Spanned<i64>>,
    /// `on_item_error:` — defaults to `fail`. The one per-item key that stays
    /// map-wide, because it is a strategy rather than something typed against
    /// a target (grammar 8.6 rule 7).
    pub on_item_error: Option<Spanned<ItemError>>,
    /// `input:` — per-item bindings, homogeneous form only; without it the
    /// whole item is passed. Both forms are legal: a field map, or a bare
    /// scalar binding a string-in agent's single unnamed input (grammar 8.6
    /// rule 12, Decision D75).
    pub input: Option<NodeInput>,
    /// `writes:` — write remap, homogeneous form only; targets must be reduced
    /// channels (Decision D85).
    pub writes: Option<Writes>,
    /// `detach:` — homogeneous form only (Decisions D31, D85).
    pub detach: Option<Spanned<bool>>,
    /// The block's own span.
    pub span: Span,
}

/// How a `map` dispatches (grammar 8.6 rule 2).
#[derive(Clone, Debug, PartialEq)]
pub enum MapDispatch {
    /// `node:` — one target for every item.
    Homogeneous {
        /// The dispatch target: `agent.*`, `tool.*`, or `flow.*`.
        node: Spanned<Address>,
    },
    /// `route_by:` + `routes:` — one target per discriminator variant.
    Routed {
        /// The literal discriminator field name (never CEL — rule 8).
        route_by: Spanned<Ident>,
        /// The named routes, in declaration order.
        routes: Vec<MapRoute>,
        /// The catch-all route, narrowed to the unrouted variants
        /// (Decision D30).
        default: Option<Box<MapRoute>>,
    },
    /// Neither form, or both. A diagnostic was reported.
    Invalid,
}

/// One destination of a discriminator-routed map (grammar 8.6).
#[derive(Clone, Debug, PartialEq)]
pub struct MapRoute {
    /// The variant tag this route serves; `None` for the `default:` route.
    pub tag: Option<Spanned<Ident>>,
    /// `node:` — the dispatch target.
    pub node: Option<Spanned<Address>>,
    /// `max_concurrency:` — must be at most the map's.
    pub max_concurrency: Option<Spanned<i64>>,
    /// `input:` — per-item bindings for this route, in either form (grammar
    /// 8.6 rule 12, Decision D75).
    pub input: Option<NodeInput>,
    /// `writes:` — write remap for this route.
    pub writes: Option<Writes>,
    /// `detach:` — fire-and-forget dispatch.
    pub detach: Option<Spanned<bool>>,
    /// The route object's own span.
    pub span: Span,
}

/// One edge of a flow (grammar 7.2).
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    /// `from:` — a node id or `start`.
    pub from: Option<Spanned<EdgeSource>>,
    /// `to:` — a node id or `end`.
    pub to: Option<Spanned<EdgeTarget>>,
    /// `when:` — a CEL guard over the source node's output.
    pub when: Option<Spanned<Cel>>,
    /// `else: true` — marks the default edge. The span is of the `true`
    /// literal; no other value is legal (Decision D61).
    pub else_edge: Option<Span>,
    /// `max_iterations:` — the cycle bound, 1..=1000.
    pub max_iterations: Option<Spanned<i64>>,
    /// The edge object's own span.
    pub span: Span,
}
