//! The graph document: the record types `agent-compose visualize --format json`
//! writes, and the data the embedded HTML template renders.
//!
//! `docs/graph.md` is the normative account of every one of them, under the
//! stability discipline `docs/trace.md` §10 gives the trace, and
//! `crates/compose-core/tests/graph_format_inventory.rs` is that account made
//! executable — a top-level key this module writes and that document does not
//! name is a test failure, because the document is what a consumer pins
//! `graph_version` on.
//!
//! # Why the leaves are strings
//!
//! The IR is faithful to the composition (see [`crate::ir`]): a duration is a
//! [`Duration`](crate::ast::common::Duration) carrying its amount and its unit,
//! an address is an [`Address`](crate::ast::common::Address) carrying its
//! namespace, an expression is a [`Cel`](crate::ast::common::Cel) that has
//! never been read. This document is *derived* from that one and is read by a
//! renderer, so every one of those leaves reaches it as the text the author
//! wrote — `"90s"`, `"agent.triage"`, `"state.patches"`. Nothing here is a
//! second spelling: the strings are the IR's own `as_str()`, so a value in this
//! document is the value in the spec, and a consumer that wants the structured
//! form reads the IR.
//!
//! The two exceptions are both counts the grammar fixes as integers —
//! `max_concurrency`, `max_items` and their kin — and a `settings:` value, which
//! is an arbitrary literal and reaches JSON as JSON.

use serde::Serialize;
use serde_json::Value;

