//! Error policy (grammar 9): `retry`, `timeout`, `on_error`.

use crate::ast::policy::{OnError, PolicyBlock, Retry};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::{Node, Yaml};

use super::lexical;
use super::reader::{Cx, Fields, expect_mapping, in_range, list};

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

/// Read an `on_error:` strategy (grammar 9.2).
pub(crate) fn on_error(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<OnError>> {
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
                    .with_help(format!(
                        "`on_error` takes {}, or `{{ fallback: <node id or end> }}`",
                        list(["fail", "skip"])
                    )),
                );
                None
            }
        },
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
                .with_help(format!(
                    "`on_error` takes {}, or `{{ fallback: <node id or end> }}`",
                    list(["fail", "skip"])
                )),
            );
            None
        }
    }
}

/// Read `retry`/`timeout`/`on_error` from an already-opened mapping.
pub(crate) fn policy_fields(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> PolicyBlock {
    let retry = fields
        .take("retry")
        .and_then(|node| retry_block(node, subject, cx));
    let timeout = fields
        .take("timeout")
        .and_then(|node| lexical::duration(node, "`timeout`", cx));
    let on_error = fields
        .take("on_error")
        .and_then(|node| on_error(node, subject, cx));
    PolicyBlock {
        retry,
        timeout,
        on_error,
    }
}

fn retry_block(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<Retry>> {
    retry(node, subject, cx)
}

/// Read a `policy:` / `defaults:` block: the same three keys, on their own
/// (grammar 9.3).
pub(crate) fn policy_block(
    node: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<PolicyBlock>> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);
    let block = policy_fields(&mut fields, subject, cx);
    fields.finish(cx);
    Some(Spanned::new(block, node.span.clone()))
}
