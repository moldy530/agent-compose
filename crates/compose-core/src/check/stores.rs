//! Store ops, map-write keying, and session-scope coherence (grammar 11).
//!
//! # The op catalog
//!
//! Grammar 11.4's table is normative in both directions: which parameters an op
//! takes — which the parser already decides, because `op:` is a literal in the
//! same node — and what those parameters *hold*, which needs the store
//! definition and so is decided here. So is the one half of the row the parser
//! cannot read: **which kind** takes the op, since the kind is the store's and
//! the store may be declared in another file. `value` on a `kv set` is checked
//! against the store's `value_schema`, `filter` and `metadata` keys against its
//! `metadata_schema`, and `key`, `query` and `prefix` are each one expression
//! whose result must be a string.
//!
//! # The partial updates and the conditions (PRD resolved q68)
//!
//! `merge`, `append` and `incr` are the schema check of a `set` extended to
//! three more rows. A `merge`'s `fields:` is checked like a `set`'s `value:`
//! except that it is not total — it names the fields it changes. An `append`'s
//! `field:` must be an **array** field of `V` and its `value:` one item of it;
//! an `incr`'s `field:` must be an `integer` or `number` field and its `by:` a
//! value of that field's type. `if_version:` is an integer expression, and a
//! version is a `kv` key's: the one op that takes it on another kind — `delete`
//! — is refused there.
//!
//! A `vector` store that declares **no** `metadata_schema` has no metadata at
//! all rather than empty metadata, so `filter:` and `metadata:` there have no
//! legal key and naming any is an error against the store (Decision D114).
//!
//! # Keying a write inside a fan-out
//!
//! A store write performed inside a `map`-dispatched instance must have an
//! **item-derived key**, or be a keyed `kv` write (grammar 11.4, Decision D67).
//! Item-derivation is traced through the dispatch binding rather than sniffed
//! at the store node (Decision D83): a `key: "input.doc_id"` whose `doc_id` is
//! bound from `state.topic` is the *same* value in every instance, which is the
//! hazard the rule exists to prevent wearing a key-shaped mask. The trace is
//! [`reach::frames`], and it is per dispatch site, because one flow may be
//! dispatched by several maps and instantiated outside every map as well.
//!
//! # Session coherence
//!
//! A declared `http`, `schedule`, or `event` trigger whose flow **reaches** a
//! `session`-scoped store must declare `session_key:` (grammar 11.3). `manual`
//! triggers carry `"payload.session"` by default and satisfy the check
//! statically; implicit invocation is not a trigger and is quantified over by
//! nothing (Decision D64).

use crate::ast::common::{Address, Ident};
use crate::ast::definition::{StoreKind, StoreScope};
use crate::ast::flow::StoreOp;
use crate::ast::schema::ScalarKind;
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::binding::Bindings;
use crate::ir::flow::{Node, NodeKind, StoreParams, StoreValue};
use crate::ir::schema::{FieldMap, TypeForm};
use crate::ir::trigger::TriggerKind;

use super::model::{self, default_of};
use super::{Ctx, FlowCx, expr, field_names, reach, text};

