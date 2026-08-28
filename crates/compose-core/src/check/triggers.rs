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
//!
//! Two rules are about the entry a trigger declares rather than about what it
//! reads. An `http` trigger's `method:`/`path:` pair MUST be free — unclaimed by
//! another trigger, and not one of the two the generated app mounts for itself
//! ([`routes`]). And two `manual` triggers naming one flow MUST NOT declare
//! different `session_key:` expressions, because the CLI entry they describe is
//! one entry ([`manual_session_keys`]). Both are rules the normative spec does
//! not state — see each for why it is kept and reported as a doc defect.

use crate::ast::trigger::TriggerMethod;
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::binding::Bindings;
use crate::ir::trigger::{HttpTrigger, Trigger, TriggerKind};

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
    routes(ctx);
    manual_session_keys(ctx);
}

/// Two `manual` triggers naming one flow that disagree about `session_key:`
/// (grammar 13.2).
///
/// A `manual` trigger declares no route. What it does declare — the only key
/// that changes what running its flow *does* — is a `session_key:` **remap** of
/// the CLI's `--session`, over a payload whose one member is that argument. The
/// CLI entry it describes is per **flow**: `agent-compose run flow.f` names a
/// flow, never a trigger, and §13's preamble makes the entry universal and
/// unnamed ("it contributes no entry to the IR's trigger table and has no
/// name"). So two `manual` triggers naming one flow describe one entry, and two
/// different `session_key:` expressions on it are two answers to "which
/// partition does this run address" with nothing to choose between them.
///
/// Declaring **no** `session_key:` is not a third answer: the key defaults to
/// `"payload.session"`, which is the argument itself, so a trigger that leaves
/// it out asks for whatever the entry already has. Only two *written*
/// expressions can disagree, and they disagree when their sources differ as
/// written — two spellings of one meaning are still two spellings, and this
/// check does not interpret CEL.
///
/// # This rule is the compiler's, not yet the spec's
///
/// Like the trigger-against-trigger half of [`routes`], and for the same reason:
/// §13.2 makes the remap legal and §13's preamble makes the entry per flow, but
/// no sentence and no Decision entry says what two of them on one flow mean, and
/// `schemas/agent-compose.schema.json` accepts it. Refused here because the
/// alternative is an emitted CLI silently picking one — the posture D50, D61 and
/// D113 refuse everywhere else in this grammar. Reported as a doc defect: the
/// grammar wants a sentence in §13.2, and this check is written to be exactly
/// what that sentence would say.
fn manual_session_keys(ctx: &mut Ctx) {
    let Some(triggers) = ctx.ir.triggers.as_ref() else {
        return;
    };
    // The walk is the IR's canonical order — by name (Decision D55) — so the
    // trigger reported is the later of the two by name and the earlier one is
    // labelled, whichever files they were declared in.
    let mut declared: Vec<(String, &Trigger, &Spanned<crate::ast::common::Cel>)> = Vec::new();
    let mut conflicts: Vec<Diagnostic> = Vec::new();
    for trigger in triggers.entries.values() {
        if !matches!(trigger.kind, TriggerKind::Manual) {
            continue;
        }
        let Some(key) = trigger.session_key.as_ref() else {
            continue;
        };
        let flow = trigger.flow.value.to_string();
        if let Some((_, first, first_key)) = declared
            .iter()
            .find(|(named, _, other)| named == &flow && other.value.as_str() != key.value.as_str())
        {
            let name = text(&trigger.name);
            let first_name = text(&first.name);
            conflicts.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingSessionKey,
                    key.span.clone(),
                    format!(
                        "the trigger `{name}` declares `session_key: {}` for `{flow}`, and the trigger `{first_name}` declares `session_key: {}` for the same flow",
                        key.value.as_str(),
                        first_key.value.as_str()
                    ),
                )
                .with_label(
                    first_key.span.clone(),
                    format!("`{first_name}` declares it here"),
                )
                .with_help(
                    "`agent-compose run` names a flow rather than a trigger, so one flow has one CLI entry and one session identity: give the two triggers the same `session_key:`, leave it off the one that has no opinion, or point one at another flow (grammar 13.2)",
                ),
            );
        }
        declared.push((flow, trigger, key));
    }
    for conflict in conflicts {
        ctx.push(conflict);
    }
}

