//! Fan-out: what `over` resolves to, how a routed map narrows it, and what a
//! dispatch target may be bound from (grammar 8.6).
//!
//! `over` is a **path expression** so that the array's schema can be resolved
//! statically (grammar 4.2, Decision D43), and this module is where that
//! resolution happens: root, then member selections and literal indexes, over
//! the *declared* schemas rather than over the CEL type lattice — because the
//! two things the resolved node is needed for, `max_items` and the union it
//! narrows, are declarations rather than result types.
//!
//! What the resolution proves is grammar 8.6 rule 1's bounding half. Its other
//! half — that `<node>` **dominates** the map node (rule 11) — is a graph
//! property and is the next pass's; this one checks that the node exists and
//! that the path through its output resolves.
//!
//! Rules 3, 4 and 12 follow from the item type: a union item requires
//! `route_by:` and a non-union item forbids it; every variant is routed or
//! caught by `default:`; a named route sees **its variant's payload only**, and
//! `default:` sees the discriminator plus what every unrouted variant declares;
//! and each dispatch binds with the form its target's input contract takes.

use std::collections::BTreeSet;

use crate::ast::common::{Address, Ident, PathExpr, PathStep};
use crate::ast::schema::Surface;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::binding::{NodeInput, Writes};
use crate::ir::flow::{Map, MapDispatch, MapRoute, Node};
use crate::ir::schema::{Field, FieldMap, TypeForm, TypeNode, UnionType};

use super::model::{self, satisfies};
use super::{Ctx, FlowCx, InputContract, bindings, channels, expr, reach, text};

/// Check one `map` node (grammar 8.6).
pub(crate) fn map_node<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, node: &'a Node, map: &'a Map) {
    let id = text(&node.id).to_string();
    let subject = format!("the `map` of node `{id}`");
    let Some(item) = over(ctx, cx, &map.over, &subject) else {
        return;
    };
    let binding = map
        .item_binding
        .as_ref()
        .map_or_else(|| Ident::new("item"), |name| name.value.clone());

    match &map.dispatch {
        MapDispatch::Homogeneous {
            node: target,
            input,
            writes,
            detach,
        } => {
            if let TypeForm::Union(union) = &item.form {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::MissingKey,
                        map.span.clone(),
                        format!("{subject} dispatches a `{}` union with no `route_by:`", union.discriminator.value),
                    )
                    .with_label(item.span.clone(), "the item type is declared here")
                    .with_help(
                        "union items require routing: declare `route_by:` and a `routes:` entry per variant, so each target is type-checked against its own variant's payload (grammar 8.6 rules 3, 4)",
                    ),
                );
                return;
            }
            dispatch(
                ctx,
                cx,
                &subject,
                "the map's item",
                &binding,
                &item,
                &target.value,
                input.as_ref(),
                writes.as_ref(),
                detach.as_ref(),
                &map.span,
            );
        }
        MapDispatch::Routed {
            route_by,
            routes,
            default,
        } => {
            let TypeForm::Union(union) = &item.form else {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::TypeMismatch,
                        route_by.span.clone(),
                        format!("{subject} declares `route_by:`, but its items are not a discriminated union"),
                    )
                    .with_label(item.span.clone(), "the item type is declared here")
                    .with_help(
                        "`route_by:` narrows a union per variant, so it is legal only where `over` resolves to an array of one (grammar 8.6 rule 3)",
                    ),
                );
                return;
            };
            if route_by.value != union.discriminator.value {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::TypeMismatch,
                        route_by.span.clone(),
                        format!(
                            "`route_by: {}` is not the discriminator of the item union, which is `{}`",
                            route_by.value, union.discriminator.value
                        ),
                    )
                    .with_label(union.discriminator.span.clone(), "the discriminator is declared here")
                    .with_help("`route_by:` is a literal field name and must equal the union's own discriminator (grammar 8.6 rules 5, 8)"),
                );
                return;
            }
            routed(
                ctx,
                cx,
                &subject,
                &binding,
                union,
                routes,
                default.as_deref(),
                &map.span,
            );
        }
    }
}