/// Check one store-op node's parameters against its row and its store
/// (grammar 11.4).
pub(crate) fn store_node<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'a>,
    node: &'a Node,
    store: &Spanned<Address>,
    op: StoreOp,
    params: &'a StoreParams,
) {
    let id = text(&node.id).to_string();
    let subject = format!("node `{id}`");
    let scope = expr::flow_scope(ctx, cx, format!("a store-op parameter of node `{id}`"));
    let Some(definition) = ctx.store(&store.value) else {
        return;
    };

    // The half of the row only the store can answer: which kind serves this
    // op. Nothing further is read off a node whose op its store does not
    // take — every other rule below is written for a row that exists.
    if !model::kind_takes(definition.kind, op) {
        let kind = definition.kind;
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                store.span.clone(),
                format!(
                    "{subject} asks `{}` for a `{}`, which a `{}` store does not take",
                    store.value,
                    op.as_str(),
                    kind.as_str()
                ),
            )
            .with_help(format!(
                "a `{}` store takes {} (grammar 11.4)",
                kind.as_str(),
                crate::parse::reader::list(kind.operations())
            )),
        );
        return;
    }
    if definition.kind != StoreKind::Kv
        && let Some(version) = &params.if_version
    {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                version.span.clone(),
                format!(
                    "`if_version` is not a parameter of a `{}` `{}`: a version is what a `kv` key carries, and `{}` is `kind: {}`",
                    definition.kind.as_str(),
                    op.as_str(),
                    store.value,
                    definition.kind.as_str()
                ),
            )
            .with_help(
                "only a `kv` store's writes are versioned and conditional, so drop `if_version:`, or keep the record in a `kv` store (grammar 11.4, PRD resolved q68)",
            ),
        );
    }

    for (name, expression) in [
        ("key", params.key.as_ref()),
        ("query", params.query.as_ref()),
        ("prefix", params.prefix.as_ref()),
    ] {
        let Some(expression) = expression else {
            continue;
        };
        let analysis = expr::analyze(ctx, expression, &scope);
        expr::expect(
            ctx,
            expression,
            &analysis,
            &Type::String,
            &format!("`{name}` of {subject}"),
        );
    }

    match &params.value {
        Some(StoreValue::Expression { value }) => {
            let analysis = expr::analyze(ctx, value, &scope);
            expr::expect(
                ctx,
                value,
                &analysis,
                &Type::String,
                &format!("`value` of {subject}"),
            );
        }
        Some(StoreValue::Fields { bindings }) => {
            let Some(schema) = definition.value_schema.as_ref() else {
                return;
            };
            schema_bindings(
                ctx,
                bindings,
                &scope,
                schema,
                &format!("`value` of {subject}"),
                &store.value.to_string(),
                true,
            );
        }
        None => {}
    }

    if let Some(fields) = &params.fields
        && let Some(schema) = definition.value_schema.as_ref()
    {
        schema_bindings(
            ctx,
            fields,
            &scope,
            schema,
            &format!("`fields` of {subject}"),
            &store.value.to_string(),
            false,
        );
    }
    if let Some(field) = &params.field
        && let Some(schema) = definition.value_schema.as_ref()
    {
        partial_field(ctx, &scope, &subject, store, op, schema, field, params);
    }
    if let Some(version) = &params.if_version
        && definition.kind == StoreKind::Kv
    {
        let analysis = expr::analyze(ctx, version, &scope);
        expr::expect(
            ctx,
            version,
            &analysis,
            &Type::Int,
            &format!("`if_version` of {subject}"),
        );
    }

    for (name, map) in [
        ("filter", params.filter.as_ref()),
        ("metadata", params.metadata.as_ref()),
    ] {
        let Some(map) = map else {
            continue;
        };
        let subject = format!("`{name}` of {subject}");
        match definition.metadata_schema.as_ref() {
            Some(schema) => schema_bindings(
                ctx,
                map,
                &scope,
                schema,
                &subject,
                &store.value.to_string(),
                false,
            ),
            None => {
                for binding in &map.entries {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::UnknownField,
                            binding.name.span.clone(),
                            format!(
                                "`{}` is not a metadata field: `{}` declares no `metadata_schema`",
                                binding.name.value, store.value
                            ),
                        )
                        .with_help(
                            "a `vector` store that declares no `metadata_schema` has no metadata at all, so `filter:` and `metadata:` have no legal key; `metadata_schema: {}` is the other spelling, declaring one that is always empty (grammar 11.4, Decision D114)",
                        ),
                    );
                    expr::analyze(ctx, &binding.value, &scope);
                }
            }
        }
    }
}

