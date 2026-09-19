//! Flows, nodes, and edges (grammar 7, 8).

use crate::diag::{Span, Spanned};

use super::binding::{Bindings, ExecBlock, HttpBlock, InterpolatedEntry, NodeInput, Writes};
use super::common::{
    Address, Cel, ControlTarget, Duration, EdgeSource, EdgeTarget, Ident, Interpolated, PathExpr,
};
use super::definition::Settings;
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

/// The nine node kinds (grammar 7.1, Decisions D23, D136).
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
    /// `coder: { ... }` — a coding-agent harness, run as one node (grammar 8.9).
    Coder(Box<CoderBlock>),
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
            Self::Coder(_) => "coder",
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
    "agent", "coder", "exec", "http", "function", "flow", "map", "human", "store",
];

/// The coding-agent harnesses a `coder:` node may bind (grammar 8.9,
/// Decision D136, PRD resolved q57).
///
/// A **closed** enum rather than an open name, and a *binding* rather than
/// grammar: the spec never names an SDK type, so a composition reads identically
/// whichever harness serves it, and a later harness is a member of this list
/// rather than a new construct. Two of the four are reserved: they are spelled
/// so that a composition written against them is refused by name and by scope
/// rather than by a typo's suggestion list, which is the same posture grammar 15
/// takes for every other reserved surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Harness {
    /// `cc` — the Claude Agent SDK. Active in v1.
    Cc,
    /// `codex` — the Codex SDK. Active in v1.
    Codex,
    /// `deepagents` — RESERVED; refused by `validate` naming v1's scope.
    DeepAgents,
    /// `native` — RESERVED; refused by `validate` naming v1's scope.
    Native,
}

impl Harness {
    /// Every harness, in the order grammar 8.9 lists them.
    pub const ALL: &'static [Self] = &[Self::Cc, Self::Codex, Self::DeepAgents, Self::Native];

    /// The keyword `harness:` names it with.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cc => "cc",
            Self::Codex => "codex",
            Self::DeepAgents => "deepagents",
            Self::Native => "native",
        }
    }

    /// The harness spelled `keyword`, if it is one.
    #[must_use]
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|harness| harness.as_str() == keyword)
    }

    /// Whether this release lowers the harness to a driver (PRD resolved q57).
    #[must_use]
    pub const fn ships_in_v1(self) -> bool {
        matches!(self, Self::Cc | Self::Codex)
    }
}

/// A `coder:` node's `workspace:` — a **runtime** binding (grammar 8.9, 4.1,
/// Decision D147, PRD resolved q61 ruling a).
///
/// The key shipped as a grammar 4.3 class-2 binding — literal or `${ENV}`,
/// fixed for the whole process — which is why a `map` over a coder node could
/// not actually fan out: every dispatch of it named one directory, and
/// `max_concurrency` above 1 was a data race on that directory. It is an
/// **expression** instead, evaluated in the node's input scope at each
/// dispatch, so a map item carries its own checkout.
///
/// The env refs stay, and they are resolved **first**: a value's `${NAME}`
/// tokens are substituted into the expression's source, and what results is
/// what CEL evaluates (grammar 4.3, and see the class table's one row that is
/// both). That is what keeps a machine-dependent root writable —
/// `"'${REPO_ROOT}/' + item.name"` — and what keeps `References::of` walking
/// this key for the environment manifests (PRD resolved q41) and the launch
/// check (q15).
#[derive(Clone, Debug, PartialEq)]
pub enum CoderWorkspace {
    /// `workspace: fresh` — one directory per dispatch, provisioned by the
    /// runtime under the execution's scratch and named by the §9.4 instance
    /// path (PRD resolved q61 ruling c).
    ///
    /// The one **word** this surface reads rather than an expression, and
    /// there is no ambiguity in that: a bare `fresh` is not an expression this
    /// grammar could evaluate — it names no root — so the spelling was free.
    /// A directory really called `fresh` is the string literal `'fresh'`.
    Fresh,
    /// Everything else: the expression, as written.
    Expression(Interpolated),
}

