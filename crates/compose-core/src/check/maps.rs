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
//!
//! Rules 5 and 7 are about **shared state**, so they are stated over the
//! dispatch site's effective write map and nowhere else. A dispatched `flow.*`
//! is a flow *instance*: the channel set is composition-global in shape but
//! each instance holds its own values, so a node inside the instance writes
//! that instance's values and only the instance's `outputs:` cross the boundary
//! (grammar 10.1, 7.6.4 rules 2 and 3, PRD 5.6's "isolated item-scoped
//! contexts"). What crosses is what rule 5 requires a `reduce:` policy for —
//! the instances of one map are concurrent with each other (7.6.1) — and what
//! rule 7 refuses outright at a detached dispatch. A channel written only
//! *inside* a dispatched instance has one writer per instance and no race for
//! either rule to prevent; the concurrency of that flow's own nodes is its own
//! flow's question (7.6.1), asked wherever that flow is checked.
//!
//! A detached dispatch carries one more thing rule 7 gives it — the idempotency
//! key of 9.4 — and to an `exec:`-bound sink it carries it in the environment
//! that sink's own input fields arrive in. Which of the two claims the slot is
//! decidable only over the composition, so [`delivery_slot`] decides it here.

use std::collections::BTreeSet;

use crate::ast::common::{Address, Ident, PathExpr, PathStep};
use crate::ast::schema::Surface;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::binding::{NodeInput, Writes};
use crate::ir::flow::{Map, MapDispatch, MapRoute, Node, ToolImplementation};
use crate::ir::schema::{Field, FieldMap, TypeForm, TypeNode, UnionType};

use super::model::{self, satisfies};
use super::{Ctx, FlowCx, InputContract, bindings, channels, expr, text};

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

    // A detached dispatch carries the idempotency key of grammar 9.4, and to an
    // `exec:`-bound sink it carries it in the environment — which is where that
    // sink's declared input fields arrive too.
    if let Some(detach) = detach.filter(|detach| detach.value) {
        delivery_slot(ctx, subject, target, detach);
    }

    // What a dispatched instance writes to the **shared** state of the flow
    // that dispatched it is this site's effective write map, and nothing else.
    // A `flow.*` instance holds its own channel *values* — the channel set is
    // composition-global in shape only — so a node inside it writes that
    // instance's values, and only the instance's `outputs:`, materialized at
    // its quiescence, cross the boundary (grammar 10.1, 7.6.4 rules 2 and 3).
    // Rule 5 requires every write that does cross to land in a reduced channel,
    // because the instances of one map are concurrent with each other (7.6.1);
    // rule 7 refuses those writes outright where the dispatch is detached.
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
            None => channels::check_types(ctx, output, &written, subject, true),
        }
    }
}

/// The variable grammar 9.4's delivery surface names for an `exec:`-bound
/// target, and the name an input field of that target would arrive under.
const IDEMPOTENCY_ENV: &str = "IDEMPOTENCY_KEY";

/// The slot a detached delivery claims in an `exec:`-bound sink's environment
/// (grammar 9.4, 8.6 rule 7, Decision D66).
///
/// Grammar 9.4 fixes the delivery surface per binding kind, and only one of the
/// three shares a namespace with the target's declared `input:`. An `http:`
/// target reads the key out of a request *header* and a `function:` target out
/// of its *invocation context*, both of which sit beside the input object; an
/// `exec:` target reads it out of the environment, which is the very place
/// grammar 6.1 delivers that target's input fields to, upper-snake-cased. So a
/// sink declaring `idempotency_key` claims the slot the delivery claims, and
/// the two values cannot both arrive.
///
/// Which is Decision D66's rule exactly — "wherever an input object becomes
/// environment variables … a key colliding with the upper-snake-cased name of
/// one of them is a compile error", because "silently preferring one leaves the
/// other as a key that changes nothing". D66 states it of the binding's own
/// `env:`, which the parser refuses where it is written; a delivery is the
/// third writer into that environment, and the composition is the only place it
/// is visible — the tool alone is legal, and so is every dispatch of it whose
/// outcome is observed, because grammar 9.4 gives a key to nothing else.
///
/// Reported against the `detach:` that creates the pair rather than against the
/// field, for the same reason: the field is not wrong, dispatching *to* it
/// detached is. Grammar 9.4's own sentence is the rule being enforced — "the key
/// is delivery metadata, never part of the target's declared input schema" —
/// and this is the one composition under which a target can contradict it.
fn delivery_slot(ctx: &mut Ctx, subject: &str, target: &Address, detach: &Spanned<bool>) {
    let Some(tool) = ctx.tool(target) else {
        return;
    };
    if !matches!(tool.implementation, ToolImplementation::Exec { .. }) {
        return;
    }
    for field in &tool.input.fields {
        let name = text(&field.name);
        if !name.eq_ignore_ascii_case(IDEMPOTENCY_ENV) {
            continue;
        }
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingKeys,
                detach.span.clone(),
                format!(
                    "{subject} delivers `{IDEMPOTENCY_ENV}`, which is also where the input field `{name}` of `{target}` arrives"
                ),
            )
            .with_label(field.name.span.clone(), "this input field takes the same slot")
            .with_help(
                "a detached dispatch delivers its idempotency key to an `exec:`-bound sink as the `IDEMPOTENCY_KEY` environment variable, and an object input arrives as upper-snake-cased environment variables, so one of the two values would be silently discarded: rename the field, or drop `detach: true` (grammar 9.4, 8.6 rule 7, Decision D66)",
            ),
        );
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
///
/// `pub(crate)` because the emitter narrows the same way: a route's per-item
/// expressions are evaluated against the variant's payload, so the shape the
/// item is *bound* through at run time has to be the shape it was type-checked
/// against here (grammar 4.1's declared-type reading). Two spellings of one
/// narrowing rule is exactly the drift that would make an `integer` variant
/// field a `double` in a compiled router.
pub(crate) fn narrowed(
    discriminator: &Ident,
    tags: &[&str],
    fields: &FieldMap,
    span: &Span,
) -> TypeNode {
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
///
/// `pub(crate)` for [`narrowed`]'s reason: the emitter binds a `default:`
/// route's item through this same intersection.
pub(crate) fn common_fields(
    variants: &[&crate::ir::schema::UnionVariant],
    span: &Span,
) -> FieldMap {
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
    let (resolved, consumed) = match path.value.root.as_str() {
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

    match walk(resolved, &steps[consumed.min(steps.len())..]) {
        Some(resolved) => Some((Some(resolved), described)),
        // The expression type-checked, so a step this walk cannot follow is one
        // that lands on no single declaration; the CEL type still describes it.
        None => Some((None, described)),
    }
}

/// Follow the remaining steps of a path expression over the declared schemas
/// (grammar 4.2).
///
/// `pub(crate)` because the emitter resolves `over` too — it needs the item's
/// own declared type to bind the item root through, and a second walk would be
/// a second reading of grammar 4.2's two selectors.
pub(crate) fn walk(mut resolved: TypeNode, steps: &[PathStep]) -> Option<TypeNode> {
    for step in steps {
        resolved = match (step, &resolved.form) {
            (PathStep::Field(name), TypeForm::Object(object)) => object
                .properties
                .field(name.as_str())
                .map(|field| field.ty.clone())?,
            (PathStep::Index(_), TypeForm::Array(array)) => (*array.items).clone(),
            _ => return None,
        };
    }
    Some(resolved)
}