/// The one field an `append` or an `incr` addresses, and the expression that
/// goes with it: an array field and one of its items, or a numeric field and a
/// value of its type (grammar 11.4, PRD resolved q68).
#[allow(
    clippy::too_many_arguments,
    reason = "each is one fact the two rows read, and a struct would only rename them"
)]
fn partial_field(
    ctx: &mut Ctx,
    scope: &crate::cel::Scope,
    subject: &str,
    store: &Spanned<Address>,
    op: StoreOp,
    schema: &FieldMap,
    field: &Spanned<Ident>,
    params: &StoreParams,
) {
    let name = field.value.as_str();
    let Some(declared) = schema.field(name) else {
        let known = field_names(schema);
        let help = crate::parse::reader::suggest(name, &known).map_or_else(
            || {
                if known.is_empty() {
                    format!("`{}` declares no fields there", store.value)
                } else {
                    format!(
                        "the declared fields are {}",
                        crate::parse::reader::list(&known)
                    )
                }
            },
            |field| format!("did you mean `{field}`?"),
        );
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownField,
                field.span.clone(),
                format!("`{name}` is not a field `{}` declares", store.value),
            )
            .with_label(schema.span.clone(), "the store's schema is declared here")
            .with_help(help),
        );
        // The expression is still read, so a mistake in it is reported beside
        // this one rather than after the field is repaired.
        for expression in [
            params.value.as_ref().and_then(expression_of),
            params.by.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            expr::analyze(ctx, expression, scope);
        }
        return;
    };
    let is = model::describe_form(&declared.ty.form);
    match op {
        StoreOp::Append => {
            let TypeForm::Array(array) = &declared.ty.form else {
                let arrays: Vec<&str> = model::array_fields(Some(schema))
                    .into_iter()
                    .map(|(field, _)| field.name.value.as_str())
                    .collect();
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::TypeMismatch,
                        field.span.clone(),
                        format!(
                            "`append` adds one item to an array field, and `{name}` of `{}` is {is}",
                            store.value
                        ),
                    )
                    .with_label(declared.name.span.clone(), "the field is declared here")
                    .with_help(if arrays.is_empty() {
                        format!(
                            "`{}` declares no array field, so nothing it stores can be appended to: write the whole value with `set`, or change one field with `merge` (grammar 11.4)",
                            store.value
                        )
                    } else {
                        format!(
                            "name an array field — `{}` declares {} — or write the field with `merge` (grammar 11.4)",
                            store.value,
                            crate::parse::reader::list(&arrays)
                        )
                    }),
                );
                if let Some(value) = params.value.as_ref().and_then(expression_of) {
                    expr::analyze(ctx, value, scope);
                }
                return;
            };
            if let Some(value) = params.value.as_ref().and_then(expression_of) {
                let analysis = expr::analyze(ctx, value, scope);
                expr::expect_field(
                    ctx,
                    value,
                    &analysis,
                    &array.items,
                    &format!("`value` of {subject}, one item of `{name}`,"),
                );
            }
        }
        StoreOp::Incr => {
            let numeric = matches!(
                &declared.ty.form,
                TypeForm::Scalar(scalar)
                    if matches!(scalar.kind, ScalarKind::Integer | ScalarKind::Number)
            );
            if !numeric {
                let numbers: Vec<&str> = model::numeric_fields(Some(schema))
                    .into_iter()
                    .map(|field| field.name.value.as_str())
                    .collect();
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::TypeMismatch,
                        field.span.clone(),
                        format!(
                            "`incr` adds to a numeric field, and `{name}` of `{}` is {is}",
                            store.value
                        ),
                    )
                    .with_label(declared.name.span.clone(), "the field is declared here")
                    .with_help(if numbers.is_empty() {
                        format!(
                            "`{}` declares no `integer` or `number` field, so nothing it stores can be added to: write the field with `merge` (grammar 11.4)",
                            store.value
                        )
                    } else {
                        format!(
                            "name an `integer` or `number` field — `{}` declares {} — or write the field with `merge` (grammar 11.4)",
                            store.value,
                            crate::parse::reader::list(&numbers)
                        )
                    }),
                );
                if let Some(by) = &params.by {
                    expr::analyze(ctx, by, scope);
                }
                return;
            }
            if let Some(by) = &params.by {
                let analysis = expr::analyze(ctx, by, scope);
                expr::expect_field(
                    ctx,
                    by,
                    &analysis,
                    &declared.ty,
                    &format!("`by` of {subject}, added to `{name}`,"),
                );
            }
        }
        _ => {}
    }
}