impl CoderWorkspace {
    /// The word `fresh`, which is the whole of this surface's keyword
    /// vocabulary.
    pub const FRESH: &'static str = "fresh";

    /// The value as the composition wrote it — `fresh`, or the expression's
    /// own text with its `${NAME}` references unresolved.
    ///
    /// What every surface that quotes this key prints: a diagnostic, the graph
    /// document (`docs/graph.md` §5.8), `plan`, and the journal's replay
    /// identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Fresh => Self::FRESH,
            Self::Expression(expression) => expression.as_str(),
        }
    }

    /// The expression, where the value is one.
    #[must_use]
    pub const fn expression(&self) -> Option<&Interpolated> {
        match self {
            Self::Fresh => None,
            Self::Expression(expression) => Some(expression),
        }
    }
}

/// What a coder node's harness may do inside its workspace (grammar 8.9,
/// Decision D138).
///
/// One vocabulary, mapped down per harness: `codex` has sandbox presets of
/// exactly this shape, and the Agent SDK has a permission surface these three
/// select a setting of. The names are this grammar's own — a spec never spells a
/// vendor's — and the mapping is the adapter's.
///
/// The sentence each variant carries is what a preset *means*; **what holds it
/// is stated per harness and never implied equivalent** (PRD resolved q57 ruling
/// c). The two statements are the tables in `docs/grammar.md` §8.9 and the two
/// drivers' own `CC_PERMISSION`/`CODEX_SANDBOX`, and they differ in kind: one
/// harness holds `ReadOnly` at the operating system and the other holds it with
/// a permission mode. An author reading only this enum has read half of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkspaceAccess {
    /// Read the workspace; write nothing.
    ReadOnly,
    /// Read and write inside the workspace (the default).
    WorkspaceWrite,
    /// No containment beyond the working directory the harness is started in.
    FullAccess,
}

impl WorkspaceAccess {
    /// Every access level, in the order grammar 8.9 lists them.
    pub const ALL: &'static [Self] = &[Self::ReadOnly, Self::WorkspaceWrite, Self::FullAccess];

    /// The keyword `access:` names it with.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::WorkspaceWrite => "workspace_write",
            Self::FullAccess => "full_access",
        }
    }

    /// The access level spelled `keyword`, if it is one.
    #[must_use]
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|access| access.as_str() == keyword)
    }
}

/// The **approval mode** a coder node's harness runs its own loop under
/// (grammar 8.9, Decision D146, PRD resolved q60 ruling a).
///
/// The one place this grammar spells a vendor's vocabulary, and it is deliberate.
/// [`WorkspaceAccess`] is *containment* — where a run may reach — and it is
/// `codex`-shaped because that is the harness whose primitive is named; the Agent
/// SDK carries a second, richer axis beside it, **how a call is approved**, whose
/// six modes the `access:` enum flattened onto three. Inventing three more names
/// of our own for the other half would be a translation with nothing to check it
/// against, which is the argument Decision D138 already made for taking `codex`'s
/// three; taken whole, the argument lands the other way here, so these are the
/// pinned SDK's own spellings and the per-harness table in [`crate::harness`] is
/// what says which harness has the axis at all.
///
/// **A mode is never a widening.** What a run may *do* is the node's `access:`,
/// and a mode may only choose among the approvals that level already admits —
/// which is the admissibility table in [`crate::harness::PERMISSION`], stated
/// normatively in grammar 8.9 and enforced at `validate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PermissionMode {
    /// Standard behaviour: the harness prompts for dangerous operations.
    Default,
    /// File edits are auto-accepted.
    AcceptEdits,
    /// Every permission check is bypassed.
    BypassPermissions,
    /// Planning: the loop reads and reports and executes no tool.
    Plan,
    /// Nothing is prompted for; anything not pre-approved is denied.
    DontAsk,
    /// A model classifier answers the permission prompts.
    Auto,
}