/// Declares one of this document's **closed vocabularies**: the enumeration, and
/// the `ALL` list every bind over it reads, out of one declaration.
///
/// `docs/graph.md` §9.1 makes a reader entitled to the members of each of these
/// enumerations and §9.3 makes adding one a version bump, which are promises
/// about a list that has to be *in* that document — so
/// `crates/compose-core/tests/graph_format_inventory.rs` checks every member is
/// written out there, and the two vocabularies the canvas *draws* with,
/// [`NodeKind`] and [`EdgeClass`], are additionally held to the page's own `KIND`
/// and `EDGE` tables (`the_pages_kind_table_names_every_kind_and_badges_it_the_same`,
/// `the_pages_edge_table_names_every_class`).
/// A bind like that is worth exactly as much as the list it runs over: a
/// hand-kept one goes stale the moment a variant is added beside it, and every
/// test over it keeps passing while the documentation and the page have never
/// heard of the new member. Generating the list from the same declaration is
/// what makes it the enumeration rather than a second inventory of it — the
/// shape resolved q23 gives `explain`, where an exhaustive `match` over
/// `DiagnosticCode` makes a new failure class fail to compile until it has an
/// explanation.
macro_rules! vocabulary {
    (
        $(#[$enumeration:meta])*
        pub enum $name:ident {
            $($(#[$member:meta])* $variant:ident),+ $(,)?
        }
    ) => {
        $(#[$enumeration])*
        pub enum $name {
            $($(#[$member])* $variant),+
        }

        impl $name {
            /// Every member, in declaration order.
            ///
            /// Written by the declaration above rather than beside it, so a
            /// member cannot be absent from it.
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];
        }
    };
}

/// The shape of the document this module emits.
///
/// Independent of [`IR_VERSION`](crate::ir::IR_VERSION), of
/// [`PLAN_VERSION`](crate::plan::PLAN_VERSION) and of the `spec_version` a
/// composition declares: a graph document is a rendering surface *over* an
/// artifact and changes for reasons of its own. It is the first key of the
/// document, so a consumer can dispatch on it before reading anything else, and
/// a consumer that does not know a version must refuse the document rather than
/// guess.
/// Version `2` added a **node kind**: `coder`, PRD resolved q57's coding-agent
/// harness node. [`NodeKind`] is one of §9.1's closed vocabularies and §9.3
/// makes a member added to one a bump, not an addition — a reader written
/// against `1` was entitled to the nine kinds that document named, and a tenth
/// makes it wrong. Everything else the kind brought — [`CoderView`] and the
/// [`GraphNode::coder`] key that reaches it — would have been compatible on its
/// own under §9.2, and rides this bump because it arrived with the member that
/// forced one.
pub const GRAPH_VERSION: u32 = 2;

/// One composition's flows, as a picture of them.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphDocument {
    /// The shape of this document — always [`GRAPH_VERSION`] when this compiler
    /// wrote it.
    pub graph_version: u32,
    /// The entrypoint's path, relative to the project root, exactly as the IR
    /// records it.
    pub entrypoint: String,
    /// The deploy target the composition was resolved for.
    pub target: String,
    /// The DSL version the composition declares.
    pub spec_version: String,
    /// Every flow this document covers, sorted by address — or the one flow
    /// `--flow` narrowed to.
    pub flows: Vec<FlowGraph>,
}

impl GraphDocument {
    /// The document as pretty-printed JSON, with a trailing newline.
    ///
    /// Deterministic by construction: every collection here is either in the
    /// order the composition declares it or sorted by a name, so the same
    /// artifact answers the same bytes (PRD 5.12, §9.15).
    ///
    /// # Errors
    ///
    /// Returns the serializer's error. Every value this document can hold came
    /// out of the IR, which [`Ir::to_json`](crate::ir::Ir::to_json) already
    /// establishes is representable; it is surfaced rather than swallowed all
    /// the same.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }

    /// The flow at this address, if the document carries one.
    #[must_use]
    pub fn flow(&self, address: &str) -> Option<&FlowGraph> {
        self.flows.iter().find(|flow| flow.address == address)
    }
}

/// One flow, as one canvas.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlowGraph {
    /// The flow's typed address, `flow.<name>`.
    pub address: String,
    /// `description:` — absent where the flow declares none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `inputs:` — the module's parameter surface, in declaration order.
    pub inputs: Vec<FieldView>,
    /// `outputs:` — the module's result surface, in declaration order.
    pub outputs: Vec<FieldView>,
    /// Every declared trigger that targets this flow, in trigger-name order.
    pub triggers: Vec<TriggerView>,
    /// The nodes: the flow's own, the two pseudo-nodes, and one satellite per
    /// `map` dispatch.
    pub nodes: Vec<GraphNode>,
    /// Every edge an execution could take.
    pub edges: Vec<GraphEdge>,
}

/// One field of a declared schema, as a line a reader can take in.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FieldView {
    /// The field name.
    pub name: String,
    /// Its type, summarized — `string (min_length 1)`,
    /// `array<string> (max_items 200)`, `union on kind`.
    #[serde(rename = "type")]
    pub ty: String,
    /// `description:` — absent where the field declares none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One declared schema, and where it came from.
///
/// A node that declares none still *has* one wherever the grammar supplies a
/// kind default (an inline `exec:`'s `{ exit_code, stdout }`, an inline
/// `http:`'s `{ status, body }`), so the source is carried beside the fields:
/// an author reading a picture of their graph should not have to remember which
/// of the two they are looking at.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SchemaView {
    /// Where the shape comes from.
    pub source: SchemaSource,
    /// The fields, in declaration order.
    pub fields: Vec<FieldView>,
}

vocabulary! {
    /// Where a [`SchemaView`]'s shape comes from.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum SchemaSource {
        /// The composition declares it.
        Declared,
        /// The construct's kind supplies it (grammar 8.2, 8.3).
        KindDefault,
        /// An agent that declares no `input:` takes one unnamed string
        /// (grammar 5.3), so there are no fields to name.
        StringInput,
    }
}

/// One declared trigger, as the flow it targets sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TriggerView {
    /// The trigger's name.
    pub name: String,
    /// `manual`, `http`, `schedule`, or `event`.
    #[serde(rename = "type")]
    pub ty: String,
    /// The one line that says how an execution arrives — `POST /reports`,
    /// `cron 0 3 * * * (UTC)`, `source bug_reports`.
    pub summary: String,
    /// `session_key:` — the CEL supplying session identity, where the trigger
    /// declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<String>,
    /// `description:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `input:` — flow input field to CEL over `payload`, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub input: Vec<BindingView>,
}

