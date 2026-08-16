//! Running the CEL front-end at each of grammar 4.1's surfaces.
//!
//! Grammar 4.1 is a table of surfaces and the roots each one exposes, and this
//! module is that table: one constructor per row, each building the [`Scope`]
//! the front-end walks against, plus the two ways a caller states what it
//! expects back — a fixed type ([`expect`]) or a declared destination
//! ([`expect_field`]).
//!
//! Two rows are deliberately wider here than the table reads, and both widen in
//! the direction the next pass narrows:
//!
//! * **`map.over`** exposes `<node>.output` for *every* node of the flow, while
//!   the table exposes it for the nodes that **dominate** the map node (grammar
//!   8.6 rule 11). Dominance is a graph property; resolving the path is a
//!   schema one. Each half is checked where its evidence is, and a path naming
//!   a node that exists but does not dominate is this pass's silence and the
//!   graph pass's diagnostic.
//! * an **edge guard** leaving `start` exposes no node handle at all, which is
//!   the table's own second row: `start` has no output (grammar 2.4).

use crate::ast::common::{Cel, EdgeSource, Ident};
use crate::cel::ty::{Origin, Property, Type};
use crate::cel::{Analysis, Scope};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::flow::{Edge, Flow, Node};
use crate::ir::schema::{FieldMap, TypeNode};
use crate::ir::trigger::TriggerKind;

use super::model;
use super::{Ctx, FlowCx, text};

/// Parse and check one expression, reporting everything the front-end found
/// against the expression's own span.
pub(crate) fn analyze(ctx: &mut Ctx, expression: &Spanned<Cel>, scope: &Scope) -> Analysis {
    let analysis = crate::cel::analyze(expression.value.as_str(), scope);
    for problem in &analysis.problems {
        ctx.push(
            Diagnostic::error(
                problem.code,
                expression.span.clone(),
                problem.message.clone(),
            )
            .with_optional_help(problem.help.clone()),
        );
    }
    analysis
}

/// Require an expression's result to be of a fixed type.
pub(crate) fn expect(
    ctx: &mut Ctx,
    expression: &Spanned<Cel>,
    analysis: &Analysis,
    want: &Type,
    subject: &str,
) {
    if analysis.ty.assignable_to(want) {
        return;
    }
    ctx.error(
        DiagnosticCode::TypeMismatch,
        &expression.span,
        format!("{subject} must evaluate to {want}, found {}", analysis.ty),
    );
}

/// Require an expression's result to be a legal value of a declared field.
///
/// This is grammar 8.0's step 1: an explicit binding is typed by its *result*,
/// so what is compared is the result's shape and not the constraints of a
/// declaration it does not have (see [`model`]).
pub(crate) fn expect_field(
    ctx: &mut Ctx,
    expression: &Spanned<Cel>,
    analysis: &Analysis,
    field: &TypeNode,
    subject: &str,
) {
    let want = model::type_of(field);
    let Err(mismatch) = analysis.ty.check_assignable(&want) else {
        return;
    };
    let help = match (&analysis.ty, &want) {
        (Type::String, Type::Enum(_)) => Some(
            "an enum field takes one of its variants: bind it from an enum-typed source or write the variant as a literal"
                .to_string(),
        ),
        (Type::Int, Type::Double) | (Type::Double, Type::Int) => Some(
            "CEL has no implicit numeric conversion: an `integer` field takes an integer expression and a `number` field takes either"
                .to_string(),
        ),
        _ => None,
    };
    // Where the two are objects, "expects this object, found this object" is
    // the whole of what the two shapes can say for themselves, so the message
    // is the member they disagree at instead (PRD G3).
    let at = if mismatch.path.is_empty() {
        String::new()
    } else {
        format!(" at `{}`", mismatch.path())
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::TypeMismatch,
            expression.span.clone(),
            format!(
                "{subject} expects {}, found {}{at}",
                mismatch.expected, mismatch.found
            ),
        )
        .with_label(field.span.clone(), "the destination is declared here")
        .with_optional_help(help),
    );
}

// --- the surfaces of grammar 4.1 -----------------------------------------

/// The `input` root of a flow-scoped surface: the enclosing flow's own input
/// object (grammar 4.1).
fn flow_input(flow: &Flow, address: &str) -> Type {
    flow.inputs.as_ref().map_or_else(
        || {
            Type::object(
                Origin::Declared(format!("`{address}`'s inputs")),
                Vec::new(),
            )
        },
        |inputs| model::object_of(inputs, format!("`{address}`'s inputs")),
    )
}

/// The `execution` root: run metadata, with a fixed member set (grammar 4.1).
///
/// `item_index` is present here at every surface. Whether it *has a value* is a
/// property of the site rather than of the composition — the same flow is a
/// dispatch target at one instantiation and a root instance at another — so
/// grammar 4.1 and Decision D115 make an absent index a runtime failure, not a
/// static one.
fn execution() -> Type {
    let member = |name: &str, ty: Type| Property {
        name: name.to_string(),
        ty,
        optional: false,
        defaulted: false,
    };
    Type::object(
        Origin::Execution,
        vec![
            member("id", Type::String),
            member("session_key", Type::String),
            member("item_index", Type::Int),
        ],
    )
}

