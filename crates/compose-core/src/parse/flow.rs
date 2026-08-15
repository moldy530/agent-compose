//! Flows, nodes, and edges (grammar 7, 8).

use crate::ast::binding::NodeInput;
use crate::ast::common::Namespace;
use crate::ast::flow::{
    Edge, FlowContext, FlowDef, FlowNode, HumanBlock, ItemError, MapBlock, MapDispatch, MapRoute,
    NODE_KIND_KEYS, Node as FlowNodeAst, NodeKind, StoreNode, StoreOp, StoreOpParams, StoreValue,
};
use crate::ast::policy::PolicyBlock;
use crate::ast::schema::Surface;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::yaml::{Node, Yaml};

use super::binding::{self, NameForm};
use super::definition::description;
use super::lexical;
use super::policy::{self, PolicyLevel};
use super::reader::{
    Cx, Fields, expect_integer, expect_mapping, expect_sequence, expect_string, in_range, list,
};
use super::schema;

/// Read a `flow.*` definition (grammar 7).
pub(crate) fn flow_def(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> FlowDef {
    let description = description(fields, cx);
    let inputs = fields.take("inputs").and_then(|node| {
        schema::field_map(node, &format!("`inputs` of {subject}"), Surface::Input, cx)
    });
    let outputs = fields.require("outputs", cx).and_then(|node| {
        schema::field_map(
            node,
            &format!("`outputs` of {subject}"),
            Surface::Result,
            cx,
        )
    });

    let mut nodes = Vec::new();
    if let Some(node) = fields.require("nodes", cx)
        && let Some(mapping) = expect_mapping(node, &format!("`nodes` of {subject}"), cx)
    {
        if mapping.is_empty() {
            cx.error(
                DiagnosticCode::InvalidValue,
                &node.span,
                format!("`nodes` of {subject} must declare at least one node"),
            );
        }
        for entry in mapping.entries() {
            let Some(id) = lexical::node_id(&entry.key, "node id", cx) else {
                continue;
            };
            nodes.push(node_object(id, &entry.value, cx));
        }
    }

    let mut edges: Vec<Edge> = Vec::new();
    if let Some(node) = fields.require("edges", cx)
        && let Some(items) = expect_sequence(node, &format!("`edges` of {subject}"), cx)
    {
        if items.is_empty() {
            cx.error(
                DiagnosticCode::InvalidValue,
                &node.span,
                format!("`edges` of {subject} must declare at least one edge"),
            );
        }
        // Whether some edge was unreadable. The start-edge rule below is stated
        // over the whole array, so a partial one cannot answer it: reporting
        // "no guaranteed edge leaves `start`" on top of the diagnostic that
        // already explained the broken edge would be a second error for one
        // mistake.
        let mut incomplete = false;
        for item in items {
            let Some(read) = edge(item, cx) else {
                incomplete = true;
                continue;
            };
            incomplete |= read.incomplete;
            let edge = read.edge;
            if let Some(first) = edges.iter().find(|other| same_edge(other, &edge)) {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        edge.span.clone(),
                        "duplicate edge: the same `from`, `to`, and `when` are already declared",
                    )
                    .with_label(first.span.clone(), "first declared here")
                    .with_help("a second identical edge adds no transition"),
                );
                continue;
            }
            edges.push(edge);
        }
        if !incomplete && !items.is_empty() {
            check_start_is_guaranteed(&edges, &node.span, subject, cx);
            check_else_edges_have_a_guarded_sibling(&edges, cx);
        }
    }

    FlowDef {
        description,
        inputs,
        outputs,
        nodes,
        edges,
    }
}

/// At least one edge leaving `start` must be guaranteed to fire — unconditional
/// or carrying `else: true` — so an execution always has a first step instead of
/// dying on "no viable route" before doing any work (grammar 7.6.3 rule 2,
/// Decision D71).
///
/// This is the one no-dead-end rule decidable from a single file: it is stated
/// over the `edges:` array, which is one value. Its two siblings — every node
/// having an outgoing edge, and the `on_error: skip` escape — relate an edge to
/// a node, so they belong to the validator alongside reachability.
///
/// The `else: true` spelling carries its own precondition, a `when:`-guarded
/// sibling, which [`check_else_edges_have_a_guarded_sibling`] decides
/// separately: this check reads exactly what grammar 7.6.3 rule 2 states, and a
/// lone `else: true` leaving `start` is refused by the other rule rather than
/// by a second reading of this one.
fn check_start_is_guaranteed(edges: &[Edge], span: &Span, subject: &str, cx: &mut Cx) {
    let leaves_start = |edge: &&Edge| {
        matches!(
            edge.from.as_ref().map(|from| &from.value),
            Some(crate::ast::common::EdgeSource::Start)
        )
    };
    let mut from_start = edges.iter().filter(leaves_start).peekable();
    if from_start.peek().is_none() {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                span.clone(),
                format!("no edge leaves `start` in `edges` of {subject}"),
            )
            .with_help(
                "a flow's entry is `start`, and one or more edges leave it: write `- { from: start, to: <node> }` (grammar 2.4)",
            ),
        );
        return;
    }
    if from_start.any(|edge| edge.when.is_none()) {
        return;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            span.clone(),
            format!(
                "every edge leaving `start` in {subject} is guarded: at least one must be unconditional or carry `else: true`"
            ),
        )
        .with_help(
            "with every guard false an execution takes no edge at all and fails before its first step; guarded `start` edges are legal, they just cannot be the only ones (grammar 7.6.3)",
        ),
    );
}