impl PermissionMode {
    /// Every mode, in the order the pinned Agent SDK's own `PermissionMode`
    /// declares them.
    ///
    /// The SDK's order rather than an order of our own, because this list is an
    /// audit of somebody else's type: a reader comparing the two reads them in
    /// one order or reads them twice.
    pub const ALL: &'static [Self] = &[
        Self::Default,
        Self::AcceptEdits,
        Self::BypassPermissions,
        Self::Plan,
        Self::DontAsk,
        Self::Auto,
    ];

    /// The keyword `permission_mode:` names it with, which is the SDK's own
    /// spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "acceptEdits",
            Self::BypassPermissions => "bypassPermissions",
            Self::Plan => "plan",
            Self::DontAsk => "dontAsk",
            Self::Auto => "auto",
        }
    }

    /// What the mode **does**, as the pinned SDK's own documentation states it —
    /// the sentence a diagnostic quotes back when a mode is refused.
    #[must_use]
    pub const fn decides(self) -> &'static str {
        match self {
            Self::Default => "prompts for dangerous operations",
            Self::AcceptEdits => "auto-accepts file edits",
            Self::BypassPermissions => "bypasses every permission check",
            Self::Plan => "plans, and executes no tool",
            Self::DontAsk => "prompts for nothing and denies what is not pre-approved",
            Self::Auto => "lets a model classifier answer the prompts",
        }
    }

    /// The mode spelled `keyword`, if it is one.
    #[must_use]
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|mode| mode.as_str() == keyword)
    }
}

/// A `coder:` block: one run of a coding-agent harness (grammar 8.9,
/// Decision D136, PRD resolved q57).
///
/// The surfaces sit *inside* the block, as a `human:` node's do, because the
/// construct has no definition of its own to carry them: a coder node is the
/// whole component, declared where it is used.
#[derive(Clone, Debug, PartialEq)]
pub struct CoderBlock {
    /// `harness:` — which harness runs it. Required.
    pub harness: Option<Spanned<Harness>>,
    /// `model:` — a `model.*` address, exactly as an agent node spells it.
    pub model: Option<Spanned<Address>>,
    /// `workspace:` — the root the run works inside. **Required**, and a
    /// runtime binding: an expression evaluated per dispatch, or `fresh`
    /// (grammar 4.1, 8.9, Decision D147).
    pub workspace: Option<Spanned<CoderWorkspace>>,
    /// `access:` — the containment preset; absent means `workspace_write`.
    pub access: Option<Spanned<WorkspaceAccess>>,
    /// `permission_mode:` — the approval mode inside that containment; absent
    /// means the mode `access:` derives (Decision D146).
    pub permission_mode: Option<Spanned<PermissionMode>>,
    /// `prompt:` — required, literal text (Decision D13's rule, one surface on).
    pub prompt: Option<Spanned<String>>,
    /// `input:` — the declared input surface; absent means string-in (§5.3).
    pub input: Option<FieldMap>,
    /// `output:` — required; the structured answer the gate parses.
    pub output: Option<FieldMap>,
    /// `allow_tools:` — the harness tool names the run may use.
    pub allow_tools: Vec<Spanned<String>>,
    /// `env:` — the scrubbed environment the harness runs with (q54 ruling b).
    pub env: Vec<InterpolatedEntry>,
    /// `inherit_env:` — absent means `false`.
    pub inherit_env: Option<Spanned<bool>>,
    /// `settings:` — harness config, held on Decision D140's two tiers.
    pub settings: Option<Settings>,
    /// The block's own span.
    pub span: Span,
}

/// Conversation-history scoping on a `flow:` node (grammar 8.5, Decision D27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlowContext {
    /// A fresh history for the subflow (the default).
    Isolated,
    /// Share the caller's history.
    Inherit,
}

impl FlowContext {
    /// The keyword that names this scoping.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Isolated => "isolated",
            Self::Inherit => "inherit",
        }
    }
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