/// One node of a canvas.
///
/// A flat record with one optional block per kind rather than a tagged union:
/// the renderer reads it, and a renderer that had to match on a tag before it
/// could ask whether there is a policy to draw would be a second switch over
/// the same eight kinds. [`Self::kind`] is the tag a reader dispatches on; the
/// blocks below it are present exactly where that kind has one.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphNode {
    /// The node's id on this canvas: a flow-local node id, `start`, `end`, or a
    /// map satellite's `<map id>/<variant>` — a spelling no node id can take,
    /// because a node id is an identifier (grammar 2.1).
    pub id: String,
    /// Which of grammar 7.1's kinds this node is, plus the two pseudo-nodes.
    pub kind: NodeKind,
    /// What this node is bound to: a typed address, or the one line an inline
    /// block is read by (`POST https://…`, `${OPS_BIN}/log-event`). Absent on
    /// the pseudo-nodes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    /// `description:` — the node's own, or the definition's where the node
    /// declares none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `input:` — the node's bindings, resolved to the form they were written
    /// in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<InputView>,
    /// `writes:` — output field to state channel, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<WriteView>,
    /// The node's policy, resolved through grammar 9.3's chain.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<PolicyView>,
    /// What this node is handed, and what it answers with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schemas: Option<SchemasView>,
    /// An `agent:` node's resolved configuration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentView>,
    /// A `coder:` node's harness run.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coder: Option<CoderView>,
    /// An inline `exec:` block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exec: Option<ExecView>,
    /// An inline `http:` block, or the one a `function:` node's tool binds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub http: Option<HttpView>,
    /// A `function:` node's tool, whatever it is bound to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolView>,
    /// A `store:` node's operation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store: Option<StoreView>,
    /// A `flow:` node's instantiation — the jump target, and what crosses the
    /// boundary.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subflow: Option<SubflowView>,
    /// A `human:` node's wait.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub human: Option<HumanView>,
    /// A `map:` node's fan-out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub map: Option<MapView>,
    /// A satellite's place in its map: which map dispatched it, and on what.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<DispatchView>,
}

vocabulary! {
    /// Which kind of node this is (grammar 7.1, PRD 5.5), plus the two
    /// pseudo-nodes every flow's edges name.
    ///
    /// Every kind renders distinguishably, which is the content requirement PRD
    /// resolved q56 states: a reader must be able to tell an agent from a tool
    /// from a store operation without opening the spec.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum NodeKind {
        /// The `start` pseudo-node.
        Start,
        /// The `end` pseudo-node.
        End,
        /// `agent: agent.*`.
        Agent,
        /// `coder: { … }` — one run of a coding-agent harness (grammar 8.9).
        Coder,
        /// `function: tool.*` — the graph-invoked use of a tool definition.
        Function,
        /// `exec: { … }`.
        Exec,
        /// `http: { … }`.
        Http,
        /// `human: { … }`.
        Human,
        /// `store: store.*`.
        Store,
        /// `flow: flow.*` — a subgraph instantiation.
        Flow,
        /// `map: { … }` — a fan-out.
        Map,
    }
}

impl NodeKind {
    /// The badge a canvas prints on this kind.
    #[must_use]
    pub const fn badge(self) -> &'static str {
        match self {
            Self::Start => "START",
            Self::End => "END",
            Self::Agent => "AGENT",
            Self::Coder => "CODER",
            Self::Function => "TOOL",
            Self::Exec => "EXEC",
            Self::Http => "HTTP",
            Self::Human => "HUMAN",
            Self::Store => "STORE",
            Self::Flow => "SUBGRAPH",
            Self::Map => "MAP",
        }
    }
}

/// A node's input bindings (grammar 8.0).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "form", rename_all = "snake_case")]
pub enum InputView {
    /// `input: "input.goal"` — one unnamed value.
    Scalar {
        /// The expression supplying it.
        value: String,
    },
    /// `input: { goal: "input.goal" }` — per-field bindings.
    Fields {
        /// The bindings, in declaration order.
        bindings: Vec<BindingView>,
    },
}

