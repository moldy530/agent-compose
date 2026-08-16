//! State channels: what a write supplies, where it lands, and what a flow
//! materializes out of them (grammar 7.5, 8.0, 10).
//!
//! Every write in this grammar is name-based unless a `writes:` remap says
//! otherwise, so a node's **effective write map** — each output field paired
//! with the channel it actually writes — is the object all three rules here are
//! stated over (Decision D93):
//!
//! * it must be **injective**, because the canonical write order is defined per
//!   writer and two writes to one channel from one writer have no order between
//!   them (grammar 7.6.4);
//! * every write is **typed by the channel's reduce policy** — the whole value
//!   for an unreduced or `last_wins` channel, one element for an `append`
//!   channel, a partial object for a `merge` one (grammar 10.2, Decision D58);
//! * every destination must be a **declared** channel (grammar 10.3).
//!
//! The read counterpart lives one section away: a flow's `outputs:` reads the
//! channel of the same name, which must satisfy the field it lands in (grammar
//! 7.5, Decision D111). Both directions are the same relation pointed opposite
//! ways, which is why they share [`satisfies`](super::model).

use crate::ast::document::Reduce;
use crate::diag::{Diagnostic, DiagnosticCode, Span};
use crate::ir::Channel;
use crate::ir::binding::Writes;
use crate::ir::flow::Node;
use crate::ir::schema::{FieldMap, TypeForm, TypeNode};

use super::model::satisfies;
use super::{Ctx, FlowCx, field_names, text};

/// One entry of a node's effective write map.
pub(crate) struct Written<'a> {
    /// The output field that is written.
    pub(crate) field: String,
    /// The channel it lands in.
    pub(crate) channel: &'a Channel,
    /// Where to report: the remap entry, or the node itself for a name-based
    /// write.
    pub(crate) at: Span,
}

/// Check one node's `writes:` remap and every write it implies.
pub(crate) fn node_writes<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, node: &'a Node) {
    let Some(output) = ctx.node_output(node) else {
        // A `map` node has no output of its own; its dispatched instances write
        // instead, and `maps` checks those (grammar 8.6 rules 5, 9).
        return;
    };
    let subject = format!("node `{}`", text(&node.id));
    // A name-based write is written nowhere, so it is anchored on the node that
    // makes it — at the node's *id* rather than its block, because a block
    // mapping ends where the next node's key begins and underlining a sibling
    // node names the wrong site (PRD G3).
    let written = write_map(
        ctx,
        &output,
        node.writes.as_ref(),
        &node.id.span,
        &subject,
        cx.address,
    );
    check_types(ctx, &output, &written, &subject, false);
}

/// Resolve a node's effective write map, reporting the remap's own mistakes and
/// the collisions the whole map draws (grammar 8.0, Decision D93).
pub(crate) fn write_map<'a>(
    ctx: &mut Ctx<'a>,
    output: &FieldMap,
    writes: Option<&Writes>,
    fallback: &Span,
    subject: &str,
    flow: &str,
) -> Vec<Written<'a>> {
    report_remap(ctx, output, writes, subject);
    let effective = effective(ctx, output, writes, fallback);
    report_collisions(ctx, &effective, subject, flow);
    effective
}

/// The remap's own mistakes: a key that is not an output field, a value that is
/// not a declared channel (grammar 8.0).
fn report_remap(ctx: &mut Ctx, output: &FieldMap, writes: Option<&Writes>, subject: &str) {
    let Some(writes) = writes else {
        return;
    };
    for entry in &writes.entries {
        let field = text(&entry.field);
        if output.field(field).is_none() {
            let known = field_names(output);
            let help = crate::parse::reader::suggest(field, &known).map_or_else(
                || {
                    if known.is_empty() {
                        format!("{subject} declares no output fields")
                    } else {
                        format!(
                            "the declared output fields are {}",
                            crate::parse::reader::list(&known)
                        )
                    }
                },
                |name| format!("did you mean `{name}`?"),
            );
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownField,
                    entry.field.span.clone(),
                    format!("`{field}` is not an output field of {subject}"),
                )
                .with_label(output.span.clone(), "the result schema is declared here")
                .with_help(help),
            );
            continue;
        }
        let name = text(&entry.channel);
        if ctx.channel(name).is_none() {
            undefined_channel(ctx, name, &entry.channel.span, "a `writes:` destination");
        }
    }
}