/// `input`, `state`, `execution` — the roots every flow-scoped surface but
/// `map.over` exposes.
pub(crate) fn flow_scope(ctx: &Ctx, cx: &FlowCx, surface: impl Into<String>) -> Scope {
    Scope::new(
        surface,
        vec![
            ("input".to_string(), flow_input(cx.flow, cx.address)),
            ("state".to_string(), ctx.state_type()),
            ("execution".to_string(), execution()),
        ],
    )
}

/// A node handle: the root of `<node>.output.<field>` (grammar 4.1).
fn node_handle(ctx: &Ctx, node: &Node) -> Type {
    let id = text(&node.id).to_string();
    let output = ctx.node_output(node);
    let properties = output.map_or_else(Vec::new, |output| {
        vec![Property {
            name: "output".to_string(),
            ty: model::object_of(&output, format!("`{id}`'s output")),
            optional: false,
            defaulted: false,
        }]
    });
    Type::object(
        Origin::Node {
            id,
            has_output: !properties.is_empty(),
        },
        properties,
    )
}

/// The scope of an edge guard: the source node's output, and the flow's own
/// roots (grammar 4.1).
pub(crate) fn edge_guard(ctx: &mut Ctx, cx: &FlowCx, edge: &Edge) {
    let Some(guard) = &edge.when else {
        return;
    };
    let mut scope = flow_scope(ctx, cx, "an edge guard");
    if let EdgeSource::Node(id) = &edge.from.value
        && let Some(node) = ctx.node_of(cx.flow, id)
    {
        scope
            .roots
            .insert(0, (id.as_str().to_string(), node_handle(ctx, node)));
    }
    let analysis = analyze(ctx, guard, &scope);
    expect(ctx, guard, &analysis, &Type::Bool, "an edge guard");
    // A guard that did not type-check is not read again by the routing
    // analyses: grammar 7.3.1's table reads an unrecognized term as `∅`, so a
    // guard already reported as a mistake would come back a second time as a
    // variant it fails to cover, or as a fork it fails to be exclusive with
    // (see `routing`, `convergence`).
    if !analysis.problems.is_empty() || !analysis.ty.assignable_to(&Type::Bool) {
        ctx.reject_guard(&guard.span);
    }
}

/// The scope of a `map.over` path: every node of the flow, plus `input` and
/// `state` — no `execution`, which grammar 4.1's row does not expose.
pub(crate) fn over_scope(ctx: &Ctx, cx: &FlowCx) -> Scope {
    let mut roots: Vec<(String, Type)> = cx
        .flow
        .nodes
        .iter()
        .map(|node| (text(&node.id).to_string(), node_handle(ctx, node)))
        .collect();
    roots.push(("input".to_string(), flow_input(cx.flow, cx.address)));
    roots.push(("state".to_string(), ctx.state_type()));
    Scope::new("a `map.over` path", roots)
}

/// The scope of a `map`'s per-item bindings: the item, and the enclosing
/// flow's roots (grammar 4.1, 8.6 rule 12).
pub(crate) fn item_scope(ctx: &Ctx, cx: &FlowCx, binding: &Ident, item: Type) -> Scope {
    let mut scope = flow_scope(ctx, cx, "a `map` per-item binding");
    scope.roots.insert(0, (binding.as_str().to_string(), item));
    scope
}

/// The scope inside a `tool.*` implementation binding: the tool's own declared
/// `input:`, and nothing else (grammar 6.1, Decision D65).
pub(crate) fn tool_scope(input: &FieldMap, address: &str) -> Scope {
    Scope::new(
        format!("a `{address}` implementation binding"),
        vec![(
            "input".to_string(),
            model::object_of(input, format!("`{address}`'s input")),
        )],
    )
}

/// The scope of a trigger's expressions: `payload`, shaped by the trigger type
/// (grammar 13).
pub(crate) fn trigger_scope(kind: &TriggerKind, surface: impl Into<String>) -> Scope {
    let property = |name: &str, ty: Type| Property {
        name: name.to_string(),
        ty,
        optional: false,
        defaulted: false,
    };
    let payload = match kind {
        // The CLI's `--session <key>` is the manual payload's one member
        // (grammar 13.2).
        TriggerKind::Manual => Type::object(
            Origin::Payload("manual"),
            vec![property("session", Type::String)],
        ),
        TriggerKind::Http(_) => Type::object(
            Origin::Payload("http"),
            vec![
                // A decoded JSON object: present and readable, with a shape the
                // spec never declares (grammar 13.3, Decision D117).
                property("body", Type::Map(std::sync::Arc::new(Type::Dyn))),
                property("query", Type::Map(std::sync::Arc::new(Type::String))),
                property("headers", Type::Map(std::sync::Arc::new(Type::String))),
                property("path", Type::String),
                property("method", Type::String),
            ],
        ),
        TriggerKind::Schedule(_) => Type::object(
            Origin::Payload("schedule"),
            vec![
                property("scheduled_at", Type::String),
                property("trigger", Type::String),
            ],
        ),
        TriggerKind::Event(_) => Type::object(
            Origin::Payload("event"),
            vec![
                property("id", Type::String),
                property("body", Type::Map(std::sync::Arc::new(Type::Dyn))),
                property("attributes", Type::Map(std::sync::Arc::new(Type::String))),
                property("source", Type::String),
            ],
        ),
    };
    Scope::new(surface, vec![("payload".to_string(), payload)])
}