/// An edge carrying `else: true` must have a `when:`-guarded sibling leaving the
/// same node (grammar 7.3, Decision D107).
///
/// Grammar 7.3 rule 4 gives an `else:` edge exactly one behaviour — taken iff no
/// guarded sibling was taken — so with no guarded sibling it fires on every
/// pass, which is what an edge carrying neither keyword already is. The keyword
/// then states nothing, while the author who wrote it to mean "only if the other
/// edge did not fire" gets multicast to both targets: two concurrent branches, a
/// reduced-channel requirement they did not expect, and possibly an unbalanced
/// convergence downstream, none of it diagnosed. It is the inert key
/// Decision D61 already refuses in its `else: false` spelling.
///
/// Grammar Appendix B lists this among the rules the *published schema* cannot
/// express — it relates two items of one `edges:` array through a shared `from`
/// value. That is a statement about JSON Schema, not about this pass: the rule
/// needs no resolution, so it is decided here, exactly as `optional:` entries
/// naming declared properties are (grammar 3.4, Decision D89).
fn check_else_edges_have_a_guarded_sibling(edges: &[Edge], cx: &mut Cx) {
    for edge in edges {
        let Some(else_span) = edge.else_edge.as_ref() else {
            continue;
        };
        let Some(from) = edge.from.as_ref() else {
            continue;
        };
        let has_guarded_sibling = edges.iter().any(|other| {
            other.when.is_some()
                && other.from.as_ref().map(|other| &other.value) == Some(&from.value)
        });
        if has_guarded_sibling {
            continue;
        }
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                else_span.clone(),
                format!(
                    "the `else: true` edge leaving `{}` has no `when`-guarded sibling",
                    describe_source(&from.value)
                ),
            )
            .with_label(from.span.clone(), "no other edge leaving this node is guarded")
            .with_help(
                "an `else:` edge is taken when no guarded sibling was taken, so with none it fires on every pass — which is what an edge carrying neither keyword already is: drop the `else:`, or guard the sibling it is meant to be else to (grammar 7.3, Decision D107)",
            ),
        );
    }
}

/// How an edge's `from` reads in a diagnostic.
fn describe_source(source: &crate::ast::common::EdgeSource) -> String {
    match source {
        crate::ast::common::EdgeSource::Start => "start".to_owned(),
        crate::ast::common::EdgeSource::Node(id) => id.to_string(),
    }
}

/// Whether two edges declare the same transition (grammar 7.2).
///
/// An edge missing an endpoint declares no transition at all, so it is never
/// the same one as anything: two entries that each merely omit `to:` are two
/// broken edges, not a duplicate pair, and saying "the same `from`, `to`, and
/// `when` are already declared" of a `to:` neither one wrote would assert a
/// violation the source does not contain. Each has its own missing-key
/// diagnostic already.
fn same_edge(left: &Edge, right: &Edge) -> bool {
    if [left, right]
        .iter()
        .any(|edge| edge.from.is_none() || edge.to.is_none())
    {
        return false;
    }
    let endpoints = left.from.as_ref().map(|s| &s.value) == right.from.as_ref().map(|s| &s.value)
        && left.to.as_ref().map(|s| &s.value) == right.to.as_ref().map(|s| &s.value);
    let guards = left.when.as_ref().map(|g| g.value.as_str())
        == right.when.as_ref().map(|g| g.value.as_str());
    endpoints && guards
}

/// One `edges:` entry as read, with whether anything the author wrote in it
/// failed to read.
///
/// The rules stated over the whole `edges:` array cannot be answered from a
/// partial one, so they are skipped when any entry is incomplete: reporting
/// them on top of the diagnostic that already explained the broken edge would
/// be a second error for one mistake.
struct ReadEdge {
    edge: Edge,
    incomplete: bool,
}

fn edge(node: &Node, cx: &mut Cx) -> Option<ReadEdge> {
    let mapping = expect_mapping(node, "each entry of `edges`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "an edge");

    let from = fields
        .require("from", cx)
        .and_then(|node| lexical::edge_source(node, cx));
    let to = fields
        .require("to", cx)
        .and_then(|node| lexical::edge_target(node, cx));
    // Whether the author wrote `when:` at all, which is a different question
    // from whether it read as an expression. Both `max_iterations` below and
    // the `else:` sibling rule are stated over an edge that *declares* a guard
    // (grammar 7.2, 7.3), so a `when:` whose value is the wrong YAML kind has
    // to count as declared: treating it as absent turns one mistake into two
    // diagnostics, the second of them about a different edge.
    let when_entry = fields.take_entry("when");
    let when = when_entry.and_then(|entry| lexical::cel(&entry.value, "`when`", cx));
    let guard_unreadable = when_entry.is_some() && when.is_none();

    let else_edge = fields.take("else").and_then(|node| match &node.value {
        Yaml::Bool(true) => Some(node.span.clone()),
        _ => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    node.span.clone(),
                    "`else` takes the literal `true`",
                )
                .with_help(
                    "an edge with neither `when` nor `else` is already unconditional, so any other value would change nothing",
                ),
            );
            None
        }
    });

    if let (Some(when), Some(else_span)) = (when.as_ref(), else_edge.as_ref()) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingKeys,
                else_span.clone(),
                "an edge declares `when` or `else`, never both",
            )
            .with_label(when.span.clone(), "the guard is declared here")
            .with_help("`else` marks the edge taken when no guarded sibling was"),
        );
    }

    // An exhausted edge is not taken whatever its guard says (grammar 7.3
    // rule 5), so a budget on an unconditional or `else:` edge would quietly
    // withdraw the guarantee four other rules rest on — exhaustiveness clause 1,
    // the `start` edge, the skip escape, and the cycle escape (Decision D90).
    let max_iterations = fields.take_entry("max_iterations").and_then(|entry| {
        if when_entry.is_none() {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    entry.key.span.clone(),
                    "`max_iterations` is legal only on an edge that also declares `when`",
                )
                .with_help(
                    "an exhausted edge is not taken whatever its guard says, so a budget on an unconditional or `else:` edge would withdraw the guarantee that makes it an escape: bound the guarded back-edge instead (grammar 7.2, Decision D90)",
                ),
            );
            return None;
        }
        super::reader::expect_integer(&entry.value, "`max_iterations`", cx)
    });
    let max_iterations =
        max_iterations.filter(|value| in_range(value, "`max_iterations`", 1..=1000, cx));
    fields.finish(cx);

    let incomplete = from.is_none() || to.is_none() || guard_unreadable;
    Some(ReadEdge {
        edge: Edge {
            from,
            to,
            when,
            else_edge,
            max_iterations,
            span: node.span.clone(),
        },
        incomplete,
    })
}