/// One name bound to one expression.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BindingView {
    /// The bound name.
    pub name: String,
    /// The CEL expression bound to it, as written.
    pub expression: String,
}

/// One `writes:` remap.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WriteView {
    /// An output field of the node.
    pub field: String,
    /// The state channel it is written to.
    pub channel: String,
    /// The channel's `reduce:` policy, where it declares one — which is what
    /// makes a write from inside a `map` legal (grammar 10.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce: Option<String>,
}

/// The two schemas a node has.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SchemasView {
    /// What the node is handed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<SchemaView>,
    /// What it answers with.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<SchemaView>,
}

/// One node's `retry`/`timeout`/`on_error`, resolved through grammar 9.3.
///
/// Levels 2 through 4 only, and the level each field came from is carried
/// beside it. Level 1 — a `flow:` node's `policy:` — belongs to the
/// *instantiation site* and one flow may be instantiated from several, so it is
/// reported on the instantiating node ([`SubflowView::policy`]) rather than
/// folded into the nodes inside the instance, which have no single answer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PolicyView {
    /// `retry:` — absent where the chain resolves to none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryView>,
    /// `timeout:` — absent where the chain resolves to none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<TimeoutView>,
    /// `on_error:` — always resolved; the built-in is `fail`.
    pub on_error: OnErrorView,
}

/// A resolved `retry:` block.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RetryView {
    /// Additional attempts after the first.
    pub max: i64,
    /// The initial delay, as written.
    pub backoff: String,
    /// The exponential factor, where the block declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<f64>,
    /// The cap on a single delay, where the block declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_backoff: Option<String>,
    /// Full jitter, where the block declares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter: Option<bool>,
    /// Which level of the chain supplied it.
    pub level: PolicyLevel,
}

/// A resolved `timeout:`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TimeoutView {
    /// The budget, as written.
    pub value: String,
    /// Which level of the chain supplied it.
    pub level: PolicyLevel,
}

/// A resolved `on_error:`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OnErrorView {
    /// `fail`, `skip`, or `fallback`.
    pub strategy: String,
    /// The fallback's target — a node id, or `end`. Present on `fallback` and
    /// nowhere else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Which level of the chain supplied it.
    pub level: PolicyLevel,
}

vocabulary! {
    /// Which level of grammar 9.3's chain a resolved field came from.
    ///
    /// The chain's fourth outcome — *exempt*, a `human` node's `timeout:` and
    /// `retry:`, which resolve at no level at all (Decision D102) — is not a
    /// member here: it never accompanies a value, and this document writes it as
    /// the key's **absence** rather than as a level (`docs/graph.md` §5.2). A
    /// member a document can never carry is a branch a consumer writes and never
    /// reaches.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum PolicyLevel {
        /// Level 2: the node's own key.
        Node,
        /// Level 3: the composition's `defaults:`.
        Defaults,
        /// Level 4: the built-in.
        BuiltIn,
    }
}

/// An `agent:` node's resolved configuration.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentView {
    /// The agent's address.
    pub address: String,
    /// The model this agent calls, resolved through to its provider.
    pub model: ModelView,
    /// Every tool on the wire: declared, built-in, and synthesized from an
    /// attached store — in that order.
    pub tools: Vec<ToolView>,
    /// `stores:` — the attached stores, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stores: Vec<String>,
    /// `prompt:` — verbatim.
    pub prompt: String,
    /// `max_tool_iterations:` — absent means the default, `8` (Decision D51).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_iterations: Option<i64>,
}

/// A `model.*` resolved: the direct binding, or the route and its members.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelView {
    /// The model's address.
    pub address: String,
    /// `direct` or `route`.
    pub form: String,
    /// `description:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The provider-native model id — the direct form only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The connection — the direct form only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderView>,
    /// `settings:` sorted by key — the direct form only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingView>,
    /// `route:` — the failover members **in failover order**, each resolved the
    /// same way. The route form only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub route: Vec<ModelView>,
    /// `route_on:` — the conditions failover happens on, where the route
    /// declares them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub route_on: Vec<String>,
}