/// A node's effective write map: each output field paired with the channel it
/// actually writes (Decision D93). Silent — the mistakes are
/// [`report_remap`]'s, so a caller that only needs the map does not report them
/// a second time.
pub(crate) fn effective<'a>(
    ctx: &Ctx<'a>,
    output: &FieldMap,
    writes: Option<&Writes>,
    fallback: &Span,
) -> Vec<Written<'a>> {
    let mut effective: Vec<Written<'a>> = Vec::new();
    if let Some(writes) = writes {
        for entry in &writes.entries {
            let field = text(&entry.field);
            if output.field(field).is_none() {
                continue;
            }
            if let Some(channel) = ctx.channel(text(&entry.channel)) {
                effective.push(Written {
                    field: field.to_string(),
                    channel,
                    at: entry.channel.span.clone(),
                });
            }
        }
    }

    // Name-based destinations: every output field the remap did not move,
    // where a channel of that name is declared. A field with no channel stays
    // node-scoped and writes nothing (grammar 8.0).
    for field in &output.fields {
        let name = text(&field.name);
        if writes.is_some_and(|writes| {
            writes
                .entries
                .iter()
                .any(|entry| text(&entry.field) == name)
        }) {
            continue;
        }
        if let Some(channel) = ctx.channel(name) {
            effective.push(Written {
                field: name.to_string(),
                channel,
                at: fallback.clone(),
            });
        }
    }
    effective
}

fn report_collisions(ctx: &mut Ctx, effective: &[Written<'_>], subject: &str, flow: &str) {
    for (index, write) in effective.iter().enumerate() {
        if let Some(other) = effective
            .iter()
            .take(index)
            .find(|other| other.channel.name.value == write.channel.name.value)
        {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingWrites,
                    write.at.clone(),
                    format!(
                        "`{}` and `{}` of {subject} both write the channel `{}`",
                        other.field,
                        write.field,
                        text(&write.channel.name)
                    ),
                )
                .with_label(other.at.clone(), "the other write is here")
                .with_label(write.channel.span.clone(), "the channel is declared here")
                .with_help(format!(
                    "a node's effective write map — remapped fields and same-named channels together — must be injective, because two writes to one channel from one writer have no order between them (grammar 7.6.4, Decision D93); remap one of them in `{flow}`"
                )),
            );
        }
    }
}

/// Type every write against the reduce policy of the channel it lands in
/// (grammar 10.2), and — inside a fan-out — require that policy to exist at
/// all (grammar 8.6 rule 5).
pub(crate) fn check_types(
    ctx: &mut Ctx,
    output: &FieldMap,
    written: &[Written<'_>],
    subject: &str,
    fan_out: bool,
) {
    for write in written {
        let Some(field) = output.field(&write.field) else {
            continue;
        };
        let channel = write.channel;
        let name = text(&channel.name);
        if fan_out && channel.reduce.is_none() {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::UnreducedWrite,
                    write.at.clone(),
                    format!("{subject} writes the unreduced channel `{name}`"),
                )
                .with_label(channel.span.clone(), "the channel is declared here")
                .with_help(
                    "dispatched instances are concurrent writers, so the channel they write needs a declared `reduce:` policy — `append`, `merge`, or an explicit `last_wins` (grammar 8.6 rule 5, 10.2)",
                ),
            );
            continue;
        }
        let outcome = match channel.reduce {
            None | Some(Reduce::LastWins) => satisfies(&field.ty, &channel.ty).map_err(|mismatch| {
                (
                    format!("the channel takes its whole value: {}", mismatch.describe()),
                    None,
                )
            }),
            Some(Reduce::Append) => match &channel.ty.form {
                TypeForm::Array(array) => satisfies(&field.ty, &array.items).map_err(|mismatch| {
                    (
                        format!("an `append` write supplies one element: {}", mismatch.describe()),
                        Some(
                            "appending several values in one write is deliberately not expressible: fan out with a `map` so each element is its own write, or declare the channel `last_wins` and set it whole (grammar 10.2, Decision D58)"
                                .to_string(),
                        ),
                    )
                }),
                _ => Ok(()),
            },
            Some(Reduce::Merge) => merge_write(&field.ty, &channel.ty),
        };
        if let Err((message, help)) = outcome {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    write.at.clone(),
                    format!(
                        "`{}` of {subject} cannot be written to `{name}`: {message}",
                        write.field
                    ),
                )
                .with_label(channel.span.clone(), "the channel is declared here")
                .with_label(field.ty.span.clone(), "the output field is declared here")
                .with_optional_help(help),
            );
        }
    }
}