/// The named routes, the catch-all, and the variants each one sees
/// (grammar 8.6 rule 4).
#[allow(clippy::too_many_arguments)]
fn routed<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'a>,
    subject: &str,
    binding: &Ident,
    union: &UnionType,
    routes: &'a [MapRoute],
    default: Option<&'a MapRoute>,
    at: &Span,
) {
    let mut served: BTreeSet<String> = BTreeSet::new();
    for route in routes {
        let Some(tag) = &route.tag else {
            continue;
        };
        let Some(variant) = union
            .variants
            .iter()
            .find(|variant| variant.tag.value == tag.value)
        else {
            let known: Vec<&str> = union
                .variants
                .iter()
                .map(|variant| variant.tag.value.as_str())
                .collect();
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownVariant,
                    tag.span.clone(),
                    format!("`{}` is not a variant of the item union", tag.value),
                )
                .with_label(
                    union.discriminator.span.clone(),
                    "the union is declared here",
                )
                .with_help(
                    crate::parse::reader::suggest(tag.value.as_str(), &known).map_or_else(
                        || format!("the variants are {}", crate::parse::reader::list(&known)),
                        |variant| format!("did you mean `{variant}`?"),
                    ),
                ),
            );
            continue;
        };
        served.insert(tag.value.to_string());
        // A named route's target sees its variant's payload only — the
        // narrowing PRD 5.6 requires (grammar 8.6 rule 4).
        let item = narrowed(
            &union.discriminator.value,
            &[tag.value.as_str()],
            &variant.fields,
            &route.span,
        );
        dispatch(
            ctx,
            cx,
            &format!("the `{}` route of {subject}", tag.value),
            &format!("the item narrowed to `{}`", tag.value),
            binding,
            &item,
            &route.node.value,
            route.input.as_ref(),
            route.writes.as_ref(),
            route.detach.as_ref(),
            &route.span,
        );
    }

    let unrouted: Vec<&crate::ir::schema::UnionVariant> = union
        .variants
        .iter()
        .filter(|variant| !served.contains(variant.tag.value.as_str()))
        .collect();

    if let Some(default) = default {
        if unrouted.is_empty() {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    default.span.clone(),
                    format!("the `default:` route of {subject} is unreachable"),
                )
                .with_help(
                    "every variant of the item union already has a route, so nothing reaches the catch-all: drop it, or drop the route that covers the variant it was written for (grammar 8.6 rule 4, Decision D30)",
                ),
            );
        }
        let tags: Vec<&str> = unrouted
            .iter()
            .map(|variant| variant.tag.value.as_str())
            .collect();
        // The catch-all is narrowed to the *unrouted* variants: the
        // discriminator, and any field every one of them declares identically
        // (grammar 8.6 rule 4, Decision D30).
        let item = narrowed(
            &union.discriminator.value,
            &tags,
            &common_fields(&unrouted, &default.span),
            &default.span,
        );
        dispatch(
            ctx,
            cx,
            &format!("the `default:` route of {subject}"),
            "the item narrowed to the unrouted variants",
            binding,
            &item,
            &default.node.value,
            default.input.as_ref(),
            default.writes.as_ref(),
            default.detach.as_ref(),
            &default.span,
        );
    } else if !unrouted.is_empty() {
        let missing: Vec<&str> = unrouted
            .iter()
            .map(|variant| variant.tag.value.as_str())
            .collect();
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::NonExhaustive,
                at.clone(),
                format!(
                    "{subject} routes no target for {}",
                    crate::parse::reader::list(&missing)
                ),
            )
            .with_label(union.discriminator.span.clone(), "the union is declared here")
            .with_help(
                "every variant of the item union needs a `routes:` entry, or a `default:` catch-all: the agent structurally cannot produce an unroutable item (grammar 8.6 rule 4)",
            ),
        );
    }
}