/// The connection a model binds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProviderView {
    /// The provider's address.
    pub address: String,
    /// `kind:` — which plugin serves it.
    pub kind: String,
    /// The connection keys this provider declares, sorted by key, with every
    /// `${ENV}` reference left exactly as written (PRD 5.9).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub config: Vec<SettingView>,
    /// `server_tools:` — the provider-side tools appended to every request this
    /// connection serves, by `type:`, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<String>,
}

/// One `settings:` entry, or one provider connection key.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SettingView {
    /// The key.
    pub name: String,
    /// Its value: JSON for a `settings:` literal, the text as written for a
    /// connection key that may embed `${ENV}` references.
    pub value: Value,
}

/// One tool: on an agent's wire, or invoked as a `function:` node.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ToolView {
    /// The name the model calls it by — a tool's local name, a built-in's
    /// provider-dictated name, or a synthesized store tool's
    /// `<store>_<op>` (grammar 11.5).
    pub name: String,
    /// Where this tool came from.
    pub source: ToolSource,
    /// The typed address it was attached by, where there is one: a `tool.*`, a
    /// `flow.*` used as a tool, or the `store.*` that synthesized it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// Which implementation a `tool.*` is bound to: `exec`, `http`, `function`,
    /// `module`, or `builtin` (grammar 6.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<String>,
    /// The store op a synthesized tool performs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub op: Option<String>,
    /// `description:` — the LLM's selection signal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The one line the implementation is read by, where it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

vocabulary! {
    /// Where a [`ToolView`] came from.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum ToolSource {
        /// A `tool.*` in the agent's `tools:` list.
        Tool,
        /// A `flow.*` in the same list, used as a tool (PRD resolved q19).
        Flow,
        /// A `builtin.*` shorthand entry (grammar 5.5, PRD resolved q54).
        Builtin,
        /// Synthesized from an attached store (grammar 11.5, PRD 5.8).
        Store,
    }
}

/// An inline `exec:` block, or the one a tool binds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ExecView {
    /// `command:` — argv[0], never shell-interpreted.
    pub command: String,
    /// `args:` — literal argv entries, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// `cwd:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// `env:` — added to the child environment, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<BindingView>,
    /// `expect_exit:` — the accepted exit statuses, where declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expect_exit: Vec<i64>,
}

/// An inline `http:` block, or the one a tool binds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HttpView {
    /// `method:`.
    pub method: String,
    /// `url:`, with every `${ENV}` reference as written.
    pub url: String,
    /// `headers:`, in declaration order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<BindingView>,
    /// `query:` — parameter name to CEL.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub query: Vec<BindingView>,
    /// `body:` — field name to CEL.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<BindingView>,
    /// `expect_status:` — the accepted response statuses, where declared.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expect_status: Vec<i64>,
}

/// A `store:` node's operation (grammar 8.8, 11.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StoreView {
    /// The store's address.
    pub address: String,
    /// `kind:` — `kv`, `vector`, or `blob`.
    pub kind: String,
    /// `scope:` — the lifetime.
    pub scope: String,
    /// `op:` — which operation this node performs.
    pub op: String,
    /// `backend:` — the abstract alias the active target resolves.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// The operation's parameters, in grammar 11.4's own order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<BindingView>,
}

/// A `flow:` node's instantiation (grammar 8.5).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SubflowView {
    /// The flow this node instantiates — the canvas a jump link opens.
    pub address: String,
    /// `context:` — absent means the default, `isolated` (PRD 5.7).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// `policy:` — level 1 of grammar 9.3, applying to every node *inside* this
    /// instance (Decision D60).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<InstancePolicyView>,
}

/// An instantiation-site `policy:` override — grammar 9.3's level 1, as
/// written.
///
/// Not a [`PolicyView`]: nothing is resolved here, because level 1 *is* the
/// override. What it resolves against is every node inside the instance, and
/// those live on another canvas.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct InstancePolicyView {
    /// `retry:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryView>,
    /// `timeout:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// `on_error:` — `fail` or `skip`; level 1 takes no `fallback`
    /// (Decision D103).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
}