/// A `merge` write supplies an object whose properties are a subset of the
/// channel's, with matching types (grammar 10.2).
fn merge_write(written: &TypeNode, channel: &TypeNode) -> Result<(), (String, Option<String>)> {
    let TypeForm::Object(target) = &channel.form else {
        return Ok(());
    };
    let TypeForm::Object(source) = &written.form else {
        return Err((
            "a `merge` write supplies a partial object, and this field is not one".to_string(),
            None,
        ));
    };
    for property in &source.properties.fields {
        let name = text(&property.name);
        let Some(want) = target.properties.field(name) else {
            return Err((
                format!(
                    "a `merge` write supplies a subset of the channel's properties, and `{name}` is not one of them"
                ),
                Some(format!(
                    "the channel declares {}",
                    crate::parse::reader::list(field_names(&target.properties))
                )),
            ));
        };
        satisfies(&property.ty, &want.ty)
            .map_err(|mismatch| (format!("at `.{name}`: {}", mismatch.describe()), None))?;
    }
    Ok(())
}

/// A flow's `outputs:` are materialized from the channels of the same names
/// (grammar 7.5, Decisions D53, D111).
pub(crate) fn flow_outputs(ctx: &mut Ctx, cx: &FlowCx) {
    for field in &cx.flow.outputs.fields {
        let name = text(&field.name);
        let Some(channel) = ctx.channel(name) else {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::UndefinedChannel,
                    field.name.span.clone(),
                    format!("`{}` declares the output `{name}`, which no state channel supplies", cx.address),
                )
                .with_help(
                    "each `outputs:` field is read from the state channel of the same name at quiescence: declare it in `state:`, or feed a differently-named channel with a node `writes:` remap (grammar 7.5, Decision D53)",
                ),
            );
            continue;
        };
        if let Err(mismatch) = satisfies(&channel.ty, &field.ty) {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::TypeMismatch,
                    field.name.span.clone(),
                    format!(
                        "the channel `{name}` cannot satisfy the output `{name}` of `{}`: {}",
                        cx.address,
                        mismatch.describe()
                    ),
                )
                .with_label(channel.span.clone(), "the channel is declared here")
                .with_help(
                    "materialization reads the channel's whole value with nothing between the two declarations, so the channel must satisfy the field — a channel with no `max_items` cannot feed a bounded one (grammar 7.5, Decision D111)",
                ),
            );
        }
    }
}

/// A `state.*` reference, a `writes:` destination, or an `outputs:` field
/// naming a channel `state:` does not declare (grammar 10.3).
pub(crate) fn undefined_channel(ctx: &mut Ctx, name: &str, at: &Span, subject: &str) {
    let declared: Vec<String> = ctx
        .ir
        .state
        .as_ref()
        .map(|section| section.entries.keys().cloned().collect())
        .unwrap_or_default();
    let known: Vec<&str> = declared.iter().map(String::as_str).collect();
    let help = crate::parse::reader::suggest(name, &known).map_or_else(
        || {
            if known.is_empty() {
                "the composition declares no `state:` channels".to_string()
            } else {
                format!(
                    "the declared channels are {}",
                    crate::parse::reader::list(&known)
                )
            }
        },
        |channel| format!("did you mean `{channel}`?"),
    );
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UndefinedChannel,
            at.clone(),
            format!("`{name}` is {subject} that `state:` does not declare"),
        )
        .with_help(help),
    );
}