/// One dispatch: the per-item binding form, its fields, and its writes
/// (grammar 8.6 rules 5, 7, 12).
#[allow(clippy::too_many_arguments)]
fn dispatch<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'a>,
    subject: &str,
    item_label: &str,
    binding: &Ident,
    item: &TypeNode,
    target: &Address,
    input: Option<&NodeInput>,
    writes: Option<&Writes>,
    detach: Option<&Spanned<bool>>,
    at: &Span,
) {
    let contract = ctx.target_input(target);
    let scope = expr::item_scope(ctx, cx, binding, model::type_of_named(item, item_label));

    match (input, contract) {
        (Some(NodeInput::Scalar { value }), InputContract::StringIn) => {
            let analysis = expr::analyze(ctx, value, &scope);
            expr::expect(
                ctx,
                value,
                &analysis,
                &crate::cel::ty::Type::String,
                &format!("the scalar `input:` of {subject}"),
            );
        }
        (Some(NodeInput::Scalar { value }), contract) => {
            // What the target *does* declare is the half of this the author
            // needs, and "named input fields" is false of a target that
            // declares none — a `flow.*` with no `inputs:`, a no-argument tool.
            let contract_note = match contract.declared() {
                Some(fields) if !fields.is_empty() => format!(
                    "`{target}` declares {}",
                    crate::parse::reader::list(super::field_names(fields))
                ),
                _ => format!("`{target}` declares no input fields"),
            };
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    value.span.clone(),
                    format!("{subject} binds one unnamed value, but `{target}` is not a string-in agent"),
                )
                .with_help(format!(
                    "{contract_note}: the scalar form binds a string-in agent's single unnamed input, and every other target takes the field-map form (grammar 8.6 rule 12, Decision D75)"
                )),
            );
        }
        (Some(NodeInput::Fields { bindings }), InputContract::StringIn) => {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    bindings.span.clone(),
                    format!("{subject} binds fields by name, but `{target}` is a string-in agent"),
                )
                .with_help(
                    "a string-in agent takes the scalar form — `input: \"item.summary\"` is how an object item feeds one (grammar 8.6 rule 12, Decision D75)",
                ),
            );
        }
        (Some(NodeInput::Fields { bindings }), contract) => {
            bindings::field_bindings(ctx, bindings, &scope, contract, subject);
            if let Some(fields) = contract.declared() {
                for field in &fields.fields {
                    let name = text(&field.name);
                    let bound = bindings
                        .entries
                        .iter()
                        .any(|binding| binding.name.value == name);
                    if !bound && model::default_of(&field.ty).is_none() {
                        ctx.push(
                            Diagnostic::error(
                                DiagnosticCode::MissingBinding,
                                bindings.span.clone(),
                                format!("`{name}` of `{target}` is not bound by {subject}"),
                            )
                            .with_label(field.name.span.clone(), "the input field is declared here")
                            .with_help(
                                "a dispatch is a module boundary: every input field without a `default:` is bound explicitly, and nothing falls through by name (grammar 8.0, Decision D68)",
                            ),
                        );
                    }
                }
            }
        }
        (None, contract) => whole_item(ctx, subject, item, target, contract, at),
    }

    // Anything a dispatched instance writes to shared state must target a
    // reduced channel (grammar 8.6 rule 5) — and a **detached** dispatch must
    // write no state at all (rule 7).
    if let Some(output) = ctx.target_output(target) {
        let written = channels::write_map(ctx, output, writes, at, subject, cx.address);
        match detach.filter(|detach| detach.value) {
            Some(detach) => {
                for write in &written {
                    detached_write(
                        ctx,
                        detach,
                        &write.at,
                        write.channel,
                        format!(
                            "{subject} is detached, and `{}` of `{target}` writes the channel `{}`",
                            write.field,
                            text(&write.channel.name)
                        ),
                    );
                }
            }
            // A dispatch issued from inside a detached instance is fire-and-
            // forget too, whatever this map says: rule 7 refuses these writes
            // over the instance that carries them
            // ([`check_dispatched_writes`]), so rule 5's verdict on the same
            // write is not added on top of it.
            None if !ctx.detached(cx.address).is_empty() => {}
            None => channels::check_types(ctx, output, &written, subject, true),
        }
    }
}

/// One write a detached dispatch makes (grammar 8.6 rule 7, Decisions D31,
/// D94).
///
/// The parser refuses the `writes:` half of the rule where it is written
/// (`detach: true` beside a remap is a `conflicting-keys` error); what only the
/// composition shows is the *name-based* half — an output field that lands in a
/// channel of the same name, with no remap to refuse.
fn detached_write(
    ctx: &mut Ctx,
    detach: &Spanned<bool>,
    at: &Span,
    channel: &crate::ir::Channel,
    message: String,
) {
    ctx.push(
        Diagnostic::error(DiagnosticCode::DetachedWrite, at.clone(), message)
            .with_label(detach.span.clone(), "the dispatch is detached here")
            .with_label(channel.span.clone(), "the channel is declared here")
            .with_help(
                "a detached dispatch is resolved at dispatch: the join is over the moment it is issued and its outcome is never observed, so what it wrote would land — or not — after everything that reads it (grammar 8.6 rule 7, Decisions D31, D94); drop `detach: true`, or send the result out through the target itself",
            ),
    );
}