/// Whether a common key is legal on a node kind, and when it is not, the help
/// text that says why — every kind that rejects one rejects it for its own
/// reason, so the note has to come from the kind rather than from the key.
enum KeyRule {
    Allowed,
    Rejected(&'static str),
}

/// Which common keys a node kind accepts (grammar 7.1, 8.6 rule 9, 8.7, 8.8).
struct CommonKeys {
    input: KeyRule,
    writes: KeyRule,
    retry_timeout: KeyRule,
}

impl CommonKeys {
    /// Every common key, as the six ordinary node kinds take them.
    const fn all() -> Self {
        Self {
            input: KeyRule::Allowed,
            writes: KeyRule::Allowed,
            retry_timeout: KeyRule::Allowed,
        }
    }
}

fn node_object(id: Spanned<crate::ast::common::Ident>, node: &Node, cx: &mut Cx) -> FlowNodeAst {
    let subject = format!("node `{}`", id.value);
    let Some(mapping) = expect_mapping(node, &subject, cx) else {
        return FlowNodeAst {
            id,
            kind: NodeKind::Invalid,
            input: None,
            writes: None,
            policy: PolicyBlock::default(),
            description: None,
            span: node.span.clone(),
        };
    };
    let mut fields = Fields::new(mapping, node.span.clone(), &subject);
    fields.note_known(NODE_KIND_KEYS);

    let declared: Vec<&str> = NODE_KIND_KEYS
        .iter()
        .copied()
        .filter(|key| fields.contains(key))
        .collect();

    if declared.len() > 1 {
        for key in &declared[1..] {
            let span = fields
                .take_entry(key)
                .map_or_else(|| fields.span.clone(), |entry| entry.key.span.clone());
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    span,
                    format!(
                        "{subject} declares both `{}` and `{key}`; a node carries exactly one kind key",
                        declared[0]
                    ),
                )
                .with_help(format!("the node kinds are {}", list(NODE_KIND_KEYS))),
            );
        }
    }

    let (kind, common) = match declared.first().copied() {
        None => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingKey,
                    fields.span.clone(),
                    format!(
                        "{subject} declares no kind key: a node carries exactly one of {}",
                        list(NODE_KIND_KEYS)
                    ),
                )
                .with_help("the kind key selects what the node does and what config it takes"),
            );
            (NodeKind::Invalid, CommonKeys::all())
        }
        Some(key) => node_kind(&mut fields, key, &subject, cx),
    };

    let input = match common.input {
        KeyRule::Allowed => fields
            .take("input")
            .and_then(|node| node_input(node, &kind, &subject, cx)),
        KeyRule::Rejected(why) => {
            reject_key(&mut fields, "input", &subject, why, cx);
            None
        }
    };
    let writes = match common.writes {
        KeyRule::Allowed => fields
            .take("writes")
            .and_then(|node| binding::writes(node, &format!("`writes` of {subject}"), cx)),
        KeyRule::Rejected(why) => {
            reject_key(&mut fields, "writes", &subject, why, cx);
            None
        }
    };

    let policy = match common.retry_timeout {
        KeyRule::Allowed => policy::policy_fields(&mut fields, &subject, PolicyLevel::Node, cx),
        KeyRule::Rejected(why) => {
            for key in ["retry", "timeout"] {
                reject_key(&mut fields, key, &subject, why, cx);
            }
            PolicyBlock {
                retry: None,
                timeout: None,
                on_error: fields
                    .take("on_error")
                    .and_then(|node| policy::on_error(node, &subject, PolicyLevel::Node, cx)),
            }
        }
    };

    // An inline `exec:` node's bindings become environment variables, so an
    // in-block `env:` key of the same name would silently win (Decision D66).
    if let (NodeKind::Exec(block), Some(NodeInput::Fields(bindings))) = (&kind, input.as_ref()) {
        let names: Vec<Spanned<String>> = bindings
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        binding::reject_env_collisions(block, names.iter(), cx);
    }

    // An inline `http:` node's bindings become the body or the query string, so
    // declaring the in-block key that would carry them leaves one inert (D66).
    if let (NodeKind::Http(block), Some(_)) = (&kind, input.as_ref())
        && let Some(method) = block.method.as_ref()
    {
        let (competing, slot) = if method.value.carries_body() {
            (block.body.as_ref(), "body")
        } else {
            (block.query.as_ref(), "query string")
        };
        if let Some(competing) = competing {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    competing.span.clone(),
                    format!(
                        "{subject} declares both `input:` and an in-block `{}`, which both claim the request {slot}",
                        if method.value.carries_body() { "body" } else { "query" }
                    ),
                )
                .with_help("write the request out in full, or let the bindings build it"),
            );
        }
    }

    let description = description(&mut fields, cx);
    fields.finish(cx);

    FlowNodeAst {
        id,
        kind,
        input,
        writes,
        policy,
        description,
        span: node.span.clone(),
    }
}