/// The one expression a `value:` holds, where it holds one rather than a field
/// map.
const fn expression_of(value: &StoreValue) -> Option<&Spanned<crate::ast::common::Cel>> {
    match value {
        StoreValue::Expression { value } => Some(value),
        StoreValue::Fields { .. } => None,
    }
}

/// Bindings checked against a store schema: `value` against `value_schema`,
/// `filter`/`metadata` against `metadata_schema` (grammar 11.4).
fn schema_bindings(
    ctx: &mut Ctx,
    map: &Bindings,
    scope: &crate::cel::Scope,
    schema: &FieldMap,
    subject: &str,
    store: &str,
    total: bool,
) {
    for binding in &map.entries {
        let name = binding.name.value.as_str();
        let field = schema.field(name);
        if field.is_none() {
            let known = field_names(schema);
            let help = crate::parse::reader::suggest(name, &known).map_or_else(
                || {
                    if known.is_empty() {
                        format!("`{store}` declares no fields there")
                    } else {
                        format!(
                            "the declared fields are {}",
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
                    format!("`{name}` is not a field `{store}` declares"),
                )
                .with_label(schema.span.clone(), "the store's schema is declared here")
                .with_help(help),
            );
        }
        let analysis = expr::analyze(ctx, &binding.value, scope);
        if let Some(field) = field {
            expr::expect_field(
                ctx,
                &binding.value,
                &analysis,
                &field.ty,
                &format!("the `{name}` entry of {subject}"),
            );
        }
    }
    if !total {
        return;
    }
    // A `kv set` replaces the whole value at its key, so every field the store
    // declares without a `default:` has to be supplied.
    for field in &schema.fields {
        let name = text(&field.name);
        if map
            .entries
            .iter()
            .any(|binding| binding.name.value.as_str() == name)
            || default_of(&field.ty).is_some()
        {
            continue;
        }
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                map.span.clone(),
                format!("{subject} does not supply `{name}`"),
            )
            .with_label(field.name.span.clone(), "the store declares it here")
            .with_help(
                "a `kv set` writes the whole value at its key, so it supplies every field of the store's `value_schema` (grammar 11.4)",
            ),
        );
    }
}