/// A `human:` node's wait (grammar 8.7).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanView {
    /// `timeout:` — the wait's budget, where the block declares one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<String>,
    /// `on_timeout:` — where control transfers on expiry: a node id, or `end`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_timeout: Option<String>,
}

/// A `coder:` node's harness run (grammar 8.9, PRD resolved q57).
///
/// The **harness is named**, which is PRD resolved q56's content requirement
/// read for this kind: two coder nodes side by side are two different agent
/// loops with two different containment stories, and a picture that drew them
/// identically would be hiding the one fact a reader most needs.
///
/// `access` carries the level the run is contained at — the preset `codex` maps
/// to a sandbox and `cc` to a permission mode — and `tools_enforced` says
/// whether the harness enforces `allow_tools` **in its own loop**. That
/// asymmetry is a fact about the harness rather than about the composition
/// (grammar 8.9, Decision D138), and a reader comparing two coder nodes cannot
/// derive it from anything else in the document.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CoderView {
    /// `harness:` — which harness runs it.
    pub harness: String,
    /// `model:` — the registry address the adapter maps down to a model id.
    pub model: ModelView,
    /// `workspace:` — the root the run works inside, exactly as written.
    pub workspace: String,
    /// `access:` — the containment preset, with the default materialized:
    /// omitting the key is `workspace_write`, and a picture answers the
    /// question rather than leaving it.
    pub access: String,
    /// `prompt:` — the run's system instructions, verbatim.
    pub prompt: String,
    /// `allow_tools:` — the harness tool names the run may use, in declaration
    /// order. Absent where the node declares none, which is the harness's own
    /// default set.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub allow_tools: Vec<String>,
    /// Whether this harness enforces `allow_tools` inside its own loop.
    /// `cc` does, through its per-call permission callback; `codex` bounds at
    /// the sandbox only (grammar 8.9, PRD resolved q57 ruling c).
    pub tools_enforced: bool,
    /// `env:` — the declared environment, in declaration order. Absent where
    /// the node declares none, which is a wholly scrubbed child environment.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<BindingView>,
    /// `inherit_env:` — with the default materialized, for the reason `access`
    /// is: `false` is the containment claim, and a reader should not have to
    /// know it is the default to read it.
    pub inherit_env: bool,
    /// `settings:` sorted by key — the harness config, exactly as a model's
    /// `settings:` reach this document.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingView>,
}

/// A `map:` node's fan-out (grammar 8.6, PRD 5.6).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MapView {
    /// `over:` — the path to the array this map fans out over.
    pub over: String,
    /// The `max_items` bound the array declares — the cardinality half of
    /// mandatory bounding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_items: Option<i64>,
    /// `as:` — the per-item binding name; absent means the default, `item`.
    pub item_binding: String,
    /// `max_concurrency:` — the node-wide bound.
    pub max_concurrency: i64,
    /// `homogeneous` or `routed`.
    pub dispatch: String,
    /// `route_by:` — the discriminator field, on a routed map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route_by: Option<String>,
    /// `on_item_error:` — absent means the default, `fail`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_item_error: Option<ItemErrorView>,
    /// What the join barrier waits on, in one sentence.
    pub join: String,
    /// Every route: the named ones in declaration order, then `default:`.
    pub routes: Vec<RouteView>,
}

/// A `map`'s per-item error strategy (grammar 8.6 rule 10).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemErrorView {
    /// `fail`, `skip`, or `retry`.
    pub strategy: String,
    /// The retry block a `retry` strategy carries inline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryView>,
}