fn node_input(node: &Node, kind: &NodeKind, subject: &str, cx: &mut Cx) -> Option<NodeInput> {
    let context = format!("`input` of {subject}");
    // Which kinds bind field by field, and the declaration each one's bindings
    // have to match. Everything else may take the scalar form of grammar 8.0.
    // The scalar form is legal only where a single unnamed value has a defined
    // destination: a string-in agent, and an inline `exec:` node's stdin
    // (grammar 8.0, Decision D88). Every other kind names its fields, so a bare
    // scalar there names nothing.
    let named = match kind {
        // A subgraph receives parent state only through explicit named
        // bindings (grammar 8.5, PRD 5.7).
        NodeKind::Flow(_) => {
            Some("a subflow declares its inputs by name, and each is bound by name (grammar 8.5)")
        }
        // A human node's own `input:` field map sits in the same node object,
        // so every binding has a declared name to match (grammar 8.7).
        NodeKind::Human(_) => {
            Some("a human node's `input:` field map declares the names to bind (grammar 8.7)")
        }
        // An inline `http:` node builds an ad-hoc request object out of named
        // fields — the JSON body, or the query string (grammar 8.3).
        NodeKind::Http(_) => Some(
            "an inline `http:` node builds its request out of named fields, which become the JSON body or the query string (grammar 8.3)",
        ),
        // A `function:` node's arguments are checked field by field against the
        // tool's declared `input` (grammar 8.4).
        NodeKind::Function(_) => Some(
            "a `function:` node's arguments are checked field by field against the tool's declared `input` (grammar 8.4)",
        ),
        _ => None,
    };
    match named {
        Some(why) => binding::field_input(node, &context, why, cx).map(NodeInput::Fields),
        None => binding::node_input(node, &context, cx),
    }
}

fn reject_key(fields: &mut Fields<'_>, key: &'static str, subject: &str, why: &str, cx: &mut Cx) {
    let Some(entry) = fields.take_entry(key) else {
        return;
    };
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            entry.key.span.clone(),
            format!("`{key}` is not legal on {subject}"),
        )
        .with_help(why.to_owned()),
    );
}

const FLOW_CONTEXTS: &[(&str, FlowContext)] = &[
    ("isolated", FlowContext::Isolated),
    ("inherit", FlowContext::Inherit),
];

const STORE_OPS: &[(&str, StoreOp)] = &[
    ("get", StoreOp::Get),
    ("set", StoreOp::Set),
    ("delete", StoreOp::Delete),
    ("list", StoreOp::List),
    ("search", StoreOp::Search),
    ("upsert", StoreOp::Upsert),
    ("put", StoreOp::Put),
];

fn node_kind(
    fields: &mut Fields<'_>,
    key: &str,
    subject: &str,
    cx: &mut Cx,
) -> (NodeKind, CommonKeys) {
    let all = CommonKeys::all();
    match key {
        "agent" => {
            let kind = fields
                .take("agent")
                .and_then(|node| lexical::reference(node, "`agent`", &[Namespace::Agent], cx))
                .map_or(NodeKind::Invalid, NodeKind::Agent);
            (kind, all)
        }
        "function" => {
            let kind = fields
                .take("function")
                .and_then(|node| lexical::reference(node, "`function`", &[Namespace::Tool], cx))
                .map_or(NodeKind::Invalid, NodeKind::Function);
            (kind, all)
        }
        "exec" => {
            let kind = fields
                .take("exec")
                .and_then(|node| {
                    binding::exec_block(node, &format!("the `exec` block of {subject}"), true, cx)
                })
                .map_or(NodeKind::Invalid, NodeKind::Exec);
            (kind, all)
        }
        "http" => {
            let kind = fields
                .take("http")
                .and_then(|node| {
                    binding::http_block(node, &format!("the `http` block of {subject}"), true, cx)
                })
                .map_or(NodeKind::Invalid, NodeKind::Http);
            (kind, all)
        }
        "flow" => {
            let flow = fields
                .take("flow")
                .and_then(|node| lexical::reference(node, "`flow`", &[Namespace::Flow], cx));
            let context = fields
                .take("context")
                .and_then(|node| lexical::keyword(node, "`context`", FLOW_CONTEXTS, cx));
            let policy = fields.take("policy").and_then(|node| {
                policy::policy_block(node, &format!("the `policy` block of {subject}"), cx)
            });
            let kind = flow.map_or(NodeKind::Invalid, |flow| {
                NodeKind::Flow(FlowNode {
                    flow,
                    context,
                    policy,
                })
            });
            (kind, all)
        }
        "human" => {
            let kind = fields
                .take("human")
                .and_then(|node| human_block(node, subject, cx))
                .map_or(NodeKind::Invalid, NodeKind::Human);
            (
                kind,
                CommonKeys {
                    retry_timeout: KeyRule::Rejected(
                        "a human wait is not an activity timeout, and re-prompting a human is not a retry",
                    ),
                    ..CommonKeys::all()
                },
            )
        }
        "map" => {
            let kind = fields
                .take("map")
                .and_then(|node| map_block(node, subject, cx))
                .map_or(NodeKind::Invalid, NodeKind::Map);
            (
                kind,
                CommonKeys {
                    input: KeyRule::Rejected(
                        "a map node has no input of its own: the per-item binding lives in the `map:` block",
                    ),
                    writes: KeyRule::Rejected(
                        "a map node has no output of its own: the write remap lives in the `map:` block or on a route",
                    ),
                    retry_timeout: KeyRule::Allowed,
                },
            )
        }
        _ => {
            let store = fields
                .take("store")
                .and_then(|node| lexical::reference(node, "`store`", &[Namespace::Store], cx));
            let op = fields
                .take("op")
                .and_then(|node| lexical::keyword(node, "store `op`", STORE_OPS, cx));
            if op.is_none() && !fields.contains("op") {
                cx.error(
                    DiagnosticCode::MissingKey,
                    &fields.span.clone(),
                    format!("missing required key `op` in {subject}"),
                );
            }
            let params = store_params(fields, op.as_ref(), subject, cx);
            let kind = store.map_or(NodeKind::Invalid, |store| {
                NodeKind::Store(StoreNode { store, op, params })
            });
            (
                kind,
                CommonKeys {
                    input: KeyRule::Rejected(
                        "a store op takes its parameters as its own keys — `key:`, `value:`, `query:` — rather than through `input:` (grammar 8.8, 11.4)",
                    ),
                    ..CommonKeys::all()
                },
            )
        }
    }
}