/// A store write inside a fan-out takes an item-derived key, or is a keyed
/// `kv` write (grammar 11.4, Decisions D67, D83).
pub(crate) fn check_map_writes(ctx: &mut Ctx) {
    for frame in reach::frames(ctx) {
        for node in &frame.flow.nodes {
            let NodeKind::Store { store, op, params } = &node.kind else {
                continue;
            };
            if !op.writes() {
                continue;
            }
            let Some(definition) = ctx.store(&store.value) else {
                continue;
            };
            // A `kv` write addresses a slot the author named, so concurrent
            // instances sharing a key are many writers on one record: a `set`
            // is a declared overwrite — the store-side counterpart of `reduce:
            // last_wins` — and a `merge`, `append` or `incr` is serialized
            // against every other write to its key, so each instance's change
            // lands (Decisions D67, D152).
            if definition.kind == StoreKind::Kv {
                continue;
            }
            let derived = params.key.as_ref().is_some_and(|key| {
                reach::is_item_derived(key.value.as_str(), None, &frame.derived)
            });
            if derived {
                continue;
            }
            let mut diagnostic = Diagnostic::error(
                DiagnosticCode::UnkeyedMapWrite,
                params
                    .key
                    .as_ref()
                    .map_or_else(|| node.span.clone(), |key| key.span.clone()),
                format!(
                    "{} dispatches `{}`, whose node `{}` writes `{}` with a key that is not item-derived",
                    frame.dispatcher,
                    frame.address,
                    text(&node.id),
                    store.value
                ),
            )
            .with_label(frame.span.clone(), "the dispatch is here");
            // The store node and the map are both innocent: the edit is at the
            // binding that fed this key a value every instance shares, which may
            // be several instantiations away from either (grammar 11.4's worked
            // example). Name it, for each `input.<field>` the key reads.
            let read = params
                .key
                .as_ref()
                .map(|key| reach::input_fields(key.value.as_str()))
                .unwrap_or_default();
            for field in read {
                if frame.derived.get(&field) == Some(&false)
                    && let Some(at) = frame.bound_at.get(&field)
                {
                    diagnostic = diagnostic.with_label(
                        at.clone(),
                        format!("`{field}` is bound here, to a value every instance shares"),
                    );
                }
            }
            ctx.push(
                diagnostic
                // Every repair named here has to type-check as a `key` as well
                // as satisfy this rule, which the bare index does not: grammar
                // 11.4 types every op's `key` as a CEL string and 4.1 types the
                // index as an integer. Naming `key: "execution.item_index"` as
                // a repair would send the author from this diagnostic straight
                // into a `type-mismatch`, so it is named as what it is instead
                // — item-derived, and a key only from inside a string-valued
                // expression, which is the spelling 11.4's fix list carries.
                .with_help(
                    "concurrent instances would address one key, so N items' content would land in one slot: key the write from the item — an `input.<field>` the dispatch binds from the item, or the whole item passed through — or use a `kv` store, whose write is a declared overwrite; `execution.item_index` is item-derived too, but a `key` is a string and the index is an integer, so it keys a write only from inside a string-valued expression (grammar 4.1, 11.4, Decisions D67, D83)",
                ),
            );
        }
    }
}

/// A trigger whose flow reaches a `session`-scoped store supplies a session
/// identity (grammar 11.3).
pub(crate) fn check_session_scope(ctx: &mut Ctx) {
    let ir = ctx.ir;
    let Some(triggers) = ir.triggers.as_ref() else {
        return;
    };
    for trigger in triggers.entries.values() {
        // A `manual` trigger's `session_key:` defaults to `"payload.session"`,
        // so it always has one (grammar 13.2, Decision D64).
        if matches!(trigger.kind, TriggerKind::Manual) || trigger.session_key.is_some() {
            continue;
        }
        let flow = trigger.flow.value.to_string();
        for store in reach::stores_of(ctx.ir, &flow) {
            let Some(definition) =
                ctx.ir
                    .definitions
                    .get(&store)
                    .and_then(|definition| match &definition.body {
                        crate::ir::definition::DefinitionBody::Store(store) => Some(store),
                        _ => None,
                    })
            else {
                continue;
            };
            if definition.scope != StoreScope::Session {
                continue;
            }
            let span = ctx.ir.definitions.get(&store).map_or_else(
                || trigger.span.clone(),
                |definition| definition.span.clone(),
            );
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingSessionKey,
                    trigger.span.clone(),
                    format!(
                        "the trigger `{}` declares no `session_key:`, and `{flow}` reaches the session-scoped store `{store}`",
                        trigger.name.value
                    ),
                )
                .with_label(span, "the store is declared here")
                .with_help(
                    "a session-scoped store keys off the session identity the trigger supplies: declare `session_key: <CEL over payload>` on the trigger, or give the store another `scope:` (grammar 11.3)",
                ),
            );
            break;
        }
    }
}

/// A `vector` store's `embed:` names the connection that computes its vectors,
/// and that connection must be able to (grammar 11.2, Decision D116).
///
/// The capability check itself is [`providers`](super::providers)'s, which owns
/// the table; this is the site grammar 11.2 runs it at.
pub(crate) fn check_embeddings(ctx: &mut Ctx) {
    let ir = ctx.ir;
    for (address, definition) in &ir.definitions {
        let crate::ir::definition::DefinitionBody::Store(store) = &definition.body else {
            continue;
        };
        let Some(embed) = &store.embed else {
            continue;
        };
        super::providers::require_embeddings(ctx, &embed.provider, address);
    }
}