/// An `http` trigger claiming a route that is already claimed.
///
/// The generated app mounts one route per declared `http` trigger, at its
/// effective `path:` and `method:` — the pair each one *defaults* rather than
/// the pair each one writes, since `path:` defaults to `/triggers/<name>` and
/// `method:` to `POST` (§13.3's table). It also mounts two of its own, at fixed
/// addresses and whatever the composition says, because §13.3 has it expose
/// `status` and `resume` beside the per-trigger `start`
/// ([`crate::codegen::serve::RESERVED_ROUTES`]). A router cannot dispatch one
/// pair two ways, so a *second* claim on one of those pairs — by another
/// trigger, or by the app itself — makes the app refuse to start: Fastify
/// answers `FST_ERR_DUPLICATED_ROUTE` at `listen`, and had it not, every request
/// to that route would have to run two things with no answer to "which one"
/// written anywhere. That is a guaranteed runtime failure visible in the trigger
/// object, and refusing it at compile time naming the construct is the standing
/// posture §7.6.3 takes on a guaranteed dead end and §13.3 takes on a `GET`
/// reading `payload.body`.
///
/// # Half of this rule is the compiler's, not yet the spec's
///
/// The **reserved** half follows from a sentence §13.3 writes: the app exposes
/// those routes, so they are taken, and a trigger cannot have one. The
/// **trigger-against-trigger** half does not. §13.3 fixes the mount model it is
/// derived from — `path:` is "route of the generated app", it defaults to
/// `/triggers/<name>`, `method:` defaults to `POST` — but says nothing about two
/// triggers landing on one pair, no Decision entry covers it (D117 is
/// `payload.body` on a bodyless request), and `schemas/agent-compose.schema.json`
/// accepts it. So a spec that an editor validates green is refused here.
///
/// That direction is the one Appendix B sanctions — "a file that passes the
/// schema and fails `validate` is normal and expected", and the schema cannot
/// see two sibling triggers' *effective* routes anyway — but the appendix
/// enumerates the rules it defers, and this one is not among them. Reported as a
/// doc defect: the grammar wants a sentence in §13.3 and a Decision entry, and
/// the check is written to be exactly what that sentence would say, so ratifying
/// it changes no behaviour. It is kept rather than dropped because dropping it
/// ships the failure instead of the diagnostic.
///
/// **Exact pairs only.** A router's parameter syntax "passes through
/// unexamined" (§13.3), so `/reviews/:id` beside `/reviews/:name` is a conflict
/// this check does not see: deciding it means knowing that both are one segment
/// pattern, which is a property of the router rather than of the grammar. What
/// is decidable here is decided here, and the router still reports the rest —
/// which `src/cli.ts` now reports as the route collision it is rather than as a
/// failure to take the address.
fn routes(ctx: &mut Ctx) {
    let Some(triggers) = ctx.ir.triggers.as_ref() else {
        return;
    };
    // Built first and pushed after, because the walk borrows the artifact the
    // report is written into.
    //
    // The order walked is the IR's canonical one, which is by **name** rather
    // than by file layout (Decision D55): the trigger reported is the later of
    // the two by name and the earlier one is labelled. Two triggers can be
    // declared in two files, so a report keyed to the order the files happened
    // to be read in would move under an `imports:` reordering that changed
    // nothing.
    let mut claimed: Vec<(String, &'static str, &Trigger, &HttpTrigger)> = Vec::new();
    let mut collisions: Vec<Diagnostic> = Vec::new();
    for trigger in triggers.entries.values() {
        let TriggerKind::Http(http) = &trigger.kind else {
            continue;
        };
        let path = route_path(trigger, http);
        let method = route_method(http);
        if crate::codegen::serve::RESERVED_ROUTES
            .iter()
            .any(|(verb, reserved)| *verb == method && *reserved == path)
        {
            let name = text(&trigger.name);
            collisions.push(
                Diagnostic::error(
                    DiagnosticCode::DuplicateRoute,
                    route_span(trigger, http),
                    format!(
                        "the trigger `{name}` declares the route `{method} {path}`, which the generated app mounts for itself"
                    ),
                )
                .with_help(format!(
                    "a generated app mounts `status` and `resume` for itself, beside each trigger's `start` (grammar 13.3) — {} — and cannot dispatch one method and path two ways: give the trigger its own `path:`",
                    crate::parse::reader::list(
                        crate::codegen::serve::RESERVED_ROUTES
                            .iter()
                            .map(|(verb, reserved)| format!("{verb} {reserved}"))
                            .collect::<Vec<_>>()
                    )
                )),
            );
        }
        if let Some((_, _, first, first_http)) = claimed
            .iter()
            .find(|(taken, at, _, _)| taken == &path && *at == method)
        {
            let name = text(&trigger.name);
            let first_name = text(&first.name);
            collisions.push(
                Diagnostic::error(
                    DiagnosticCode::DuplicateRoute,
                    route_span(trigger, http),
                    format!(
                        "the trigger `{name}` declares the route `{method} {path}`, which the trigger `{first_name}` already declares"
                    ),
                )
                .with_label(
                    route_span(first, first_http),
                    format!("`{first_name}` claims it here"),
                )
                .with_help(
                    "the generated app mounts one route per `http` trigger and cannot dispatch one method and path two ways: give one of them its own `path:` or `method:` (grammar 13.3)",
                ),
            );
        }
        claimed.push((path, method, trigger, http));
    }
    for collision in collisions {
        ctx.push(collision);
    }
}

/// The route a trigger is mounted at, defaults applied (grammar 13.3).
fn route_path(trigger: &Trigger, http: &HttpTrigger) -> String {
    http.path.as_ref().map_or_else(
        || format!("/triggers/{}", text(&trigger.name)),
        |path| path.value.clone(),
    )
}

/// The method a trigger is mounted at, defaulted to `POST` (grammar 13.3).
const fn route_method(http: &HttpTrigger) -> &'static str {
    match http.method {
        Some(TriggerMethod::Get) => "GET",
        Some(TriggerMethod::Put) => "PUT",
        Some(TriggerMethod::Post) | None => "POST",
    }
}

/// Where a route collision is pointed at: the `path:` that claims it, or the
/// whole trigger when the path is the defaulted one and there is nothing else
/// to underline.
fn route_span(trigger: &Trigger, http: &HttpTrigger) -> Span {
    http.path
        .as_ref()
        .map_or_else(|| trigger.span.clone(), |path| path.span.clone())
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