/// Every store-op parameter name (grammar 11.4).
const STORE_PARAMS: &[&str] = &[
    "key",
    "value",
    "query",
    "prefix",
    "top_k",
    "limit",
    "filter",
    "metadata",
    "content_type",
];

fn store_params(
    fields: &mut Fields<'_>,
    op: Option<&Spanned<StoreOp>>,
    subject: &str,
    cx: &mut Cx,
) -> StoreOpParams {
    fields.note_known(STORE_PARAMS);
    let mut params = StoreOpParams::default();
    let Some(op) = op else {
        // Without a legible `op:` there is no parameter row to check the
        // parameters against, so take them all rather than reporting each as an
        // unknown key on top of the diagnostic the op already produced.
        for name in STORE_PARAMS {
            let _ = fields.take(name);
        }
        return params;
    };

    let legal: Vec<&str> = op
        .value
        .required_parameters()
        .iter()
        .chain(op.value.optional_parameters())
        .copied()
        .collect();

    for name in STORE_PARAMS {
        if legal.contains(name) {
            continue;
        }
        let Some(entry) = fields.take_entry(name) else {
            continue;
        };
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                entry.key.span.clone(),
                format!(
                    "`{name}` is not a parameter of the `{}` op",
                    op.value.as_str()
                ),
            )
            .with_label(op.span.clone(), "the op is declared here")
            .with_help(format!("`{}` takes {}", op.value.as_str(), list(&legal))),
        );
    }

    for name in op.value.required_parameters() {
        if !fields.contains(name) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingKey,
                    fields.span.clone(),
                    format!(
                        "missing required key `{name}` in {subject}: the `{}` op takes {}",
                        op.value.as_str(),
                        list(op.value.required_parameters())
                    ),
                )
                .with_label(op.span.clone(), "the op is declared here"),
            );
        }
    }

    params.key =
        store_param(fields, &legal, "key").and_then(|node| lexical::cel(node, "`key`", cx));
    params.query =
        store_param(fields, &legal, "query").and_then(|node| lexical::cel(node, "`query`", cx));
    params.prefix =
        store_param(fields, &legal, "prefix").and_then(|node| lexical::cel(node, "`prefix`", cx));
    params.top_k = store_param(fields, &legal, "top_k")
        .and_then(|node| expect_integer(node, "`top_k`", cx))
        .filter(|value| in_range(value, "`top_k`", 1..=100, cx));
    params.limit = store_param(fields, &legal, "limit")
        .and_then(|node| expect_integer(node, "`limit`", cx))
        .filter(|value| in_range(value, "`limit`", 1..=1000, cx));
    params.filter = store_param(fields, &legal, "filter")
        .and_then(|node| binding::bindings(node, "`filter`", NameForm::Identifier, cx));
    params.metadata = store_param(fields, &legal, "metadata")
        .and_then(|node| binding::bindings(node, "`metadata`", NameForm::Identifier, cx));
    params.content_type = store_param(fields, &legal, "content_type")
        .and_then(|node| lexical::text(node, "`content_type`", cx));

    params.value = store_param(fields, &legal, "value").and_then(|node| match op.value {
        // `kv set` writes a value_schema-shaped object; `vector upsert` and
        // `blob put` write one string (grammar 11.4).
        StoreOp::Set => {
            binding::bindings(node, "`value`", NameForm::Identifier, cx).map(StoreValue::Fields)
        }
        _ => lexical::cel(node, "`value`", cx).map(StoreValue::Expression),
    });

    params
}

/// The value under one store-op parameter, unless the op's row does not admit
/// the name.
///
/// The loop above has already refused every parameter outside the row, by name
/// and with the row quoted back. Reading one anyway would parse it a second
/// time and report its YAML type too, so `{ op: get, top_k: "state.k" }` — one
/// mistake — would draw both "`top_k` is not a parameter of the `get` op" and
/// "expected an integer for `top_k`". Every sibling rejection helper
/// (`reject_key`, `per_target_key`, `provider_key`) returns `None` for the same
/// reason.
fn store_param<'a>(
    fields: &mut Fields<'a>,
    legal: &[&str],
    name: &'static str,
) -> Option<&'a Node> {
    if !legal.contains(&name) {
        return None;
    }
    fields.take(name)
}

fn human_block(node: &Node, subject: &str, cx: &mut Cx) -> Option<HumanBlock> {
    let context = format!("the `human` block of {subject}");
    let mapping = expect_mapping(node, &context, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), &context);

    let input = fields.require("input", cx).and_then(|node| {
        schema::field_map(node, &format!("`input` of {context}"), Surface::Input, cx)
    });
    let output = fields.require("output", cx).and_then(|node| {
        schema::field_map(node, &format!("`output` of {context}"), Surface::Result, cx)
    });
    let timeout = fields
        .take_entry("timeout")
        .and_then(|entry| lexical::duration(&entry.value, "`timeout`", cx));
    let on_timeout = fields
        .take_entry("on_timeout")
        .and_then(|entry| lexical::control_target(&entry.value, "`on_timeout`", cx));

    // Jointly optional, jointly required: either half alone is inert
    // (grammar 8.7, Decision D52).
    match (fields.contains("timeout"), fields.contains("on_timeout")) {
        (true, false) => cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                timeout
                    .as_ref()
                    .map_or_else(|| node.span.clone(), |value| value.span.clone()),
                format!("missing required key `on_timeout` in {context}: `timeout` declares a wait budget with no route"),
            )
            .with_help("`timeout` and `on_timeout` are declared together or not at all"),
        ),
        (false, true) => cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                on_timeout
                    .as_ref()
                    .map_or_else(|| node.span.clone(), |value| value.span.clone()),
                format!("missing required key `timeout` in {context}: `on_timeout` declares a route nothing can reach"),
            )
            .with_help("`timeout` and `on_timeout` are declared together or not at all"),
        ),
        _ => {}
    }

    fields.finish(cx);
    Some(HumanBlock {
        input,
        output,
        timeout,
        on_timeout,
        span: node.span.clone(),
    })
}