/// One destination of a `map` (grammar 8.6).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RouteView {
    /// The satellite node this route dispatches to, by its canvas id.
    pub node: String,
    /// The variant tag this route serves; absent on `default:` and on a
    /// homogeneous map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Whether this is the `default:` route.
    pub default: bool,
    /// Which union variants this route's target is narrowed to — its own tag,
    /// or, for `default:`, every variant no named route claims (grammar 8.6
    /// rule 4, Decision D30). Empty on a homogeneous map, whose item type is
    /// not a union.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// `node:` — the dispatch target's typed address.
    pub target: String,
    /// `max_concurrency:` — this route's own bound, at most the map's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<i64>,
    /// `input:` — per-item bindings; absent means the whole item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<InputView>,
    /// `writes:` — the write remap, whose targets must be reduced channels.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<WriteView>,
    /// `detach:` — fire-and-forget dispatch, where the route declares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detach: Option<bool>,
}

/// A satellite node's place in the map that dispatches it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DispatchView {
    /// The map node's id.
    pub map: String,
    /// The variant this satellite serves; absent on `default:` and on a
    /// homogeneous map.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Whether this satellite is the `default:` route's target.
    pub default: bool,
    /// The variants it is narrowed to (see [`RouteView::covers`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
    /// `max_concurrency:` — this route's own bound.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<i64>,
    /// `detach:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detach: Option<bool>,
}

/// One edge of a canvas: a declared edge, a map route, or one of the two
/// control-transfer positions grammar 7.8 counts alongside edges.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct GraphEdge {
    /// The node id control leaves.
    pub from: String,
    /// The node id it arrives at.
    pub to: String,
    /// Which kind of transfer this is.
    pub class: EdgeClass,
    /// The chip a canvas draws on it, where it has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `when:` — the CEL guard, on a conditional edge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// `max_iterations:` — the cycle bound a guarded back-edge carries
    /// (Decision D90).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_iterations: Option<i64>,
    /// Whether this edge is a `map`'s join barrier: it fires when every
    /// dispatched instance has completed or been resolved by `on_item_error`
    /// (PRD 5.6).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub join_barrier: bool,
}