/// With no `input:`, the whole item is the instance's input, which then has to
/// be schema-compatible with what the target declares (grammar 8.6 rule 12).
fn whole_item(
    ctx: &mut Ctx,
    subject: &str,
    item: &TypeNode,
    target: &Address,
    contract: InputContract<'_>,
    at: &Span,
) {
    match contract {
        InputContract::StringIn => {
            let is_string = matches!(
                &item.form,
                TypeForm::Scalar(scalar) if scalar.kind == crate::ast::schema::ScalarKind::String
            ) || matches!(&item.form, TypeForm::Enum(_));
            if !is_string {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::TypeMismatch,
                        at.clone(),
                        format!("{subject} passes the whole item to the string-in agent `{target}`, but the item is not a string"),
                    )
                    .with_label(item.span.clone(), "the item type is declared here")
                    .with_help(
                        "a string-in target takes the whole item only when the item's own type is `string`; otherwise select one with the scalar form, `input: \"item.<field>\"` (grammar 8.6 rule 12)",
                    ),
                );
            }
        }
        InputContract::Fields(fields) => {
            declared_object(ctx, subject, item, target, fields, Some(&fields.span), at);
        }
        // A flow with no `inputs:` has the parameter surface `inputs: {}`
        // declares — a closed object with no properties (grammar 3.9) — so the
        // whole item meets the same contract here as it does at a no-argument
        // tool. The two spellings would otherwise disagree about one dispatch,
        // and the one that says nothing about its inputs would be the one that
        // accepts anything. There is simply no declaration to point at.
        InputContract::Empty => {
            let none = model::field_map(Vec::new(), at);
            declared_object(ctx, subject, item, target, &none, None, at);
        }
        InputContract::AdHoc | InputContract::Unknown => {}
    }
}

/// The whole item against a target's declared input object (grammar 8.6
/// rule 12). `declared_at` is where that object was written, for the targets
/// that wrote one.
fn declared_object(
    ctx: &mut Ctx,
    subject: &str,
    item: &TypeNode,
    target: &Address,
    fields: &FieldMap,
    declared_at: Option<&Span>,
    at: &Span,
) {
    let object = model::node(
        TypeForm::Object(crate::ir::schema::ObjectType {
            properties: fields.clone(),
            optional: Vec::new(),
            default: None,
        }),
        declared_at.unwrap_or(at),
    );
    let Err(mismatch) = satisfies(item, &object) else {
        return;
    };
    let mut diagnostic = Diagnostic::error(
        DiagnosticCode::TypeMismatch,
        at.clone(),
        format!(
            "{subject} passes the whole item to `{target}`, which cannot accept it: {}",
            mismatch.describe()
        ),
    );
    if let Some(declared_at) = declared_at {
        diagnostic =
            diagnostic.with_label(declared_at.clone(), "the target's input is declared here");
    }
    ctx.push(
        diagnostic
            .with_label(item.span.clone(), "the item type is declared here")
            .with_help(if fields.fields.is_empty() {
                // "Bind the target's fields" is advice a target with none
                // cannot take: what fits it is a dispatch that passes nothing.
                format!(
                    "`{target}` declares no input fields, so the only per-item binding that fits it is the empty one: write `input: {{}}` to dispatch it without passing the item (grammar 8.6 rule 12)"
                )
            } else {
                "bind the target's fields from the item with `input: { <field>: \"item.<field>\" }`, or leave `input:` out only where the whole item is schema-compatible with the declared object (grammar 8.6 rule 12)".to_string()
            }),
    );
}

/// The item type a route sees: the discriminator, narrowed to the tags that
/// reach it, plus the payload fields (grammar 8.6 rule 4).
fn narrowed(discriminator: &Ident, tags: &[&str], fields: &FieldMap, span: &Span) -> TypeNode {
    let mut properties = vec![(discriminator.as_str(), model::enum_node(tags, span))];
    let mut owned: Vec<(&str, TypeNode)> = fields
        .fields
        .iter()
        .map(|field| (field.name.value.as_str(), field.ty.clone()))
        .collect();
    properties.append(&mut owned);
    model::object_node(properties, span)
}

/// The fields every unrouted variant declares identically — what `default:`
/// may select (grammar 8.6 rule 4, Decision D30).
fn common_fields(variants: &[&crate::ir::schema::UnionVariant], span: &Span) -> FieldMap {
    let mut fields: Vec<Field> = Vec::new();
    let Some((first, rest)) = variants.split_first() else {
        return model::field_map(Vec::new(), span);
    };
    for field in &first.fields.fields {
        // Two variants declare one field *identically* when they declare the
        // same type, not when they declare it in the same place: comparing the
        // type nodes with `==` would reach the spans they carry and no two
        // variants would ever share anything (see `model::identical`).
        let shared = rest.iter().all(|variant| {
            variant
                .fields
                .field(field.name.value.as_str())
                .is_some_and(|other| model::identical(&other.ty, &field.ty))
        });
        if shared {
            fields.push(field.clone());
        }
    }
    FieldMap {
        surface: Surface::Result,
        fields,
        span: span.clone(),
    }
}