fn map_block(node: &Node, subject: &str, cx: &mut Cx) -> Option<MapBlock> {
    let context = format!("the `map` block of {subject}");
    let mapping = expect_mapping(node, &context, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), &context);

    let over = fields
        .require("over", cx)
        .and_then(|node| lexical::path_expression(node, "`over`", cx));
    let item_binding = fields
        .string("as", cx)
        .and_then(|text| lexical::item_binding_name(&text, "the `as` binding", cx));
    let max_concurrency = fields
        .require("max_concurrency", cx)
        .and_then(|node| super::reader::expect_integer(node, "`max_concurrency`", cx))
        .filter(|value| in_range(value, "`max_concurrency`", 1..=256, cx));
    let on_item_error = fields
        .take("on_item_error")
        .and_then(|node| item_error(node, &context, cx));

    // A dispatch is the other module boundary for conversation history, and its
    // isolation is unconditional: every instance runs on a fresh history that is
    // discarded when it completes, and the sharing opt-in is a `flow:` node's
    // key. Reporting `context:` here as a plain unknown key would leave the
    // author who wrote it to guess why (grammar 8.6 rule 13, 10.4, Decision
    // D105).
    if let Some(entry) = fields.take_entry("context") {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownKey,
                entry.key.span.clone(),
                format!("`context` is not legal in {context}"),
            )
            .with_help(
                "a `map` dispatch always isolates conversation history: inheriting into concurrent instances would either fork the channel or serialize the fan-out, so the opt-in stays a `flow:` node's key (grammar 8.6 rule 13, Decision D105)",
            ),
        );
    }

    // `input:`, `writes:`, and `detach:` each describe a dispatch *target*, so
    // a map that routes declares them on its routes: a map-level `input:` would
    // have to type-check against every variant at once, a map-level `writes:`
    // would name output fields only some route targets declare, and a blanket
    // `detach:` would silently detach sinks written to be joined (grammar 8.6
    // rule 7, Decisions D31, D85).
    let routed = fields.contains("route_by") || fields.contains("routes");
    let input = per_target_key(&mut fields, "input", routed, cx)
        .and_then(|node| binding::node_input(node, &format!("`input` of {context}"), cx));
    let writes = per_target_key(&mut fields, "writes", routed, cx)
        .and_then(|node| binding::writes(node, &format!("`writes` of {context}"), cx));
    let detach =
        per_target_key(&mut fields, "detach", routed, cx).and_then(|node| match &node.value {
            Yaml::Bool(value) => Some(Spanned::new(*value, node.span.clone())),
            _ => {
                cx.wrong_type(node, "`detach`", "a boolean");
                None
            }
        });
    reject_detached_writes(
        detach.as_ref(),
        writes.as_ref().map(|w| &w.span),
        &context,
        cx,
    );

    let dispatch = dispatch(&mut fields, &context, max_concurrency.as_ref(), cx);
    fields.finish(cx);

    Some(MapBlock {
        over,
        item_binding,
        dispatch,
        max_concurrency,
        on_item_error,
        input,
        writes,
        detach,
        span: node.span.clone(),
    })
}

/// Why a routed map declares each of the three per-dispatch-target keys on its
/// routes instead (grammar 8.6 rule 7, Decisions D31, D85).
fn per_target_reason(key: &str) -> &'static str {
    match key {
        "input" => {
            "each route's per-item bindings are narrowed to its own variant's payload, so a map-level `input` would have to type-check against every variant at once: declare it on the routes (grammar 8.6 rule 7)"
        }
        "writes" => {
            "the routes of a heterogeneous map are independently typed targets with their own output schemas, so a map-level `writes` would name fields only some of them declare: declare it on the routes (grammar 8.6 rule 7)"
        }
        _ => {
            "the routes of a heterogeneous map are independently typed targets, and a blanket `detach` would silently detach sinks written to be joined: declare it on the routes that want it (grammar 8.6 rule 7)"
        }
    }
}

/// Take a map-block key that describes a dispatch target, refusing it on the
/// routed form (grammar 8.6 rule 7).
fn per_target_key<'a>(
    fields: &mut Fields<'a>,
    key: &'static str,
    routed: bool,
    cx: &mut Cx,
) -> Option<&'a Node> {
    let entry = fields.take_entry(key)?;
    if !routed {
        return Some(&entry.value);
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::ConflictingKeys,
            entry.key.span.clone(),
            format!("`{key}` is not legal on a discriminator-routed map"),
        )
        .with_help(per_target_reason(key)),
    );
    None
}

