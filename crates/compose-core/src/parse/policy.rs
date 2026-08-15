//! Error policy (grammar 9): `retry`, `timeout`, `on_error`.

use crate::ast::policy::{OnError, PolicyBlock, Retry};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::{Node, Yaml};

use super::lexical;
use super::reader::{Cx, Fields, expect_finite, expect_mapping, in_range, list};

/// Read a `retry:` block (grammar 9.1).
pub(crate) fn retry(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<Retry>> {
    let mapping = expect_mapping(node, subject, cx)?;
    let context = format!("the `retry` block of {subject}");
    let mut fields = Fields::new(mapping, node.span.clone(), &context);

    let max = fields
        .require("max", cx)
        .and_then(|node| super::reader::expect_integer(node, "`max`", cx))
        .filter(|value| in_range(value, "`max`", 1..=10, cx));
    let backoff = fields
        .require("backoff", cx)
        .and_then(|node| lexical::duration(node, "`backoff`", cx));
    let multiplier = fields.take("multiplier").and_then(|node| {
        let value = match &node.value {
            Yaml::Int(value) => Some(Spanned::new(*value as f64, node.span.clone())),
            Yaml::Float(value) => Some(Spanned::new(*value, node.span.clone())),
            _ => {
                cx.wrong_type(node, "`multiplier`", "a number");
                None
            }
        }?;
        // Checked before the bound, because NaN passes `< 1.0` by failing it.
        if !expect_finite(value.value, "`multiplier`", &value.span, cx) {
            return None;
        }
        if value.value < 1.0 {
            cx.error(
                DiagnosticCode::ValueOutOfRange,
                &value.span,
                format!("`multiplier` must be at least 1, found {}", value.value),
            );
            return None;
        }
        Some(value)
    });
    let max_backoff = fields
        .take("max_backoff")
        .and_then(|node| lexical::duration(node, "`max_backoff`", cx));
    let jitter = fields.boolean("jitter", cx);
    fields.finish(cx);

    Some(Spanned::new(
        Retry {
            max,
            backoff,
            multiplier,
            max_backoff,
            jitter,
            span: node.span.clone(),
        },
        node.span.clone(),
    ))
}

/// Which level of the resolution chain an `on_error:` was written at, and so
/// which forms it admits (grammar 9.2, 9.3, Decision D103).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyLevel {
    /// A node's own key — chain level 2, the only level whose `on_error:`
    /// takes the `{ fallback: … }` form, because it is the only one that names
    /// the flow the target is local to.
    Node,
    /// A composition-wide level: the `defaults:` section (level 3) or a `flow:`
    /// node's `policy:` override (level 1). Both reach nodes of flows they do
    /// not name, so `on_error:` here takes `fail` or `skip` only.
    Composition,
}

impl PolicyLevel {
    /// The `on_error:` forms this level admits, for a diagnostic's help text.
    fn strategies(self) -> String {
        match self {
            Self::Node => format!(
                "`on_error` takes {}, or `{{ fallback: <node id or end> }}`",
                list(["fail", "skip"])
            ),
            Self::Composition => format!("`on_error` takes {} here", list(["fail", "skip"])),
        }
    }
}

/// Read an `on_error:` strategy (grammar 9.2).
pub(crate) fn on_error(
    node: &Node,
    subject: &str,
    level: PolicyLevel,
    cx: &mut Cx,
) -> Option<Spanned<OnError>> {
    match &node.value {
        Yaml::String(text) => match text.as_str() {
            "fail" => Some(Spanned::new(OnError::Fail, node.span.clone())),
            "skip" => Some(Spanned::new(OnError::Skip, node.span.clone())),
            other => {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        node.span.clone(),
                        format!("`{other}` is not an `on_error` strategy in {subject}"),
                    )
                    .with_help(level.strategies()),
                );
                None
            }
        },
        // A fallback target is a node id of the flow the declaring node sits in,
        // and neither composition-wide level names a flow: `defaults:` reaches
        // every node of every flow, and a `policy:` reaches every node of the
        // instantiated subflow and of everything it instantiates in turn
        // (Decision D103).
        Yaml::Mapping(_) if level == PolicyLevel::Composition => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    node.span.clone(),
                    format!("`on_error: {{ fallback: … }}` is not legal in {subject}"),
                )
                .with_help(
                    "a fallback names a node of one flow, and this level names no flow: declare the fallback on the node that needs it, and keep `fail` or `skip` here (grammar 9.3, Decision D103)",
                ),
            );
            None
        }
        Yaml::Mapping(mapping) => {
            let context = format!("the `on_error` block of {subject}");
            let mut fields = Fields::new(mapping, node.span.clone(), &context);
            let target = fields
                .require("fallback", cx)
                .and_then(|node| lexical::control_target(node, "`fallback`", cx));
            fields.finish(cx);
            target.map(|target| Spanned::new(OnError::Fallback(target), node.span.clone()))
        }
        _ => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::WrongType,
                    node.span.clone(),
                    format!(
                        "expected an `on_error` strategy for {subject}, found {}",
                        node.description()
                    ),
                )
                .with_help(level.strategies()),
            );
            None
        }
    }
}

/// Read `retry`/`timeout`/`on_error` from an already-opened mapping.
pub(crate) fn policy_fields(
    fields: &mut Fields<'_>,
    subject: &str,
    level: PolicyLevel,
    cx: &mut Cx,
) -> PolicyBlock {
    let retry = fields
        .take("retry")
        .and_then(|node| retry_block(node, subject, cx));
    let timeout = fields
        .take("timeout")
        .and_then(|node| lexical::duration(node, "`timeout`", cx));
    let on_error = fields
        .take("on_error")
        .and_then(|node| on_error(node, subject, level, cx));
    PolicyBlock {
        retry,
        timeout,
        on_error,
    }
}

fn retry_block(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<Retry>> {
    retry(node, subject, cx)
}

/// Read a `policy:` / `defaults:` block: the same three keys, on their own, at
/// a composition-wide level of the chain (grammar 9.3).
pub(crate) fn policy_block(
    node: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<PolicyBlock>> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);
    let block = policy_fields(&mut fields, subject, PolicyLevel::Composition, cx);
    fields.finish(cx);
    Some(Spanned::new(block, node.span.clone()))
}