vocabulary! {
    /// Which kind of transfer an edge is.
    ///
    /// Every routing decision an execution could take is one of these, which is
    /// the promise the canvas makes: what you see is every path the validator
    /// proved (PRD resolved q56).
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
    #[serde(rename_all = "snake_case")]
    pub enum EdgeClass {
        /// A declared edge with no `when:` and no `else:` — it always fires.
        Unconditional,
        /// A declared edge carrying a CEL `when:` guard (grammar 7.3).
        Conditional,
        /// A declared edge marked `else: true` — taken iff no guarded sibling was
        /// (grammar 7.3 rule 4).
        Default,
        /// A `map`'s dispatch to one of its routes (grammar 8.6).
        MapRoute,
        /// `on_error: { fallback: … }` — control transfers instead of this node's
        /// own edges being evaluated (grammar 9.2, 7.8).
        ErrorFallback,
        /// `human.on_timeout:` — control transfers when the wait runs out
        /// (grammar 8.7, 7.8).
        Timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind has a badge, and no two share one: the badge is how a reader
    /// tells the kinds apart on the canvas, which is the content requirement
    /// PRD resolved q56 states.
    ///
    /// Over [`NodeKind::ALL`], which the declaration of the enumeration writes —
    /// so a kind added to it is a kind this runs over rather than one a list
    /// beside the enumeration never heard of.
    #[test]
    fn every_node_kind_has_its_own_badge() {
        let mut seen = std::collections::BTreeSet::new();
        for kind in NodeKind::ALL {
            assert!(
                seen.insert(kind.badge()),
                "`{}` is two kinds' badge",
                kind.badge()
            );
        }
        assert_eq!(seen.len(), NodeKind::ALL.len());
    }

    /// …and the page's own table names exactly those kinds, with those badges.
    ///
    /// The canvas draws `KIND[node.kind].label` out of an object literal in
    /// `template.html`, so that table and this enumeration are two spellings of
    /// one list — and the template **falls back**, `KIND[node.kind] ||
    /// KIND.map`, so a kind the table had never heard of would be drawn with
    /// the map's colour under the badge `MAP` rather than failing. Two kinds
    /// collapsed into one is precisely the distinguishability PRD resolved q56
    /// requires, and nothing else would notice, so the two sides are bound
    /// here: adding a variant fails this test until the page learns to draw it.
    ///
    /// Which it does because the list it runs over is [`NodeKind::ALL`],
    /// generated by the enumeration's own declaration. A hand-kept list here
    /// would have made this test pass on exactly the change it exists to catch.
    #[test]
    fn the_pages_kind_table_names_every_kind_and_badges_it_the_same() {
        let table = kind_table();
        let mut expected: Vec<(String, String)> = NodeKind::ALL
            .iter()
            .map(|kind| (member(kind), kind.badge().to_string()))
            .collect();
        expected.sort();

        let mut found: Vec<(String, String)> = table
            .iter()
            .map(|(key, label)| (key.clone(), label.clone()))
            .collect();
        found.sort();

        assert_eq!(
            found, expected,
            "`template.html`'s `KIND` table and `NodeKind` disagree. The canvas draws \
             `KIND[node.kind].label`, so a kind missing there is drawn as a `map`"
        );
    }

    /// The `(key, label)` pairs of the page's `KIND` object literal.
    fn kind_table() -> Vec<(String, String)> {
        let template = crate::graph::TEMPLATE;
        let from = template
            .find("const KIND = {")
            .expect("the page carries its kind table");
        let rest = &template[from..];
        let to = rest.find("};").expect("the table is closed");
        rest[..to]
            .lines()
            .filter_map(|line| {
                let (key, rest) = line.split_once(':')?;
                let label = rest.split_once("label: \"")?.1.split_once('"')?.0;
                Some((key.trim().to_string(), label.to_string()))
            })
            .collect()
    }

    /// …and the page's `EDGE` table names every class, for the same reason.
    ///
    /// `EDGE[edge.class]` supplies the arrowhead the canvas draws and the chip
    /// the detail pane names the transfer by, and it falls back to
    /// `EDGE.unconditional` — so a class the table had never heard of would be
    /// drawn as a plain solid edge with nothing said about it, which is a routing
    /// decision rendered as if it were unconditional. Bound over
    /// [`EdgeClass::ALL`], so adding a class fails this test until the page draws
    /// it.
    #[test]
    fn the_pages_edge_table_names_every_class() {
        let mut expected: Vec<String> = EdgeClass::ALL.iter().map(member).collect();
        expected.sort();
        let mut found = table_keys("const EDGE = {");
        found.sort();
        assert_eq!(
            found, expected,
            "`template.html`'s `EDGE` table and `EdgeClass` disagree. The canvas reads \
             `EDGE[edge.class]`, so a class missing there is drawn as an unconditional edge"
        );
    }

    /// The keys of one of the page's object literals.
    fn table_keys(opening: &str) -> Vec<String> {
        let template = crate::graph::TEMPLATE;
        let from = template
            .find(opening)
            .unwrap_or_else(|| panic!("the page carries its `{opening}` table"));
        let rest = &template[from + opening.len()..];
        let to = rest.find("};").expect("the table is closed");
        rest[..to]
            .lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(key, _)| key.trim().to_string())
            .collect()
    }

    /// The serde name of one enumeration member — what the document actually
    /// carries, and what the page indexes its tables by.
    fn member<T: Serialize>(value: &T) -> String {
        serde_json::to_value(value)
            .expect("a unit variant serializes")
            .as_str()
            .expect("as a string")
            .to_string()
    }

    /// The version is the first key, so a consumer can dispatch on it before
    /// reading anything else.
    #[test]
    fn the_version_is_the_documents_first_key() {
        let document = GraphDocument {
            graph_version: GRAPH_VERSION,
            entrypoint: "main.yml".to_string(),
            target: "local".to_string(),
            spec_version: "0.1".to_string(),
            flows: Vec::new(),
        };
        let json = document.to_json().expect("the document serializes");
        assert!(
            json.starts_with("{\n  \"graph_version\": 2,"),
            "the document opens with its version: {json}"
        );
        assert!(json.ends_with("}\n"), "and ends with a newline");
        assert!(document.flow("flow.nothing").is_none());
    }
}