/// Resolve `over` against the declared schemas, and require the array it lands
/// on to be bounded (grammar 8.6 rule 1, Decision D10).
fn over(ctx: &mut Ctx, cx: &FlowCx, path: &Spanned<PathExpr>, subject: &str) -> Option<TypeNode> {
    let (resolved, ty) = resolve(ctx, cx, path)?;
    let array = match resolved.as_ref().map(|node| &node.form) {
        Some(TypeForm::Array(array)) => array,
        _ => {
            // Either the path landed on a declaration that is not an array, or
            // it stopped short of one — `over: "plan.output"` is the whole
            // result object, and a `map` fans out over an array.
            let mut diagnostic = Diagnostic::error(
                DiagnosticCode::TypeMismatch,
                path.span.clone(),
                format!(
                    "`over` of {subject} must resolve to an array, and `{}` resolves to {ty}",
                    path.value.as_str()
                ),
            )
            .with_help(
                "a `map` fans out over an array: an agent emits one and the map dispatches over it (grammar 8.6 rule 1)",
            );
            if let Some(resolved) = &resolved {
                diagnostic = diagnostic.with_label(resolved.span.clone(), "it is declared here");
            }
            ctx.push(diagnostic);
            return None;
        }
    };
    if array.max_items.is_none() {
        let at = resolved
            .as_ref()
            .map_or_else(|| path.span.clone(), |node| node.span.clone());
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnboundedFanOut,
                path.span.clone(),
                format!("`over` of {subject} resolves to an array with no `max_items`"),
            )
            .with_label(at, "the array is declared here")
            .with_help(
                "bounding is mandatory on both axes: declare `max_items` on the array a `map.over` resolves to, and `max_concurrency` on the map node (grammar 3.5, 8.6 rule 1)",
            ),
        );
    }
    Some((*array.items).clone())
}

/// Walk a path expression over the declared schemas (grammar 4.2).
///
/// `None` means the expression itself was refused and reported. Otherwise the
/// declaration the path lands on — absent where it stops on a synthesized
/// object such as a whole `<node>.output`, which no single declaration is — and
/// the CEL type of the whole path, which is what a message describes it by.
fn resolve(
    ctx: &mut Ctx,
    cx: &FlowCx,
    path: &Spanned<PathExpr>,
) -> Option<(Option<TypeNode>, crate::cel::ty::Type)> {
    // The roots are the same ones the CEL front-end would expose, so an
    // unknown root, an unknown member, and an undefined channel are reported in
    // the one voice grammar 4.1 fixes for them.
    let scope = expr::over_scope(ctx, cx);
    let source = path.value.as_str();
    let analysis = crate::cel::analyze(source, &scope);
    if !analysis.problems.is_empty() {
        for problem in &analysis.problems {
            ctx.push(
                Diagnostic::error(problem.code, path.span.clone(), problem.message.clone())
                    .with_optional_help(problem.help.clone()),
            );
        }
        return None;
    }
    let described = analysis.ty;

    // Each root takes its first step from a different declaration: a channel,
    // a flow input, or — behind the fixed `output` selector, which is why that
    // root consumes two steps — a node's result schema (grammar 4.1).
    let steps = &path.value.steps;
    let (mut resolved, consumed) = match path.value.root.as_str() {
        "state" => {
            let Some(PathStep::Field(name)) = steps.first() else {
                return Some((None, described));
            };
            match ctx.channel(name.as_str()) {
                Some(channel) => (channel.ty.clone(), 1),
                None => return Some((None, described)),
            }
        }
        "input" => {
            let Some(PathStep::Field(name)) = steps.first() else {
                return Some((None, described));
            };
            match cx
                .flow
                .inputs
                .as_ref()
                .and_then(|inputs| inputs.field(name.as_str()))
            {
                Some(field) => (field.ty.clone(), 1),
                None => return Some((None, described)),
            }
        }
        _ => {
            let output = ctx
                .node_of(cx.flow, &path.value.root)
                .and_then(|node| ctx.node_output(node));
            let field = match (output, steps.first(), steps.get(1)) {
                (Some(output), Some(PathStep::Field(selector)), Some(PathStep::Field(name)))
                    if selector.as_str() == "output" =>
                {
                    output.field(name.as_str()).map(|field| field.ty.clone())
                }
                _ => None,
            };
            match field {
                Some(field) => (field, 2),
                None => return Some((None, described)),
            }
        }
    };

    for step in steps.iter().skip(consumed) {
        let next = match (step, &resolved.form) {
            (PathStep::Field(name), TypeForm::Object(object)) => object
                .properties
                .field(name.as_str())
                .map(|field| field.ty.clone()),
            (PathStep::Index(_), TypeForm::Array(array)) => Some((*array.items).clone()),
            _ => None,
        };
        match next {
            Some(next) => resolved = next,
            // The expression type-checked, so a step this walk cannot follow is
            // one that lands on no single declaration; the CEL type still
            // describes it.
            None => return Some((None, described)),
        }
    }
    Some((Some(resolved), described))
}