/// Read `on_item_error:` — `fail`, `skip`, or `{ retry: <retry block> }`
/// (grammar 8.6 rule 10, Decision D73).
///
/// The same enum-or-single-key-object shape `on_error:` uses one section
/// earlier, so this adds no vocabulary and one reading rule.
fn item_error(node: &Node, context: &str, cx: &mut Cx) -> Option<Spanned<ItemError>> {
    let expectation = format!(
        "`on_item_error` takes {}, or `{{ retry: {{ max: <n>, backoff: <duration> }} }}`",
        list(["fail", "skip"])
    );
    match &node.value {
        Yaml::String(text) => match text.as_str() {
            "fail" => Some(Spanned::new(ItemError::Fail, node.span.clone())),
            "skip" => Some(Spanned::new(ItemError::Skip, node.span.clone())),
            // A bare `retry` names a behaviour with no count and no backoff,
            // and there is nowhere for it to inherit one from: `defaults:`
            // applies to nodes, and a dispatched instance is not a node of this
            // flow (Decision D73).
            "retry" => {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        node.span.clone(),
                        format!(
                            "`on_item_error: retry` in {context} must carry its policy inline"
                        ),
                    )
                    .with_help(
                        "write `on_item_error: { retry: { max: <n>, backoff: <duration> } }`: a retry with no bound is the unbounded loop fan-out bounding exists to prevent, and no chain supplies an item policy (grammar 8.6 rule 10, Decision D73)",
                    ),
                );
                None
            }
            other => {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        node.span.clone(),
                        format!("`{other}` is not an `on_item_error` strategy in {context}"),
                    )
                    .with_optional_help(Some(expectation)),
                );
                None
            }
        },
        Yaml::Mapping(mapping) => {
            let block = format!("the `on_item_error` block of {context}");
            let mut fields = Fields::new(mapping, node.span.clone(), &block);
            let retry = fields
                .require("retry", cx)
                .and_then(|node| policy::retry(node, &block, cx));
            fields.finish(cx);
            retry.map(|retry| Spanned::new(ItemError::Retry(retry), node.span.clone()))
        }
        _ => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::WrongType,
                    node.span.clone(),
                    format!(
                        "expected an `on_item_error` strategy for {context}, found {}",
                        node.description()
                    ),
                )
                .with_help(expectation),
            );
            None
        }
    }
}

fn dispatch(
    fields: &mut Fields<'_>,
    context: &str,
    max_concurrency: Option<&Spanned<i64>>,
    cx: &mut Cx,
) -> MapDispatch {
    let homogeneous = fields.contains("node");
    let routed = fields.contains("route_by") || fields.contains("routes");
    fields.note_known(&["node", "route_by", "routes", "default"]);

    if homogeneous && routed {
        // Consume both routing keys, not just the one the span comes from:
        // the conflict is the whole story, and reporting the other as an
        // unknown key on top of it would be noise.
        let route_by = fields.take_entry("route_by");
        let routes = fields.take_entry("routes");
        let span = route_by
            .or(routes)
            .map_or_else(|| fields.span.clone(), |entry| entry.key.span.clone());
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingKeys,
                span,
                format!("{context} declares both a `node:` target and discriminator routing"),
            )
            .with_help(
                "a map dispatches one way or the other: `node:`, or `route_by:` with `routes:`",
            ),
        );
    }

    if homogeneous {
        if let Some(entry) = fields.take_entry("default") {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    entry.key.span.clone(),
                    "`default` is a catch-all route and is legal only with `route_by`".to_owned(),
                )
                .with_help("a homogeneous map has one target, so there is nothing to fall back to"),
            );
        }
        let target = fields.take("node").and_then(|node| {
            lexical::reference(
                node,
                "`node`",
                &[Namespace::Agent, Namespace::Tool, Namespace::Flow],
                cx,
            )
        });
        return target.map_or(MapDispatch::Invalid, |node| MapDispatch::Homogeneous {
            node,
        });
    }

    if !routed {
        // `default:` is a real `map:` key, so it must be consumed here even
        // though nothing reads it: left over, `finish` would report the map's
        // own catch-all as an unknown key. Missing dispatch is the whole
        // mistake, so it stays one diagnostic and `default:` becomes the label
        // that says what it was waiting for.
        let waiting = fields.take_entry("default").map(|entry| {
            (
                entry.key.span.clone(),
                "`default` is a catch-all route, and no `route_by` declares what it falls back from",
            )
        });
        let mut diagnostic = Diagnostic::error(
            DiagnosticCode::MissingKey,
            fields.span.clone(),
            format!("{context} declares no dispatch target"),
        )
        .with_help("a map declares `node:` for a homogeneous fan-out, or `route_by:` with `routes:` for a discriminator-routed one");
        if let Some((span, message)) = waiting {
            diagnostic = diagnostic.with_label(span, message);
        }
        cx.push(diagnostic);
        return MapDispatch::Invalid;
    }

    // The routed form takes both keys: `routes:` alone has no discriminator to
    // switch on, and `route_by:` alone has nothing to switch to (grammar 8.6
    // rule 2). Whichever one is missing is reported as missing, rather than the
    // pair being silently accepted and the routes dropped.
    let route_by = fields
        .require("route_by", cx)
        .and_then(|node| expect_string(node, "`route_by`", cx))
        .and_then(|text| lexical::identifier(&text, "`route_by`", cx));

    let mut routes = Vec::new();
    if let Some(node) = fields.require("routes", cx)
        && let Some(mapping) = expect_mapping(node, "`routes`", cx)
    {
        if mapping.is_empty() {
            cx.error(
                DiagnosticCode::InvalidValue,
                &node.span,
                "`routes` must declare at least one route",
            );
        }
        for entry in mapping.entries() {
            let Some(tag) = lexical::key_identifier(&entry.key, "route tag", cx) else {
                continue;
            };
            let label = format!("route `{}`", tag.value);
            if let Some(route) = map_route(&entry.value, &label, Some(tag), max_concurrency, cx) {
                routes.push(route);
            }
        }
    }

    let default = fields
        .take("default")
        .and_then(|node| map_route(node, "the `default` route", None, max_concurrency, cx))
        .map(Box::new);

    match route_by {
        Some(route_by) => MapDispatch::Routed {
            route_by,
            routes,
            default,
        },
        None => MapDispatch::Invalid,
    }
}

