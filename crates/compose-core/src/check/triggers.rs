//! Trigger input bindings and what a payload can supply (grammar 13).
//!
//! `input:` is legal on `http`, `schedule`, and `event` triggers and illegal on
//! `manual` ones — which the parser decides, because both halves are in one
//! object (Decision D44). What needs the composition is the other half of
//! grammar 13.1: the bindings are checked against the **target flow's**
//! `inputs`, which is another file.
//!
//! * every field the flow declares without a `default:` MUST be bound;
//! * a binding for a field the flow does not declare, or whose CEL result the
//!   field cannot accept, is a compile error.
//!
//! A `manual` trigger binds nothing, and the identical check runs against
//! `--input k=v` at run start, against the same schema — the one place this
//! check is deferred, for the same reason env-ref presence is: the values do
//! not exist until the command runs (grammar 13.1, 13.2).
//!
//! The payload itself is typed per trigger type, and one rule reads what an
//! expression does with it: a `method: GET` trigger MUST NOT read *through*
//! `payload.body`, which is `{}` on every request such a trigger can receive,
//! so the read fails every time (grammar 13.3, Decision D117).

use crate::ast::trigger::TriggerMethod;
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::binding::Bindings;
use crate::ir::trigger::{Trigger, TriggerKind};

use super::model::default_of;
use super::{Ctx, expr, field_names, text};

/// Check every declared trigger (grammar 13).
pub(crate) fn check(ctx: &mut Ctx) {
    let ir = ctx.ir;
    let Some(triggers) = ir.triggers.as_ref() else {
        return;
    };
    for trigger in triggers.entries.values() {
        one(ctx, trigger);
    }
}

fn one(ctx: &mut Ctx, trigger: &Trigger) {
    let name = text(&trigger.name);
    let scope = expr::trigger_scope(&trigger.kind, format!("the trigger `{name}`"));
    let bindings = match &trigger.kind {
        TriggerKind::Manual => None,
        TriggerKind::Http(http) => http.input.as_ref(),
        TriggerKind::Schedule(schedule) => schedule.input.as_ref(),
        TriggerKind::Event(event) => event.input.as_ref(),
    };

    // The three string-valued expressions a trigger may carry (grammar 13.1,
    // 13.3, 13.5).
    let mut strings: Vec<(&str, &Spanned<crate::ast::common::Cel>)> = Vec::new();
    if let Some(key) = &trigger.session_key {
        strings.push(("session_key", key));
    }
    if let TriggerKind::Http(http) = &trigger.kind
        && let Some(callback) = &http.callback
    {
        strings.push(("callback", callback));
    }
    if let TriggerKind::Event(event) = &trigger.kind
        && let Some(dedupe) = &event.dedupe_key
    {
        strings.push(("dedupe_key", dedupe));
    }
    for (key, expression) in strings {
        let analysis = expr::analyze(ctx, expression, &scope);
        expr::expect(
            ctx,
            expression,
            &analysis,
            &Type::String,
            &format!("`{key}` of the trigger `{name}`"),
        );
        bodyless(ctx, trigger, expression, &analysis, key);
    }

    let Some(bindings) = bindings else {
        return;
    };
    let flow = trigger.flow.value.to_string();
    let Some(definition) = ctx.flow_named(&flow) else {
        return;
    };
    let inputs = definition.inputs.clone();
    check_bindings(ctx, trigger, bindings, inputs.as_ref(), &flow, &scope);
}

fn check_bindings(
    ctx: &mut Ctx,
    trigger: &Trigger,
    bindings: &Bindings,
    inputs: Option<&crate::ir::schema::FieldMap>,
    flow: &str,
    scope: &crate::cel::Scope,
) {
    let name = text(&trigger.name);
    for binding in &bindings.entries {
        let field = inputs.and_then(|inputs| inputs.field(binding.name.value.as_str()));
        if field.is_none() {
            let known = inputs.map(field_names).unwrap_or_default();
            let help = crate::parse::reader::suggest(binding.name.value.as_str(), &known)
                .map_or_else(
                    || {
                        if known.is_empty() {
                            format!("`{flow}` declares no `inputs:`")
                        } else {
                            format!(
                                "the declared inputs are {}",
                                crate::parse::reader::list(&known)
                            )
                        }
                    },
                    |field| format!("did you mean `{field}`?"),
                );
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownField,
                    binding.name.span.clone(),
                    format!(
                        "`{}` is not an input of `{flow}`, which the trigger `{name}` targets",
                        binding.name.value
                    ),
                )
                .with_help(help),
            );
        }
        let analysis = expr::analyze(ctx, &binding.value, scope);
        bodyless(ctx, trigger, &binding.value, &analysis, "input");
        if let Some(field) = field {
            expr::expect_field(
                ctx,
                &binding.value,
                &analysis,
                &field.ty,
                &format!("the binding for `{}`", binding.name.value),
            );
        }
    }

    let Some(inputs) = inputs else {
        return;
    };
    for field in &inputs.fields {
        let field_name = text(&field.name);
        if bindings
            .entries
            .iter()
            .any(|binding| binding.name.value.as_str() == field_name)
            || default_of(&field.ty).is_some()
        {
            continue;
        }
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::MissingBinding,
                bindings.span.clone(),
                format!("the trigger `{name}` binds no `{field_name}`, which `{flow}` requires"),
            )
            .with_label(field.name.span.clone(), "the flow input is declared here")
            .with_help(
                "a trigger's `input:` is checked against its flow's `inputs:`: every field without a `default:` must be bound (grammar 13.1)",
            ),
        );
    }
}

/// A `method: GET` trigger reading *through* `payload.body` (grammar 13.3,
/// Decision D117).
fn bodyless(
    ctx: &mut Ctx,
    trigger: &Trigger,
    expression: &Spanned<crate::ast::common::Cel>,
    analysis: &crate::cel::Analysis,
    key: &str,
) {
    let TriggerKind::Http(http) = &trigger.kind else {
        return;
    };
    if http.method != Some(TriggerMethod::Get) {
        return;
    }
    let Some(read) = analysis
        .reads
        .iter()
        .find(|read| read.reads_through("payload", "body"))
    else {
        return;
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidExpression,
            expression.span.clone(),
            format!(
                "`{}` of the trigger `{}` reads `{}`, and a `GET` carries no body",
                key,
                text(&trigger.name),
                read.spelling()
            ),
        )
        .with_help(
            "`payload.body` is `{}` on every request a `GET` trigger can receive, so reading a member of it fails every one of them: bind from `payload.query` instead, which is where a `GET`'s parameters are (grammar 13.3, Decision D117)",
        ),
    );
}