/// Everything a dispatched flow instance writes has to target a reduced
/// channel, however deep inside the instance the writer is (grammar 8.6
/// rule 5) — and a **detached** instance may write nothing at all (rule 7).
pub(crate) fn check_dispatched_writes(ctx: &mut Ctx) {
    // Rule 7 first. A detached instance is resolved at dispatch, so every write
    // it makes lands after the join it was counted in — and that is as true of
    // the instances it dispatches in turn as of its own nodes, which is why the
    // relation it is quantified over is `detached_instances` rather than a
    // dispatch site's frames (Decision D94).
    for instance in ctx.detached_instances() {
        for node in &instance.flow.nodes {
            for write in writes_of(ctx, node) {
                detached_write(
                    ctx,
                    &instance.detach,
                    &node.span,
                    write.channel,
                    format!(
                        "{} detaches `{}`, whose node `{}` writes the channel `{}`",
                        instance.dispatcher,
                        instance.address,
                        text(&node.id),
                        text(&write.channel.name)
                    ),
                );
            }
        }
    }

    // Rule 5, over every site that is not one of those: a write rule 7 has
    // already refused is not also reported as unreduced, because an instance
    // rule 7 speaks about may write nothing at all — reduced or not.
    for frame in reach::frames(ctx) {
        if frame.detached || !ctx.detached(frame.owner).is_empty() {
            continue;
        }
        for node in &frame.flow.nodes {
            let Some(output) = ctx.node_output(node) else {
                continue;
            };
            // Silent resolution: the remap's own mistakes are reported where
            // the node is checked as part of its own flow.
            let written = channels::effective(ctx, &output, node.writes.as_ref(), &node.span);
            for write in &written {
                if write.channel.reduce.is_none() {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::UnreducedWrite,
                            node.span.clone(),
                            format!(
                                "{} dispatches `{}`, whose node `{}` writes the unreduced channel `{}`",
                                frame.dispatcher,
                                frame.address,
                                text(&node.id),
                                text(&write.channel.name)
                            ),
                        )
                        .with_label(write.channel.span.clone(), "the channel is declared here")
                        .with_label(frame.span.clone(), "the dispatch is here")
                        .with_help(
                            "dispatched instances are concurrent writers, so every channel they write needs a declared `reduce:` policy (grammar 8.6 rule 5, 10.2)",
                        ),
                    );
                }
            }
        }
    }
}

/// Every channel one node of a flow writes: its own result mapped onto the
/// channels (grammar 8.0), and — for a `map` node, which has no result of its
/// own (rule 9) — what each of its dispatch targets writes at the dispatch
/// site.
///
/// Silent, like [`channels::effective`]: a remap's own mistakes are reported
/// where the node is checked as part of its own flow. A dispatch that wrote
/// `detach: true` itself is left out, because [`dispatch`] reports what it
/// writes at that dispatch, in the message that names it.
fn writes_of<'a>(ctx: &Ctx<'a>, node: &'a Node) -> Vec<channels::Written<'a>> {
    if let crate::ir::flow::NodeKind::Map { map } = &node.kind {
        let mut written = Vec::new();
        for dispatch in reach::dispatches(&map.dispatch) {
            if dispatch.is_detached() {
                continue;
            }
            let Some(output) = ctx.target_output(dispatch.target) else {
                continue;
            };
            written.extend(channels::effective(
                ctx,
                output,
                dispatch.writes,
                &node.span,
            ));
        }
        return written;
    }
    let Some(output) = ctx.node_output(node) else {
        return Vec::new();
    };
    channels::effective(ctx, &output, node.writes.as_ref(), &node.span)
}