fn map_route(
    node: &Node,
    subject: &str,
    tag: Option<Spanned<crate::ast::common::Ident>>,
    map_concurrency: Option<&Spanned<i64>>,
    cx: &mut Cx,
) -> Option<MapRoute> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    let target = fields.require("node", cx).and_then(|node| {
        lexical::reference(
            node,
            "`node`",
            &[Namespace::Agent, Namespace::Tool, Namespace::Flow],
            cx,
        )
    });
    let max_concurrency = fields
        .integer("max_concurrency", cx)
        .filter(|value| in_range(value, "`max_concurrency`", 1..=256, cx))
        .inspect(|value| {
            if let Some(map) = map_concurrency
                && value.value > map.value
            {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::ValueOutOfRange,
                        value.span.clone(),
                        format!(
                            "`max_concurrency` of {subject} is {}, above the map's {}",
                            value.value, map.value
                        ),
                    )
                    .with_label(map.span.clone(), "the map's bound is declared here")
                    .with_help("a route may only tighten the map's bound (grammar 8.6 rule 1)"),
                );
            }
        });
    let input = fields
        .take("input")
        .and_then(|node| binding::node_input(node, &format!("`input` of {subject}"), cx));
    let writes = fields
        .take("writes")
        .and_then(|node| binding::writes(node, &format!("`writes` of {subject}"), cx));
    let detach = fields.boolean("detach", cx);
    reject_detached_writes(
        detach.as_ref(),
        writes.as_ref().map(|w| &w.span),
        subject,
        cx,
    );
    fields.finish(cx);

    Some(MapRoute {
        tag,
        node: target,
        max_concurrency,
        input,
        writes,
        detach,
        span: node.span.clone(),
    })
}

/// A detached dispatch is fire-and-forget, so it has nothing to write back
/// (grammar 8.6 rule 7).
fn reject_detached_writes(
    detach: Option<&Spanned<bool>>,
    writes: Option<&Span>,
    subject: &str,
    cx: &mut Cx,
) {
    let (Some(detach), Some(writes)) = (detach, writes) else {
        return;
    };
    if !detach.value {
        return;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::ConflictingKeys,
            writes.clone(),
            format!("{subject} is detached, so it may not declare `writes`"),
        )
        .with_label(detach.span.clone(), "detached here")
        .with_help(
            "a fire-and-forget dispatch is not joined, so nothing waits to record its result",
        ),
    );
}

#[cfg(test)]
mod tests {
    use crate::parse_str;

    /// Every diagnostic one source produces, as `code: message`. The whole list
    /// is compared, so a second diagnostic for one mistake fails the test.
    fn diagnostics(source: &str) -> Vec<String> {
        parse_str(source, "test.yml".to_string())
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect()
    }

    /// A flow whose only variable is its `edges:` array.
    fn flow(edges: &str) -> String {
        format!(
            "flow.demo:
  outputs:
    r: {{ type: string }}
  nodes:
    a: {{ agent: agent.x }}
    b: {{ agent: agent.x }}
    c: {{ agent: agent.x }}
  edges:
{edges}"
        )
    }

    /// Two edges that each merely omit `to:` are two broken edges, not a
    /// duplicate pair: neither declares a transition, so grammar 7.2's
    /// duplicate rule has nothing to compare. Each already carries its own
    /// missing-key diagnostic.
    #[test]
    fn two_edges_missing_the_same_endpoint_are_not_duplicates_of_each_other() {
        assert_eq!(
            diagnostics(&flow("    - { from: a }\n    - { from: a }\n")),
            [
                "missing-key: missing required key `to` in an edge",
                "missing-key: missing required key `to` in an edge",
            ]
        );
    }

    /// A `when:` whose value is the wrong YAML kind is a guard that was
    /// declared and did not read, so the `else:` edge it is meant to be else to
    /// keeps its guarded sibling (grammar 7.3, Decision D107). Reporting D107
    /// here would be a second error, against a different edge, for one mistake.
    #[test]
    fn an_unreadable_guard_still_counts_as_a_declared_one() {
        assert_eq!(
            diagnostics(&flow(
                "    - { from: start, to: a }\n    - { from: a, to: b, when: 5 }\n    - { from: a, to: c, else: true }\n"
            )),
            ["wrong-type: expected a CEL expression for `when`, found an integer"]
        );
    }

    /// The same reading of "declares" on `max_iterations`, which grammar 7.2
    /// and Decision D90 allow on an edge that declares `when`.
    #[test]
    fn an_unreadable_guard_still_carries_a_bound() {
        assert_eq!(
            diagnostics(&flow(
                "    - { from: start, to: a }\n    - { from: a, to: b, when: 5, max_iterations: 3 }\n"
            )),
            ["wrong-type: expected a CEL expression for `when`, found an integer"]
        );
    }

    /// `default:` is a `map:` key, so a map declaring no dispatch form at all
    /// reports the missing target once and points at the `default:` that was
    /// waiting for it — rather than calling the map's own catch-all an unknown
    /// key and suggesting the author meant `default`.
    #[test]
    fn a_default_route_without_a_dispatch_form_is_not_an_unknown_key() {
        let source = "flow.demo:
  outputs:
    r: { type: string }
  nodes:
    a: { agent: agent.x }
    m:
      map:
        over: \"a.output.xs\"
        max_concurrency: 2
        default: { node: tool.t }
  edges:
    - { from: start, to: a }
    - { from: a, to: m }
    - { from: m, to: end }
";
        let parsed = parse_str(source, "test.yml".to_string());
        let reported: Vec<String> = parsed
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect();
        assert_eq!(
            reported,
            ["missing-key: the `map` block of node `m` declares no dispatch target"]
        );
        let labels: Vec<&str> = parsed.diagnostics[0]
            .labels
            .iter()
            .map(|label| label.message.as_str())
            .collect();
        assert_eq!(
            labels,
            ["`default` is a catch-all route, and no `route_by` declares what it falls back from"]
        );
    }
}
