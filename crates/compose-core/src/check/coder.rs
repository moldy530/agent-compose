//! What a `coder:` node's block says that one file cannot decide (grammar 8.9,
//! Decisions D136–D143, PRD resolved q57, q58).
//!
//! The parser owns the block's *shape* — a harness that is not one of the four
//! names, a missing `workspace:`, an `access:` outside the three presets, a
//! duplicate `allow_tools:` entry — because each of those is one value against a
//! constant. Four rules are left, and each needs something the parser does not
//! have:
//!
//! * **the harness ships in this release.** `deepagents` and `native` are
//!   grammar (they are members of the enum, and a composition written against
//!   one is a composition this grammar can express), and what they lack is a
//!   *driver*. The refusal therefore names the v1 scope rather than the
//!   spelling, which is why it is its own code rather than an
//!   `unknown-variant`.
//! * **the model is a direct binding.** A `model.*` may be a failover route
//!   (grammar 12.2), and a *route* does not reach inside a harness run. PRD
//!   resolved q58 ruling a amends what q57 ruling d stopped here: the
//!   connection facts — `base_url:`, the credential, `headers:` — now cross
//!   (see the bullet below), and what still stops at this boundary is the
//!   failover ladder itself, q53's mechanism ladder, and the rest of the
//!   connection's *behavior*, because the harness owns its client and its
//!   internal retries and our `retry:` wraps whole runs. So a ladder here is a
//!   declared policy with nothing to apply it, and taking its first member
//!   silently is the one answer this compiler will not give. Deciding it needs
//!   the *definition* the address names, which is a whole-composition fact.
//! * **the provider connection has somewhere to land, and lands in one place.**
//!   PRD resolved q58 makes the resolved provider's `base_url:`, credential and
//!   `headers:` cross the boundary, mapped by each driver into the harness's own
//!   connection surface through the curated table in [`crate::harness`]. Three
//!   rules fall out of that, and each needs the whole composition. The table's
//!   rows are rows of **one wire** — `ANTHROPIC_BASE_URL` is where an Anthropic
//!   client is pointed — so a provider whose `kind:` the bound harness does not
//!   speak is an error naming both (ruling b, read at the pairing); a declared
//!   fact the harness has **no slot for** is an error naming the fact and the
//!   harness (ruling b); and a node `env:` entry spelling a variable the map
//!   would set — **or one the harness's own runtime reads for the same fact
//!   beside it**, which is the rest of the surface ruling a names — is an error
//!   naming both sources (ruling c). Deciding any of the three needs the
//!   `model.*` the node names, the `provider.*` behind it, and the table — none
//!   of which one file has.
//! * **the approval mode is inside the containment `access:` states.** PRD
//!   resolved q60 ruling a gives the Agent SDK's approval axis a key of its own,
//!   `permission_mode:`, because a bound must be readable off the node that
//!   holds it (resolved q54's principle, the same reason `allow_tools:` and
//!   `access:` are keys). Two rules come with it, and each needs the table in
//!   [`crate::harness`] rather than the one value the parser has: the bound
//!   harness must **have** such an axis at all — `codex` does not, its per-call
//!   approval tier being the app-server tier resolved q57 ruling c declines — and
//!   the mode must be one the node's own `access:` level **admits**, which is the
//!   widening bound.
//! * **the harness config is checked in three tiers.** Decision D140 holds
//!   `settings:` on resolved q30's terms: the keys the curated table knows are
//!   checked strictly, and everything else is a warning naming what could not be
//!   verified and travels to the SDK unchanged — except the keys the *adapter*
//!   owns, which are **refused** (PRD resolved q60 ruling b, amending the
//!   warn-and-silently-drop q57 shipped). Both tiers are per harness, so the
//!   check needs the `harness:` value beside the key — which the parser has, but
//!   the *tables* belong beside the refusals they produce rather than scattered
//!   through the reader, and the reserved one now lives in [`crate::harness`]
//!   with the connection table it copies.
//!
//! **What is deliberately not checked here** is the enforcement asymmetry of
//! ruling c. `allow_tools:` on a `codex` node is not a mistake — the list is
//! what the harness is offered, and the sandbox is what bounds it — so the
//! compiler states the asymmetry where an author meets it (grammar 8.9, the
//! `plan` report, and the graph document's `tools_enforced`) rather than warning
//! about a composition that is doing exactly what its author wrote.

use crate::ast::common::{Cel, Literal};
use crate::ast::flow::{Harness, PermissionMode, WorkspaceAccess};
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::harness::{
    Answered, ConnectionFact, ReservedHit, ReservedSpelling, Slot, admitted, declared,
    harnesses_with_a_permission_axis, has_permission_axis, kinds_of, levels_admitting,
    permission_level, provider_of, reserved_reached, slot_of, speaks, variables_read,
    variables_set,
};
use crate::ir::definition::{DefinitionBody, Model, Provider};
use crate::ir::flow::{Coder, Node};
use crate::parse::reader::{article, list, suggest};

use super::graph::Graph;
use super::reach;
use super::{Ctx, FlowCx};

/// What one curated `settings:` key may hold (Decision D140's first tier).
enum Shape {
    /// A boolean.
    Flag,
    /// An integer inside a closed range, inclusive at both ends.
    Integer(i64, i64),
    /// A number strictly above zero — a budget, which a zero would silence.
    Positive,
    /// One of a closed set of words.
    Enum(&'static [&'static str]),
    /// An array of non-empty strings.
    Names,
}

impl Shape {
    /// How a diagnostic names what this shape takes.
    const fn expectation(&self) -> &'static str {
        match self {
            Self::Flag => "a boolean",
            Self::Integer(..) => "an integer",
            Self::Positive => "a number above zero",
            Self::Enum(_) => "one of a closed set of words",
            Self::Names => "an array of non-empty strings",
        }
    }
}

/// The `cc` table: what the Claude Agent SDK's options this release maps by name
/// will take (grammar 8.9, Decision D140).
///
/// Four keys, and the choice of which four is the two-tier bargain working: each
/// is an option with a *meaning* a composition can get wrong in a way a run
/// discovers late — a turn bound of zero, a budget written as a string — and
/// everything the vendor ships beside them travels unchecked under a warning
/// rather than waiting for a compiler release.
///
/// The thinking budget is **not** here on purpose: it is the `anthropic`
/// plugin's `thinking:` key on the node's `model.*`, which the adapter maps down
/// (Decision D141). One option with two places to write it is the collision this
/// table exists to avoid.
const CC_SETTINGS: &[(&str, Shape)] = &[
    ("max_turns", Shape::Integer(1, 1000)),
    ("max_budget_usd", Shape::Positive),
    ("forward_subagent_text", Shape::Flag),
    ("disallowed_tools", Shape::Names),
];

/// …and the `codex` table, over the Codex SDK's own thread options.
const CODEX_SETTINGS: &[(&str, Shape)] = &[
    ("network_access", Shape::Flag),
    ("web_search", Shape::Enum(&["disabled", "cached", "live"])),
    ("skip_git_repo_check", Shape::Flag),
];

/// One harness's curated table.
const fn table(harness: Harness) -> &'static [(&'static str, Shape)] {
    match harness {
        Harness::Cc => CC_SETTINGS,
        Harness::Codex => CODEX_SETTINGS,
        // A reserved harness is refused before a settings key is read, so its
        // table is empty rather than invented: a warning naming keys nobody can
        // use would be a second complaint about a node that is already refused.
        Harness::DeepAgents | Harness::Native => &[],
    }
}

/// Check one `coder:` node.
pub(crate) fn coder_node(ctx: &mut Ctx<'_>, cx: &FlowCx<'_>, node: &Node, coder: &Coder) {
    let subject = format!("`{}` node `{}`", cx.address, node.id.value);
    // Before the harness, because it is about neither: `workspace:` is an
    // expression over this node's own input scope, and whether it reads what it
    // has to is a question about the flow rather than about the SDK behind the
    // node (Decision D147, PRD resolved q61 ruling a).
    workspace(ctx, cx, &subject, coder);
    if !harness_ships(ctx, &subject, coder) {
        return;
    }
    model_is_direct(ctx, &subject, coder);
    connection(ctx, &subject, coder);
    permission_mode(ctx, &subject, coder);
    settings(ctx, &subject, coder);
}

/// The directory one dispatch of this run works inside (PRD resolved q61
/// ruling a, Decision D147).
///
/// The key is a **runtime binding** since resolved q61: an expression the
/// node's own input scope is evaluated against, which is grammar 4.1's
/// "node `input:` bindings" row read for a key that is not an input binding.
/// What is left to check is what is left to check of any expression — its roots,
/// its constructs, and that it evaluates to a **string**, because a path is one
/// — and that is the whole of it: `validate` no longer inspects the path's
/// text, which is the trade every runtime binding makes (grammar 4.1).
///
/// `workspace: fresh` is checked by nothing here. It is a word rather than an
/// expression, and the directory it names is the runtime's to provision
/// (ruling c).
fn workspace(ctx: &mut Ctx<'_>, cx: &FlowCx<'_>, subject: &str, coder: &Coder) {
    let Some(expression) = coder.workspace.value.expression() else {
        return;
    };
    // The one spelling every composition written before resolved q61 has, and
    // the one the CEL front-end could only report as a syntax error: an
    // env-ref **value** — `workspace: ${REPO_ROOT}` — is not an expression, it
    // is the path that expression would have to produce. Answered with the
    // repair rather than with a parse error naming a character, because a
    // message an author cannot act on is the failure mode resolved q22 is
    // about (PRD G3).
    if let [name] = expression.references.as_slice()
        && expression.as_str() == format!("${{{name}}}")
    {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidExpression,
                coder.workspace.span.clone(),
                format!(
                    "`workspace: ${{{name}}}` of {subject} is a path where an expression \
                     is written"
                ),
            )
            .with_help(format!(
                "`workspace:` is evaluated in this node's input scope at each dispatch, so a \
                 path has to be written as one: `workspace: \"'${{{name}}}'\"` is that same \
                 directory as a CEL string, and the reference still resolves from the \
                 environment. What the expression buys is the dispatch — \
                 `workspace: \"'${{{name}}}/' + input.branch\"` gives a map item its own \
                 checkout, and `workspace: fresh` takes one the runtime provisions per \
                 dispatch (grammar 4.1, 8.9, Decision D147, PRD resolved q61 ruling a)"
            )),
        );
        return;
    }
    let written = Spanned::new(Cel::new(expression.as_str()), coder.workspace.span.clone());
    let scope = super::expr::flow_scope(ctx, cx, "a `coder:` node's `workspace:`");
    let analysis = super::expr::analyze(ctx, &written, &scope);
    super::expr::expect(
        ctx,
        &written,
        &analysis,
        &Type::String,
        &format!("`workspace:` of {subject}"),
    );
}

/// The approval mode the run's loop works under (PRD resolved q60 ruling a,
/// Decision D146).
///
/// Two refusals, and the order is the order the questions come in: a harness
/// that has no approval axis has no admitted set to compare against, so the
/// pairing is reported once and the walk stops. A node that states no mode at
/// all is checked by nothing here — its mode is the one its `access:` level
/// derives, which is a value this compiler chose and not a claim to verify.
fn permission_mode(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) {
    let Some(stated) = &coder.permission_mode else {
        return;
    };
    let harness = coder.harness.value;
    if !has_permission_axis(harness) {
        no_permission_axis(ctx, subject, coder, stated);
        return;
    }
    // The node's own level, with grammar 8.9's default materialized: an omitted
    // `access:` is `workspace_write`, and the bound is read off the level the
    // run really has rather than off the key the author happened to write.
    let access = coder.access.unwrap_or(WorkspaceAccess::WorkspaceWrite);
    if !admitted(harness, access).contains(&stated.value) {
        widens(ctx, subject, coder, stated, access);
    }
}

/// Ruling a: the bound harness has no approval axis for a mode to select.
///
/// Anchored at the **key**, unlike [`wrong_wire`]: nothing else in the block is
/// wrong and nothing else has to change for the composition to build — the key
/// is the line to delete or the harness is the line to change, and the message
/// says which of the two each repair is.
fn no_permission_axis(
    ctx: &mut Ctx<'_>,
    subject: &str,
    coder: &Coder,
    stated: &Spanned<PermissionMode>,
) {
    let harness = coder.harness.value.as_str();
    let carries: Vec<&str> = harnesses_with_a_permission_axis()
        .into_iter()
        .map(Harness::as_str)
        .collect();
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnsupportedPermissionMode,
            stated.span.clone(),
            format!(
                "{subject} binds `harness: {harness}`, which has no permission mode for \
                 `{}` to select",
                stated.value.as_str()
            ),
        )
        .with_label(coder.harness.span.clone(), "the harness is bound here")
        .with_help(format!(
            "the two harnesses are not symmetrical here and this document says so rather than \
             implying they are: `{harness}` contains a run with a sandbox preset, and its \
             per-call approval axis belongs to an app server this release does not adopt — so \
             there is nothing for a mode to choose and a faked one would be a bound nothing \
             holds. {} carries the axis. Drop `permission_mode:` and let `access:` state the \
             containment, which is the whole of what this harness holds a run to — or run the \
             node under {} (grammar 8.9, Decision D146, PRD resolved q57 ruling c, q60 ruling a)",
            list(&carries),
            list(&carries)
        )),
    );
}

/// Ruling a: the mode would widen what the node's `access:` states.
///
/// **The invariant of the whole key**, so the message carries all three facts a
/// repair needs: the mode, the level it is written under, and what that level
/// admits. Anchored at the mode for [`no_permission_axis`]'s reason — both
/// values are legal on their own, and what this composition cannot have is the
/// pair, which is the shape [`wrong_wire`] reports one construct over.
fn widens(
    ctx: &mut Ctx<'_>,
    subject: &str,
    coder: &Coder,
    stated: &Spanned<PermissionMode>,
    access: WorkspaceAccess,
) {
    let harness = coder.harness.value.as_str();
    let admits: Vec<&str> = admitted(coder.harness.value, access)
        .iter()
        .map(|mode| mode.as_str())
        .collect();
    let derived = permission_level(coder.harness.value, access)
        .map(|level| level.derived.as_str())
        .unwrap_or_default();
    // The level is named in the message rather than labelled in the source,
    // because a node that omits `access:` is under grammar 8.9's default and
    // there is no line to point at — and a diagnostic that labelled the key only
    // when it happened to be written would read as two different rules.
    let diagnostic = Diagnostic::error(
        DiagnosticCode::WideningPermissionMode,
        stated.span.clone(),
        format!(
            "`permission_mode: {}` of {subject} is outside what `access: {}` admits",
            stated.value.as_str(),
            access.as_str()
        ),
    )
    .with_label(
        coder.harness.span.clone(),
        format!("`harness: {harness}` is bound here"),
    );
    ctx.push(diagnostic.with_help(format!(
        "`access:` says how far a run may reach and `permission_mode:` says how its harness \
         approves a call inside that reach, so a mode may never grant an operation the level \
         would refuse: `{mode}` {decides}, and `access: {level}` {holds}. `access: {level}` \
         admits {admits} and derives `{derived}` when the key is absent. Either name one of \
         those, or {repair} (grammar 8.9, Decision D146, PRD resolved q60 ruling a)",
        mode = stated.value.as_str(),
        decides = stated.value.decides(),
        level = access.as_str(),
        holds = level_states(access),
        admits = list(&admits),
        repair = elsewhere(coder.harness.value, stated.value, access),
    )));
}

/// The second half of a widening repair: the `access:` level the refused mode
/// **does** belong under, read off the same table the refusal came from.
///
/// The levels are not a chain (Decision D146), so this cannot be "raise
/// `access:`" spelled once. `plan` is admitted under `read_only` and under
/// `full_access` and refused under the `workspace_write` between them — so the
/// most natural planning node, `permission_mode: plan` with no `access:` written
/// and grammar 8.9's default underneath it, has `read_only` as its repair and
/// would be sent to `full_access` by a message that only knew how to widen.
/// Which is the one repair this whole entry exists to discourage, handed out by
/// the diagnostic that exists to discourage it (PRD G3, resolved q22).
///
/// So the admitting levels are split at the node's own and each side is named
/// for what it is. Both sides can be occupied at once, and the narrower one is
/// read first.
fn elsewhere(harness: Harness, mode: PermissionMode, access: WorkspaceAccess) -> String {
    let admitting = levels_admitting(harness, mode);
    let named = |levels: &[WorkspaceAccess]| {
        levels
            .iter()
            .map(|level| {
                format!(
                    "`access: {}`, which {}",
                    level.as_str(),
                    level_states(*level)
                )
            })
            .collect::<Vec<_>>()
            .join("; ")
    };
    let narrower: Vec<WorkspaceAccess> = admitting
        .iter()
        .copied()
        .filter(|level| *level < access)
        .collect();
    let wider: Vec<WorkspaceAccess> = admitting
        .iter()
        .copied()
        .filter(|level| *level > access)
        .collect();
    let mode = mode.as_str();
    // A level is a containment statement a reader sees on the node, which is the
    // sentence every one of these repairs ends on — and the reason none of them
    // is "reach the mode through `settings:`".
    let seen = "a level is a containment statement a reader sees on the node rather than a mode \
                reaching around one";
    match (narrower.is_empty(), wider.is_empty()) {
        // Nothing this harness's table admits it under. Unreachable while
        // `full_access` admits every mode, and written out rather than left to
        // an `unwrap` because a table is a thing that gets edited.
        (true, true) => {
            format!(
                "drop `permission_mode:`: no `access:` level under `{harness}` admits `{mode}`",
                harness = harness.as_str()
            )
        }
        (true, false) => format!(
            "raise `access:` to a level that admits `{mode}` — {} — because {seen}",
            named(&wider)
        ),
        (false, true) => format!(
            "narrow `access:` to a level that admits `{mode}` — {} — because {seen}",
            named(&narrower)
        ),
        (false, false) => format!(
            "state the containment that admits `{mode}`, reading the narrower one first because \
             the levels are not a chain: {}; or {}. Either way {seen}",
            named(&narrower),
            named(&wider)
        ),
    }
}

/// What one `access:` level states, as grammar 8.9 words it — the half of the
/// widening message that says *why* the pair is refused.
const fn level_states(access: WorkspaceAccess) -> &'static str {
    match access {
        WorkspaceAccess::ReadOnly => "is read the workspace and write nothing",
        WorkspaceAccess::WorkspaceWrite => "bounds writes to the workspace",
        WorkspaceAccess::FullAccess => "asks for no containment at all",
    }
}

/// The provider connection the node's `model:` carries across (PRD resolved q58,
/// Decision D143).
///
/// The entry's compile errors, in one walk: the pairing first, then the facts
/// the resolved provider **declares**. A provider whose wire the harness speaks
/// and which declares nothing produces no diagnostic and injects nothing, which
/// is resolved q25's keyless posture surviving the crossing — the absence of a
/// key is the absence of a variable, never an empty one.
fn connection(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) {
    // `None` is a model that did not resolve, or a route — the resolver and
    // [`model_is_direct`] have each already said so, and a second complaint
    // about one mistake is noise.
    let Some((address, provider)) = provider_of(ctx.ir, coder) else {
        return;
    };
    // The wire first, and on its own: a slot is an endpoint and a credential of
    // *one* vendor's client, so a provider the harness's slots do not speak for
    // has no correct mapping to complain about fact by fact. Reporting the
    // pairing and stopping is the one diagnostic there is to give — and
    // [`declared`] must not be asked about a kind whose credential keys this
    // table does not spell.
    if !speaks(coder.harness.value, provider.kind) {
        wrong_wire(ctx, subject, coder, address, provider);
        return;
    }
    let held = declared(provider);
    for fact in &held {
        if slot_of(coder.harness.value, *fact).is_none() {
            no_slot(ctx, subject, coder, address, provider, *fact);
        }
    }
    // Ruling c, over the whole of the harness's connection surface rather than
    // over the three names the table happens to write. A fact has more than one
    // spelling on `cc` — `ANTHROPIC_AUTH_TOKEN` is a credential sent beside the
    // key, `CLAUDE_CODE_USE_BEDROCK` an endpoint chosen instead of the base URL
    // — and an `env:` entry naming one of those is the same two-sources mistake
    // wearing another name. One walk over the node's `env:` rather than one per
    // variable, so an entry earns at most one diagnostic.
    let mapped = variables_set(coder.harness.value, &held);
    let beside = variables_read(coder.harness.value, &held);
    for entry in &coder.env {
        let name = entry.name.value.as_str();
        if let Some((fact, variable)) = mapped.iter().find(|(_, held)| *held == name) {
            shadowed(
                ctx,
                subject,
                coder,
                address,
                provider,
                *fact,
                Spelling::Mapped(variable),
                &entry.name.span,
            );
        } else if let Some((fact, variable)) = beside.iter().find(|(_, held)| *held == name) {
            shadowed(
                ctx,
                subject,
                coder,
                address,
                provider,
                *fact,
                Spelling::Beside(variable),
                &entry.name.span,
            );
        }
    }
}

/// Ruling b, one level up: the provider's **wire** is not the harness's.
///
/// Anchored at the node's `model:` for [`no_slot`]'s reason, and it is the same
/// mistake read one step earlier: the provider is a correct definition and the
/// harness is a name the grammar has, and what this composition cannot have is
/// both at once. The label goes on the `provider.*` rather than on a key of it,
/// because no single key is wrong — `kind:` is what decides the wire, and it is
/// the definition's first line. [`provider_of`] found that definition, so the
/// lookup below answers; it is written as a lookup rather than an `expect` for
/// the reason every other check here is, which is that a checker reports.
///
/// The help has **two** shapes and they are written as two, because the repair
/// is not the same one. Where another harness speaks the kind there are two
/// moves and an author picks by intent — keep the harness and rebind the model,
/// or keep the connection and change the harness. Where none does — `bedrock`,
/// `vertex` and `azure_openai`, the three kinds no row carries — there is one
/// move, and a help that offered the second would be naming a node kind that
/// does not exist. `a_kind_no_harness_carries_is_told_there_is_one_repair`
/// covers the branch the shipped fixtures cannot reach.
fn wrong_wire(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder, address: &str, provider: &Provider) {
    let harness = coder.harness.value.as_str();
    let spoken: Vec<&str> = kinds_of(coder.harness.value)
        .iter()
        .map(|kind| kind.as_str())
        .collect();
    let carries = if spoken.is_empty() {
        "no provider connection at all".to_string()
    } else {
        format!("{} connections, and no other", list(&spoken))
    };
    // Where the other harnesses would take this kind, so the repair that keeps
    // the *connection* is a name rather than a search.
    let elsewhere: Vec<&str> = Harness::ALL
        .iter()
        .copied()
        .filter(|held| *held != coder.harness.value && speaks(*held, provider.kind))
        .map(Harness::as_str)
        .collect();
    // …and the repair, which is **one** repair where no harness speaks the kind
    // and two where one does. The two halves have to be written together: a help
    // that ended on "or run the node under the harness that speaks this one"
    // after saying no harness carries the connection would be sending an author
    // to a node kind that does not exist.
    let repair = if elsewhere.is_empty() {
        format!(
            "No harness this release lowers carries {} `{}` connection, so there is nowhere to \
             move this node to: bind its `model:` to a `provider.*` whose kind `{harness}` speaks \
             — providers are cheap — and leave `{address}` serving the agent nodes it already \
             serves",
            article(provider.kind.as_str()),
            provider.kind.as_str()
        )
    } else {
        format!(
            "{} does carry it, so there are two repairs and which is right depends on which half \
             was meant: bind this node's `model:` to a `provider.*` whose kind `{harness}` speaks \
             — providers are cheap — or run the node under {}",
            list(&elsewhere),
            list(&elsewhere)
        )
    };
    let mut diagnostic = Diagnostic::error(
        DiagnosticCode::UnsupportedProviderKind,
        coder.model.span.clone(),
        format!(
            "{subject} binds `harness: {harness}`, and `{address}` is `kind: {}`, whose \
             connection that harness cannot carry",
            provider.kind.as_str()
        ),
    );
    if let Some(definition) = ctx.ir.definitions.get(address) {
        diagnostic = diagnostic.with_label(
            definition.address.span.clone(),
            format!(
                "`{address}` is defined here, as `kind: {}`",
                provider.kind.as_str()
            ),
        );
    }
    ctx.push(
        diagnostic
            .with_label(coder.harness.span.clone(), "the harness is bound here")
            .with_help(format!(
                "a coder node's `model:` carries its provider's connection into the run, and a \
                 slot is an endpoint and a credential on one wire: `harness: {harness}` carries \
                 {carries}, so this pairing would write one vendor's endpoint and key where \
                 another's are read. {repair} (grammar 8.9, 12.1, Decision D143, PRD resolved q58 \
                 ruling b)"
            )),
    );
}

/// Ruling b: a declared fact the bound harness has nowhere to put.
///
/// Anchored at the node's `model:`, which is [`model_is_direct`]'s position and
/// for its reason: the *pairing* is what is wrong. The provider is a correct
/// definition serving every agent that binds it, and the harness is a name the
/// grammar has — what this composition cannot have is both at once, and the
/// reference that brought them together is the line to change.
fn no_slot(
    ctx: &mut Ctx<'_>,
    subject: &str,
    coder: &Coder,
    address: &str,
    provider: &Provider,
    fact: ConnectionFact,
) {
    let harness = coder.harness.value.as_str();
    // What this harness *does* carry, and where — which is the half an author
    // repairs against. Naming the slots rather than only the keys is what tells
    // a reader whether the fact they are moving has somewhere else to go.
    let carried: Vec<String> = ConnectionFact::ALL
        .iter()
        .copied()
        .filter_map(|held| {
            slot_of(coder.harness.value, held)
                .map(|slot| format!("`{}:` as {}", held.as_str(), slot.describes()))
        })
        .collect();
    let carries = if carried.is_empty() {
        "no connection fact at all".to_string()
    } else {
        format!("{}, and no other", carried.join(", "))
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnsupportedConnectionFact,
            coder.model.span.clone(),
            format!(
                "{subject} binds `harness: {harness}`, and `{address}` declares `{}:`, which that \
                 harness has nowhere to carry",
                fact.as_str()
            ),
        )
        .with_label(
            fact_span(provider, fact),
            format!("`{}:` is declared here", fact.as_str()),
        )
        .with_label(coder.harness.span.clone(), "the harness is bound here")
        .with_help(format!(
            "a coder node's `model:` carries its provider's connection into the run, mapped by a \
             curated table per harness: `harness: {harness}` carries {carries}. A fact with no \
             slot is refused rather than dropped, because a key deciding {} must not go missing \
             quietly and be found on the first live call. Bind a `model.*` on a `provider.*` \
             declaring only what this harness carries — providers are cheap — or run the node \
             under a harness with a slot for it (grammar 8.9, 12.1, Decision D143, PRD resolved \
             q58 ruling b)",
            fact.decides()
        )),
    );
}

/// Which of a fact's spellings a node's `env:` entry wrote.
///
/// The mistake is one mistake — two sources for one fact — and it arrives two
/// ways, so the diagnostic says which. An author who wrote the mapped name can
/// see the collision in the composition; an author who wrote a sibling cannot,
/// because the name they chose appears nowhere in it, and a message that only
/// said "set twice" would be pointing at a line the map never writes.
enum Spelling<'a> {
    /// The variable the connection map **sets** for this fact.
    Mapped(&'a str),
    /// A variable the harness's own runtime **reads** for this fact beside the
    /// mapped one — another endpoint, or a second credential.
    Beside(&'a str),
}

impl Spelling<'_> {
    /// The name the `env:` entry spelled.
    const fn variable(&self) -> &str {
        match self {
            Self::Mapped(variable) | Self::Beside(variable) => variable,
        }
    }
}

/// Ruling c: one spelling per fact.
///
/// Anchored at the **`env:` entry**, unlike its sibling above: the connection is
/// not the mistake here — it is what the composition asked for by binding this
/// model — and the entry is the line that says the same thing twice.
#[allow(
    clippy::too_many_arguments,
    reason = "a diagnostic naming both sources needs both"
)]
fn shadowed(
    ctx: &mut Ctx<'_>,
    subject: &str,
    coder: &Coder,
    address: &str,
    provider: &Provider,
    fact: ConnectionFact,
    spelling: Spelling<'_>,
    at: &Span,
) {
    let harness = coder.harness.value.as_str();
    let variable = spelling.variable();
    // The name the map writes, which both messages name: for a mapped spelling
    // it is the entry itself, and for a sibling it is what the entry is a second
    // spelling *of*. A sibling only exists on a fact this harness carries as a
    // variable, so the lookup answers wherever one is reported.
    let slot = slot_of(coder.harness.value, fact)
        .and_then(Slot::variable)
        .unwrap_or(variable);
    let (message, help) = match spelling {
        Spelling::Mapped(_) => (
            format!(
                "`{variable}` is set twice for {subject}: by this `env:` entry, and by \
                 `{address}`'s `{}:`",
                fact.as_str()
            ),
            format!(
                "one spelling per fact: the provider's `{fact}:` already reaches this run, so \
                 there is nothing to shadow and no precedence rule to learn. Drop the `env:` \
                 entry — it is for what the *program* needs — and a node that really has to \
                 decide {decides} for itself binds a `model.*` on another `provider.*` (grammar \
                 8.9, 12.1, Decision D143, PRD resolved q58 ruling c)",
                fact = fact.as_str(),
                decides = fact.decides()
            ),
        ),
        Spelling::Beside(_) => (
            format!(
                "`{variable}` is a second spelling of `{}:` for {subject}: `harness: {harness}` \
                 reads it beside the `{slot}` this `env:` entry does not name, and `{address}` \
                 already declares the fact",
                fact.as_str()
            ),
            format!(
                "one spelling per fact, and a fact has more than one: `{variable}` is a name \
                 `harness: {harness}` reads for {decides}, so this entry decides it a second time \
                 beside the connection the node's `model:` already carries — the run would \
                 authenticate as somebody else, or leave for an endpoint nothing in this \
                 composition names, while the graph document and the journal's record of the run \
                 both still report `{slot}`. Drop \
                 the `env:` entry — it is for what the *program* needs — and a node that really \
                 has to decide {decides} for itself binds a `model.*` on another `provider.*` \
                 (grammar 8.9, 12.1, Decision D143, PRD resolved q58 rulings a and c)",
                decides = fact.decides()
            ),
        ),
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::ConflictingConnectionVariable,
            at.clone(),
            message,
        )
        .with_label(
            fact_span(provider, fact),
            format!(
                "`{}:` is declared here, and `harness: {harness}` carries it as `{slot}`",
                fact.as_str()
            ),
        )
        .with_label(coder.model.span.clone(), "the connection is bound here")
        .with_help(help),
    );
}

/// Where one declared fact is written, for a diagnostic's label.
///
/// A `headers:` block has no span of its own in the artifact — it is a list of
/// entries — so the first entry's name is what a reader is pointed at, which is
/// the line the block opens on.
fn fact_span(provider: &Provider, fact: ConnectionFact) -> Span {
    let span = match fact {
        ConnectionFact::BaseUrl => provider.config.base_url.as_ref().map(|held| &held.span),
        ConnectionFact::Credential => provider.config.api_key.as_ref().map(|held| &held.span),
        ConnectionFact::Headers => provider
            .config
            .headers
            .first()
            .map(|entry| &entry.name.span),
    };
    span.expect("a fact is only reported where the provider declares it")
        .clone()
}

/// The harness has a driver in this release (PRD resolved q57).
///
/// Answers whether the rest of the block is worth checking: a reserved harness
/// has no curated settings table and no driver, so a second diagnostic about a
/// key of one would be noise on top of the refusal that matters.
fn harness_ships(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) -> bool {
    if coder.harness.value.ships_in_v1() {
        return true;
    }
    let shipping: Vec<&str> = Harness::ALL
        .iter()
        .filter(|harness| harness.ships_in_v1())
        .map(|harness| harness.as_str())
        .collect();
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnsupportedHarness,
            coder.harness.span.clone(),
            format!(
                "`harness: {}` is reserved: this release lowers {} and no other",
                coder.harness.value.as_str(),
                list(&shipping)
            ),
        )
        .with_label(
            coder.span.clone(),
            format!("the `coder` block of {subject}"),
        )
        .with_help(
            "`deepagents` and `native` are spelled by the grammar so that a composition written \
             against one is refused by name rather than by a suggestion list; what they lack is a \
             driver, and the set grows by a resolved question rather than by a release adding a \
             name (grammar 8.9, PRD resolved q57)",
        ),
    );
    false
}

/// A coder node's `model:` is a **direct** binding (Decision D141).
fn model_is_direct(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) {
    let address = coder.model.value.to_string();
    let Some(definition) = ctx.ir.definitions.get(&address) else {
        // An address that names nothing has already been reported by the
        // resolver, and reporting its *form* on top of that would be a second
        // complaint about one mistake.
        return;
    };
    let DefinitionBody::Model(Model::Route(route)) = &definition.body else {
        return;
    };
    let members = route.route.len();
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            coder.model.span.clone(),
            format!("`{address}` is a failover route, and {subject} runs a harness"),
        )
        .with_label(
            definition.address.span.clone(),
            format!("`{address}` is defined here, with {members} members"),
        )
        .with_help(
            "a failover ladder does not reach inside a harness run: the harness owns its client \
             and its own retries, and this compiler's `retry:` wraps whole runs. A route here \
             would be a failover policy with nothing to apply it to, so bind a direct `model.*` \
             — whose connection *does* cross (Decision D143) — and let the node's `retry:` be the \
             ladder (grammar 8.9, 12.2, PRD resolved q57 ruling d as q58 amends it)",
        ),
    );
}

/// The harness config, in Decision D140's tiers as PRD resolved q60 ruling b
/// hardens them.
///
/// Three answers now, and the order they are asked in is the order they matter:
/// a key the curated table knows is checked strictly; a key the **reserved**
/// table holds is refused; anything else is the warning resolved q30's second
/// tier exists for, and travels to the SDK unchanged.
///
/// The reserved question comes before the warning and after the curated table
/// because the middle answer is the one that changed: it was a warning and a
/// silent run-time drop, and a dropped key that would have changed what a run
/// may do is the class PRD G3 refuses to discover from a run's behaviour.
///
/// The reserved question is asked of **both** spellings a row has — the SDK's
/// name for the option and the grammar's name for the bound it states — because
/// the second is the one an author reaches for. `settings: { permission_mode: … }`
/// is the field report's own mistake written in the spelling this grammar
/// teaches, and keyed on `permissionMode` alone it fell through to the warning
/// and travelled to an SDK that has no such option
/// ([`crate::harness::reserved_reached`]).
fn settings(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) {
    let harness = coder.harness.value;
    let known = table(harness);
    for setting in &coder.settings {
        let key = setting.key.value.as_str();
        let Some((_, shape)) = known.iter().find(|(name, _)| *name == key) else {
            if let Some(hit) = reserved_reached(harness, key) {
                reserved_setting(ctx, subject, coder, &setting.key, hit);
                continue;
            }
            let names: Vec<&str> = known.iter().map(|(name, _)| *name).collect();
            ctx.push(
                Diagnostic::warning(
                    DiagnosticCode::UnknownHarnessSetting,
                    setting.key.span.clone(),
                    format!(
                        "`{key}` is not a `harness: {}` setting this release can check, so its \
                         value is unverified",
                        harness.as_str()
                    ),
                )
                .with_label(coder.harness.span.clone(), "the harness is bound here")
                .with_optional_help(
                    suggest(key, &names)
                        .map(|name| format!("did you mean `{name}`?"))
                        .or_else(|| {
                            Some(format!(
                                "it travels to the SDK unchanged, which is what keeps a harness \
                                 option usable the day the vendor ships it; the keys this release \
                                 checks are {} (grammar 8.9, Decision D140)",
                                list(&names)
                            ))
                        }),
                ),
            );
            continue;
        };
        check_shape(ctx, subject, harness, &setting.key, &setting.value, shape);
    }
}

/// Ruling b: a `settings:` key spelling an option the generated adapter owns.
///
/// **An error, and the repair is the point.** PRD resolved q57 shipped this as a
/// warning beside a silent run-time drop, and the field report resolved q60
/// comes from is what that cost: an author who needed a permission mode the
/// `access:` enum could not reach found `settings:`, was told the value was
/// unverified, watched the run ignore it, and had no way to tell a key that
/// travelled from a key that was dropped. So the key is refused where it is
/// written, and the message names three things — the option, the bound it would
/// reach around, and the key that states that bound where one does.
///
/// Anchored at the **key** rather than the value, because the value is not the
/// mistake: no value of this key reaches the SDK.
///
/// # Two spellings, one refusal
///
/// A row is reachable under the SDK's name for the option and under the
/// grammar's name for the bound it states, and both are refused
/// ([`crate::harness::reserved_reached`]). The *repair* is the row's, either
/// way — it is the same bound and the same key answers it. What the two say
/// differently is the **first sentence and the last**: an SDK spelling is an
/// option the adapter owns and would have been dropped, while a grammar
/// spelling is a key of this block written one level too low, which no option
/// answers to at all — it would have travelled to the harness and done nothing.
/// Saying "silently removed" about a key that was never removed would send an
/// author looking for a subtraction that is not there.
fn reserved_setting(
    ctx: &mut Ctx<'_>,
    subject: &str,
    coder: &Coder,
    key: &Spanned<String>,
    hit: ReservedHit,
) {
    let harness = coder.harness.value.as_str();
    let held = key.value.as_str();
    let (owned, repair) = match hit.held.answered {
        Answered::By(first_class) => (
            format!(
                "`{held}` of {subject} is a `harness: {harness}` option the generated adapter \
                 owns: it is what `{first_class}:` states"
            ),
            format!(
                "write `{first_class}:` on the node instead. A bound has to be readable off the \
                 construct that holds it, which is why `{first_class}:` is a key and not a \
                 setting, and two spellings of one option would leave a reader and `validate` \
                 disagreeing about which wins"
            ),
        ),
        Answered::Already(first_class) => (
            format!(
                "`{held}` of {subject} is a `harness: {harness}` option the generated adapter \
                 owns: it is what `{first_class}:` states"
            ),
            format!(
                "take the setting off: `{first_class}:` is required on every `coder:` block, so \
                 it is already on the node and already states this. A bound has to be readable \
                 off the construct that holds it, which is why `{first_class}:` is a key and not \
                 a setting, and two spellings of one option would leave a reader and `validate` \
                 disagreeing about which wins"
            ),
        ),
        Answered::Through(first_class, site) => (
            format!(
                "`{held}` of {subject} is a `harness: {harness}` option the generated adapter \
                 owns: it is what `{first_class}:` resolves to"
            ),
            format!(
                "`{first_class}:` is already on the node and is an address, so the value is \
                 written where it points, not here: {site}"
            ),
        ),
        Answered::Nothing(why) => (
            format!(
                "`{held}` of {subject} is a `harness: {harness}` option the generated adapter \
                 owns, and no key of this block states it"
            ),
            format!(
                "there is no key to write instead: {why}. Take the setting off — what \
                 `settings:` buys is the vendor's *other* options, never the ones this node \
                 already states"
            ),
        ),
    };
    let (message, why_refused) = match hit.spelling {
        ReservedSpelling::Option => (
            owned,
            "Refused rather than dropped, because a key silently removed leaves a run doing \
             something no line of the composition says"
                .to_string(),
        ),
        ReservedSpelling::Key => (
            format!(
                "`{held}` of {subject} is a key of the `coder:` block written inside `settings:`, \
                 where it states nothing: the bound it names is one the generated adapter owns"
            ),
            format!(
                "Refused rather than passed on, because `harness: {harness}` declares no option \
                 of this name: the key would travel to the SDK unchanged, do nothing at all, and \
                 leave a node reading as though the bound were stated"
            ),
        ),
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::ReservedHarnessSetting,
            key.span.clone(),
            message,
        )
        .with_label(coder.harness.span.clone(), "the harness is bound here")
        .with_help(format!(
            "{repair}. `settings:` is open so that a harness option the vendor ships tomorrow is \
             usable the day it ships — it is not a second way to state this construct's own \
             bounds, and a key here that reached one would be the only one no check covers. \
             {why_refused} (grammar 8.9, Decision D146, PRD resolved q60 ruling b)"
        )),
    );
}

/// One curated key's value, strictly.
fn check_shape(
    ctx: &mut Ctx<'_>,
    subject: &str,
    harness: Harness,
    key: &Spanned<String>,
    value: &Spanned<Literal>,
    shape: &Shape,
) {
    let refuse = |ctx: &mut Ctx<'_>, code: DiagnosticCode, message: String, help: String| {
        ctx.push(
            Diagnostic::error(code, value.span.clone(), message)
                .with_label(key.span.clone(), "the setting is declared here")
                .with_help(help),
        );
    };
    match (shape, &value.value) {
        (Shape::Flag, Literal::Bool(_))
        | (Shape::Positive, Literal::Int(_) | Literal::Float(_)) => {}
        (Shape::Integer(low, high), Literal::Int(held)) => {
            if !(*low..=*high).contains(held) {
                refuse(
                    ctx,
                    DiagnosticCode::ValueOutOfRange,
                    format!(
                        "`{}` of {subject} is {held}, outside {low}..={high}",
                        key.value
                    ),
                    format!(
                        "`harness: {}` takes `{}` in that range (grammar 8.9, Decision D140)",
                        harness.as_str(),
                        key.value
                    ),
                );
            }
        }
        (Shape::Enum(words), Literal::String(held)) => {
            if !words.contains(&held.as_str()) {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        value.span.clone(),
                        format!("`{held}` is not a `{}` of {subject}", key.value),
                    )
                    .with_label(key.span.clone(), "the setting is declared here")
                    .with_optional_help(
                        suggest(held, words)
                            .map(|name| format!("did you mean `{name}`?"))
                            .or_else(|| Some(format!("`{}` takes {}", key.value, list(*words)))),
                    ),
                );
            }
        }
        (Shape::Names, Literal::Sequence(items)) => {
            for item in items {
                let empty = match &item.value {
                    Literal::String(held) => held.is_empty(),
                    _ => true,
                };
                if empty {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::WrongType,
                            item.span.clone(),
                            format!("every entry of `{}` of {subject} is a tool name", key.value),
                        )
                        .with_label(key.span.clone(), "the setting is declared here")
                        .with_help("a tool name is a non-empty string (grammar 8.9)"),
                    );
                }
            }
        }
        _ => {
            refuse(
                ctx,
                DiagnosticCode::WrongType,
                format!(
                    "`{}` of {subject} takes {}, found {}",
                    key.value,
                    shape.expectation(),
                    value.value.description()
                ),
                format!(
                    "`harness: {}` maps this key onto an SDK option of that type (grammar 8.9, \
                     Decision D140)",
                    harness.as_str()
                ),
            );
        }
    }
    // A budget of zero is a run that can make no call at all, which is a
    // configuration nobody writes on purpose: the key exists to bound spend, and
    // `0` silences the node rather than bounding it.
    if matches!(shape, Shape::Positive) {
        let held = match &value.value {
            Literal::Int(held) => Some(*held as f64),
            Literal::Float(held) => Some(*held),
            _ => None,
        };
        if let Some(held) = held
            && held <= 0.0
        {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::ValueOutOfRange,
                    value.span.clone(),
                    format!("`{}` of {subject} is not above zero", key.value),
                )
                .with_label(key.span.clone(), "the setting is declared here")
                .with_help(
                    "a budget of zero stops the run before its first call rather than bounding \
                     what it may spend; omit the key to leave the run unbounded (grammar 8.9)",
                ),
            );
        }
    }
}

/// The race PRD resolved q61 ruling b makes unwritable: a coder node a `map`
/// dispatches concurrently whose `workspace:` names one directory for every
/// item.
///
/// A coder node lives inside a `flow.*` (grammar 8.9), so it is inside a
/// fan-out exactly when that flow is a dispatch target — directly or through
/// the `flow:` nodes below one — which is the relation [`reach::frames`]
/// already computes for grammar 11.4's store keys. What a frame carries is what
/// this rule needs twice over: which of the instance's input fields are
/// **item-derived** at that site, and how many dispatches of it may be in
/// flight at once.
///
/// The test is [`reach::is_item_derived`], the same predicate a store key is
/// held to (Decision D83): an expression reads the per-dispatch scope when it
/// reads the dispatch's own `execution.item_index`, or an `input.<field>` the
/// map bound from the item. Everything else — a literal path, an `${ENV}`
/// reference, a `state` channel every instance shares — is one directory for
/// the whole fan-out, which is the field report this ruling comes from: the
/// dispatches clobber each other's checkout and `max_concurrency` was a lie.
///
/// Two ways out of the rule, and both are statements rather than escapes.
/// `max_concurrency: 1` says the runs are serial, and the refusal lifts because
/// there is no longer a race to have. `workspace: fresh` satisfies it by
/// construction — the runtime provisions a directory per dispatch — and never
/// reaches this check at all, having no expression to read.
///
/// **The boundary is [`reach::frames`]'s own**: derivation travels inward
/// through `flow:` nodes and stops at an agent's `flow.*` tool, because a model
/// decides whether and when to call one and no static rule can put that call
/// inside a dispatch. A coder node reached only that way is outside this rule,
/// exactly as a store node reached only that way is outside grammar 11.4's.
pub(crate) fn dispatched_workspaces(ctx: &mut Ctx<'_>) {
    let mut reported = Vec::new();
    for frame in reach::frames(ctx) {
        // A serial dispatch is a statement the author made, and it is the
        // second repair this rule's message offers: one run at a time is one
        // run in the directory at a time (grammar 8.6 rule 1).
        if frame.concurrency <= 1 {
            continue;
        }
        for node in &frame.flow.nodes {
            let crate::ir::flow::NodeKind::Coder { coder } = &node.kind else {
                continue;
            };
            let Some(expression) = coder.workspace.value.expression() else {
                continue;
            };
            if reach::is_item_derived(expression.as_str(), None, &frame.derived) {
                continue;
            }
            reported.push((
                format!("`{}` node `{}`", frame.address, node.id.value),
                coder.workspace.span.clone(),
                frame.dispatcher.clone(),
                frame.span.clone(),
                frame.concurrency,
                expression.as_str().to_string(),
            ));
        }
    }
    for (subject, at, dispatcher, dispatched_at, concurrency, expression) in reported {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::SharedWorkspace,
                at,
                format!(
                    "{subject} is dispatched by {dispatcher} with `max_concurrency: \
                     {concurrency}`, and its `workspace:` is one directory for every dispatch"
                ),
            )
            .with_label(dispatched_at, "the fan-out is issued here")
            .with_help(format!(
                "a harness run is contained by its `workspace:`, so {concurrency} dispatches \
                 sharing `{expression}` are {concurrency} coding agents editing one checkout \
                 and overwriting each other's work. Two repairs, and each says something \
                 different about the graph: bind the item's own path — carry it on the \
                 dispatch (`input: {{ worktree: \"item.worktree\" }}`) and read it here \
                 (`workspace: \"input.worktree\"`), or write `workspace: fresh` for a directory \
                 the runtime provisions per dispatch — or declare `max_concurrency: 1` on the \
                 map, which says these runs are serial. `workspace:` is evaluated in this \
                 node's input scope at every dispatch precisely so the first repair is \
                 writable (grammar 8.6, 8.9, Decision D147, PRD resolved q61 ruling b)"
            )),
        );
    }
}

/// The same collision one construct along: two coder nodes that may be in
/// flight at once and name one directory (PRD resolved q61 ruling b).
///
/// A **warning**, and the entry says why rather than pretending otherwise:
/// what decides this is whether the two expressions resolve to one directory,
/// and a value bearing an `${ENV}` reference resolves at launch. Two values
/// that differ statically may be one path on the machine that runs them, and
/// this compiler cannot see that — so what it can see, two values that are
/// **written** identically, is reported as the thing it is: a collision worth a
/// second look rather than a fact worth refusing a build over.
///
/// `workspace: fresh` is never one of these, and not by exemption: the
/// directory it names is the §9.4 instance path, and two nodes of one flow have
/// two node ids, so two `fresh` nodes are two directories by construction
/// (ruling c).
pub(crate) fn concurrent_workspaces(ctx: &mut Ctx<'_>, cx: &FlowCx<'_>, graph: &Graph<'_>) {
    let coders: Vec<usize> = (0..graph.nodes().len())
        .filter(|at| {
            matches!(&graph.node(*at).kind,
                crate::ir::flow::NodeKind::Coder { coder }
                    if coder.workspace.value.expression().is_some())
        })
        .collect();
    let mut reported = Vec::new();
    for (one, other) in super::convergence::concurrent_among(ctx, graph, &coders) {
        let (Some(first), Some(second)) = (coder_of(graph.node(one)), coder_of(graph.node(other)))
        else {
            continue;
        };
        if first.workspace.value != second.workspace.value {
            continue;
        }
        reported.push((
            graph.id(one).to_string(),
            graph.id(other).to_string(),
            first.workspace.span.clone(),
            second.workspace.span.clone(),
            first.workspace.value.as_str().to_string(),
        ));
    }
    for (one, other, first, second, expression) in reported {
        ctx.push(
            Diagnostic::warning(
                DiagnosticCode::SharedWorkspace,
                second,
                format!(
                    "nodes `{one}` and `{other}` of `{}` can run concurrently and their \
                     `workspace:` is written the same way",
                    cx.address
                ),
            )
            .with_label(first, "the other run is contained here")
            .with_help(format!(
                "two edges of one fork that are not provably exclusive can both fire, so the \
                 branches they start are concurrent — and two harness runs inside one directory \
                 edit each other's files. `{expression}` is one expression over one scope, so \
                 both runs resolve it to one path. Give one of them a directory of its own, or \
                 write `workspace: fresh` on each for a directory the runtime provisions per \
                 dispatch. A **warning** rather than a refusal because equality here is a \
                 launch fact: a value bearing an `${{ENV}}` reference resolves on the machine \
                 that runs it, so two values this compiler reads as different may still be one \
                 directory, and two it reads as the same is the half it can see (grammar 7.6.1, \
                 8.9, Decision D147, PRD resolved q61 ruling b)"
            )),
        );
    }
}

/// The `coder:` block of a node, where the node is one.
fn coder_of(node: &Node) -> Option<&Coder> {
    match &node.kind {
        crate::ir::flow::NodeKind::Coder { coder } => Some(coder),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticCode, Harness, table};
    use crate::harness::reserved_of;

    /// **The refusal reads the table the adapter's own list is a copy of**
    /// (grammar 8.9, Decision D146, PRD resolved q60 ruling b).
    ///
    /// The reserved set moved from the emitted driver — where `validate` used to
    /// parse it back out of the TypeScript so a *warning* could say which half
    /// of Decision D140 a key landed in — to a compiler table beside the
    /// connection table it copies, because an error has to name a repair and a
    /// list of strings carries none. What can still go wrong is the same thing:
    /// a row that lost an option lets a `settings:` key through with the
    /// warning's "it travels to the SDK unchanged", which is the one sentence
    /// that is never true of a name the driver subtracts. Two of the options
    /// that reach furthest are named here per harness, plus the reserved
    /// harnesses' empty rows.
    #[test]
    fn a_refusal_reads_the_reserved_table_the_adapter_copies() {
        let cc = reserved_of(Harness::Cc);
        for option in ["cwd", "extraArgs", "settings", "resume", "permissionMode"] {
            assert!(
                cc.iter().any(|held| held.option == option),
                "`cc`'s reserved row is read without `{option}`, so a `settings:` key spelling it \
                 is warned about as one that travels to the SDK and then dropped by the driver"
            );
        }
        let codex = reserved_of(Harness::Codex);
        for option in ["sandboxMode", "workingDirectory", "approvalPolicy"] {
            assert!(
                codex.iter().any(|held| held.option == option),
                "`codex`'s reserved row is read without `{option}`"
            );
        }
        for held in [Harness::DeepAgents, Harness::Native] {
            assert!(
                reserved_of(held).is_empty(),
                "a reserved harness has no SDK whose options an adapter could own"
            );
        }
    }

    /// **Every mode a level admits builds, and the derived one builds written
    /// out longhand** (grammar 8.9, Decision D146, PRD resolved q60 ruling a).
    ///
    /// The direction the negative corpus cannot reach, and the one an
    /// over-eager admissibility check gets wrong in a way nothing else notices:
    /// a composition that *should* build stops building, and the author's only
    /// clue is a diagnostic about a line that is correct. The key exists because
    /// three of the SDK's six modes were unreachable, so a table that admitted
    /// the derived mode alone would ship the same gap under a new name.
    ///
    /// Written out as the **whole** admissibility table rather than as a
    /// sample, because what is being asserted is which modes each level takes:
    /// every admitted pairing is checked clean, and every other pairing is
    /// checked refused with the code that says why. That second half is what
    /// makes this a statement about the bound rather than about three examples —
    /// the negative corpus pins one refusal's wording, and this pins that there
    /// are exactly seven of them: eighteen pairings, eleven admitted (one, four
    /// and six, level by level), and the rest refused.
    #[test]
    fn each_access_level_takes_every_mode_it_admits_and_no_other() {
        use crate::ast::flow::{PermissionMode, WorkspaceAccess};
        use crate::codegen::test_support::ir_of;
        use crate::harness::admitted;

        let composition = |access: &str, mode: &str| {
            format!(
                "version: \"0.1\"\n\
provider.p:\n  kind: anthropic\n  api_key: ${{K}}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
state:\n  summary: {{ type: string, default: \"\" }}\n\
flow.main:\n  outputs:\n    summary: {{ type: string }}\n  nodes:\n    build:\n      coder:\n        harness: cc\n        model: model.m\n        workspace: \"'/srv/checkout'\"\n        access: {access}\n        permission_mode: {mode}\n        prompt: Do the work.\n        output:\n          summary: {{ type: string }}\n      input: \"'go'\"\n  edges:\n    - {{ from: start, to: build }}\n    - {{ from: build, to: end }}\n"
            )
        };

        let mut refusals = 0;
        for access in WorkspaceAccess::ALL.iter().copied() {
            for mode in PermissionMode::ALL.iter().copied() {
                let held = crate::check(&ir_of(&composition(access.as_str(), mode.as_str())));
                if admitted(Harness::Cc, access).contains(&mode) {
                    assert!(
                        held.is_empty(),
                        "`access: {}` admits `{}` and a node writing it was refused: {held:#?}",
                        access.as_str(),
                        mode.as_str()
                    );
                    continue;
                }
                refusals += 1;
                assert_eq!(held.len(), 1, "{access:?} + {mode:?}: {held:#?}");
                assert_eq!(
                    held[0].code,
                    DiagnosticCode::WideningPermissionMode,
                    "`access: {}` does not admit `{}` and the refusal is not the widening one",
                    access.as_str(),
                    mode.as_str()
                );
                // …and the repair names **every** level that does admit the
                // mode, narrower ones included. The negative corpus pins two of
                // these seven messages word for word; this is the property all
                // seven hold, and it is the one a sentence that only knew how to
                // *widen* would fail: `plan` under `workspace_write` would be
                // answered with `full_access` alone — the widest containment
                // this grammar grants, offered to reach the one mode that
                // executes no tool — while `read_only`, the level the mode
                // belongs under, went unnamed (PRD G3, resolved q22).
                let help = held[0].help.as_deref().unwrap_or_default();
                for level in crate::harness::levels_admitting(Harness::Cc, mode) {
                    assert!(
                        help.contains(&format!("`access: {}`", level.as_str())),
                        "`access: {}` does not admit `{}` and the help does not name \
                         `access: {}`, which does: {help}",
                        access.as_str(),
                        mode.as_str(),
                        level.as_str()
                    );
                }
            }
        }
        assert_eq!(
            refusals, 7,
            "the admissibility table changed shape: `read_only` refuses five of the six modes, \
             `workspace_write` refuses two, and `full_access` refuses none (grammar 8.9)"
        );

        // …and the absent key, which is the form every composition written
        // before this one had: it is checked by nothing here, and the mode it
        // means is the level's own.
        let absent = crate::check(&ir_of(
            &composition("read_only", "plan").replace("        permission_mode: plan\n", ""),
        ));
        assert!(absent.is_empty(), "{absent:#?}");
    }

    /// **Neither connection rule fires where it should not** (grammar 8.9,
    /// Decision D143, PRD resolved q58 rulings b and c).
    ///
    /// The negative corpus pins what each refusal says; this is the direction a
    /// corpus of refusals cannot reach, and both cases are ones an over-eager
    /// implementation gets wrong in a way nothing else notices — a composition
    /// that *should* build stops building, and the author's only clue is a
    /// diagnostic about a line that is correct.
    ///
    ///  1. **a keyless gateway leaves the credential variable free.** Resolved
    ///     q25's shape is a `base_url:` with no `api_key:`, and the map then
    ///     injects no credential at all — so a node that declares
    ///     `ANTHROPIC_API_KEY:` itself is shadowing nothing, and refusing it
    ///     would make the keyless posture unusable on exactly the harness that
    ///     carries the connection as variables. The collision list is computed
    ///     from what the provider **declares**, and this is what says so;
    ///  2. **a slot that is an option is not a variable.** `base_url:` under
    ///     `codex` becomes a `--config` flag on the CLI and touches no
    ///     environment, so a node `env:` may spell anything beside it. A list
    ///     built from the table's facts rather than from its *slots* would
    ///     refuse a composition over a name nothing writes.
    #[test]
    fn a_connection_that_sets_no_variable_leaves_a_nodes_env_alone() {
        use crate::codegen::test_support::ir_of;

        let composition = |provider: &str, harness: &str, variable: &str| {
            format!(
                "version: \"0.1\"\n\
{provider}\
model.m:\n  provider: provider.p\n  id: some-model\n\
state:\n  summary: {{ type: string, default: \"\" }}\n\
flow.main:\n  outputs:\n    summary: {{ type: string }}\n  nodes:\n    build:\n      coder:\n        harness: {harness}\n        model: model.m\n        workspace: \"'/srv/checkout'\"\n        prompt: Do the work.\n        env:\n          {variable}: ${{HELD}}\n        output:\n          summary: {{ type: string }}\n      input: \"'go'\"\n  edges:\n    - {{ from: start, to: build }}\n    - {{ from: build, to: end }}\n"
            )
        };

        // 1. The keyless gateway, on the harness whose slots are variables.
        let keyless = crate::check(&ir_of(&composition(
            "provider.p:\n  kind: anthropic\n  base_url: ${GATEWAY_URL}\n",
            "cc",
            "ANTHROPIC_API_KEY",
        )));
        assert!(
            keyless.is_empty(),
            "a provider with no `api_key:` injects no credential variable, so a node declaring \
             one shadows nothing: {keyless:#?}"
        );

        // …and the control, so the assertion above is not passing because this
        // check never fires: the same node under a provider that *does* declare
        // the credential is the corpus fixture's shape.
        let declared = crate::check(&ir_of(&composition(
            "provider.p:\n  kind: anthropic\n  base_url: ${GATEWAY_URL}\n  api_key: ${GATEWAY_KEY}\n",
            "cc",
            "ANTHROPIC_API_KEY",
        )));
        assert_eq!(declared.len(), 1, "{declared:#?}");
        assert_eq!(
            declared[0].code,
            DiagnosticCode::ConflictingConnectionVariable
        );

        // 2. A slot that is an option rather than a variable. `CODEX_API_KEY` is
        // the one name this harness *does* reach the environment with, and the
        // provider below declares no credential — so nothing is set and the node
        // may spell it.
        let option = crate::check(&ir_of(&composition(
            "provider.p:\n  kind: openai\n  base_url: ${GATEWAY_URL}\n",
            "codex",
            "CODEX_API_KEY",
        )));
        assert!(
            option.is_empty(),
            "`base_url:` under `codex` is a client option and sets no variable, so a node's \
             `env:` is free beside it: {option:#?}"
        );

        // 3. …and the same two directions on a **sibling** rather than on the
        // slot. A keyless gateway claims no credential name at all, so the
        // second spelling is free beside the first.
        let sibling_free = crate::check(&ir_of(&composition(
            "provider.p:\n  kind: anthropic\n  base_url: ${GATEWAY_URL}\n",
            "cc",
            "ANTHROPIC_AUTH_TOKEN",
        )));
        assert!(
            sibling_free.is_empty(),
            "a provider with no `api_key:` claims no credential name — not the slot and not a \
             name read beside it: {sibling_free:#?}"
        );

        // …and the same on `codex`, whose credential reaches the environment
        // through the SDK's own injection rather than through a variable slot:
        // a keyless gateway leaves the whole auth family free, and the declared
        // one takes it back.
        let codex_sibling_free = crate::check(&ir_of(&composition(
            "provider.p:\n  kind: openai\n  base_url: ${GATEWAY_URL}\n",
            "codex",
            "OPENAI_API_KEY",
        )));
        assert!(
            codex_sibling_free.is_empty(),
            "a `codex` provider with no `api_key:` claims none of the CLI's three auth \
             variables: {codex_sibling_free:#?}"
        );
        let codex_declared = crate::check(&ir_of(&composition(
            "provider.p:\n  kind: openai\n  base_url: ${GATEWAY_URL}\n  api_key: ${GATEWAY_KEY}\n",
            "codex",
            "CODEX_ACCESS_TOKEN",
        )));
        assert_eq!(codex_declared.len(), 1, "{codex_declared:#?}");
        assert_eq!(
            codex_declared[0].code,
            DiagnosticCode::ConflictingConnectionVariable
        );
    }

    /// **A second spelling of a declared fact is the same refusal** (grammar
    /// 8.9, Decision D143, PRD resolved q58 rulings a and c).
    ///
    /// The hole the mapped-name list left open, read through the checker. `cc`'s
    /// connection surface is the whole environment contract its bundled runtime
    /// reads — ruling a says so — and a guard over the three names the table
    /// writes let a node `env:` add a bearer identity the gateway would honour
    /// and repoint the endpoint through the selector family, with `validate`
    /// clean and the graph document still drawing `ANTHROPIC_BASE_URL` and
    /// `ANTHROPIC_API_KEY`.
    ///
    /// Each entry earns **one** diagnostic and it names the mapped variable, so
    /// an author who never wrote `ANTHROPIC_BASE_URL` is still told what the
    /// name they did write collides with.
    #[test]
    fn a_second_spelling_of_a_declared_fact_is_refused_beside_the_slot() {
        use crate::codegen::test_support::ir_of;

        let composition = "version: \"0.1\"\n\
provider.gateway:\n  kind: anthropic\n  base_url: ${GATEWAY_URL}\n  api_key: ${GATEWAY_KEY}\n\
model.m:\n  provider: provider.gateway\n  id: some-model\n\
state:\n  summary: { type: string, default: \"\" }\n\
flow.main:\n  outputs:\n    summary: { type: string }\n  nodes:\n    build:\n      coder:\n        harness: cc\n        model: model.m\n        workspace: \"'/srv/checkout'\"\n        prompt: Do the work.\n        env:\n          PATH: /usr/bin:/bin\n          ANTHROPIC_AUTH_TOKEN: ${SOMEONE_ELSES_TOKEN}\n          CLAUDE_CODE_USE_BEDROCK: \"1\"\n          ANTHROPIC_BEDROCK_BASE_URL: ${ELSEWHERE}\n        output:\n          summary: { type: string }\n      input: \"'go'\"\n  edges:\n    - { from: start, to: build }\n    - { from: build, to: end }\n";

        let held = crate::check(&ir_of(composition));
        assert_eq!(
            held.len(),
            3,
            "three `env:` entries respell two declared facts, and `PATH` is what an `env:` is \
             for: {held:#?}"
        );
        for diagnostic in &held {
            assert_eq!(
                diagnostic.code,
                DiagnosticCode::ConflictingConnectionVariable,
                "{diagnostic:#?}"
            );
        }
        assert!(
            held[0]
                .message
                .contains("`ANTHROPIC_AUTH_TOKEN` is a second spelling of `api_key:`")
                && held[0]
                    .message
                    .contains("reads it beside the `ANTHROPIC_API_KEY`"),
            "{:#?}",
            held[0]
        );
        for held in &held[1..] {
            assert!(
                held.message.contains("second spelling of `base_url:`")
                    && held.message.contains("beside the `ANTHROPIC_BASE_URL`"),
                "{held:#?}"
            );
        }
    }

    /// **A kind no harness carries is told there is one repair** (grammar 8.9,
    /// Decision D143, PRD resolved q58 ruling b).
    ///
    /// `bedrock`, `vertex` and `azure_openai` are the three kinds on neither
    /// row, so every one of them takes the branch where the "does carry it"
    /// half of the help has nothing to name. A help that still closed on *run
    /// the node under the harness that speaks this one* would, in the same
    /// paragraph, tell an author no harness carries the connection and then send
    /// them to the harness that does.
    ///
    /// **All three are driven**, and the third is the one a doc comment naming
    /// it was not enough for: `azure_openai` is the kind whose keyword the
    /// sentence has to article correctly, and the branch shipped reading "a
    /// `azure_openai` connection" for exactly as long as no case drove it. The
    /// corpus pins the `bedrock` wording; this pins that the wording is composed
    /// per kind rather than written once against the kind that happened to have
    /// a fixture.
    #[test]
    fn a_kind_no_harness_carries_is_told_there_is_one_repair() {
        use crate::codegen::test_support::ir_of;

        for (provider, kind, expected) in [
            (
                "provider.p:\n  kind: bedrock\n  region: us-east-1\n  access_key_id: \
                 ${AWS_KEY_ID}\n  secret_access_key: ${AWS_SECRET}\n",
                "bedrock",
                "carries a `bedrock` connection",
            ),
            (
                "provider.p:\n  kind: vertex\n  project: acme\n  location: us-central1\n  \
                 credentials_json: ${GOOGLE_CREDENTIALS}\n",
                "vertex",
                "carries a `vertex` connection",
            ),
            (
                "provider.p:\n  kind: azure_openai\n  base_url: ${AZURE_ENDPOINT}\n  api_key: \
                 ${AZURE_KEY}\n  api_version: \"2024-10-21\"\n",
                "azure_openai",
                "carries an `azure_openai` connection",
            ),
        ] {
            for harness in ["cc", "codex"] {
                let composition = format!(
                    "version: \"0.1\"\n\
{provider}\
model.m:\n  provider: provider.p\n  id: some-model\n\
state:\n  summary: {{ type: string, default: \"\" }}\n\
flow.main:\n  outputs:\n    summary: {{ type: string }}\n  nodes:\n    build:\n      coder:\n        harness: {harness}\n        model: model.m\n        workspace: \"'/srv/checkout'\"\n        prompt: Do the work.\n        output:\n          summary: {{ type: string }}\n      input: \"'go'\"\n  edges:\n    - {{ from: start, to: build }}\n    - {{ from: build, to: end }}\n"
                );
                let held = crate::check(&ir_of(&composition));
                assert_eq!(held.len(), 1, "{kind} under {harness}: {held:#?}");
                assert_eq!(held[0].code, DiagnosticCode::UnsupportedProviderKind);
                let help = held[0].help.clone().expect("the refusal carries a help");
                assert!(
                    help.contains("there is nowhere to move this node to"),
                    "{help}"
                );
                assert!(
                    !help.contains("run the node under"),
                    "the help says no harness carries this connection and then sends the author \
                     to the harness that does: {help}"
                );
                // …and it reads like a sentence for every kind, not only for the
                // one the corpus happens to have a fixture for.
                assert!(
                    help.contains(expected),
                    "the one-repair help does not read `{expected}`: {help}"
                );
            }
        }
    }

    /// The names one emitted `<HARNESS>_SETTINGS` array lists, in order.
    fn emitted(module: &str, declaration: &str) -> Vec<String> {
        let array = module
            .split_once(&format!("const {declaration}: readonly string[] = ["))
            .unwrap_or_else(|| panic!("the emitted module declares `{declaration}`"))
            .1;
        let array = &array[..array.find("];").expect("…and closes the array it opened")];
        array
            .split(',')
            .map(|piece| piece.trim().trim_matches('"').to_string())
            .filter(|piece| !piece.is_empty())
            .collect()
    }

    /// **The two curated `settings:` tables are one table.**
    ///
    /// [`CC_SETTINGS`](super::CC_SETTINGS) and
    /// [`CODEX_SETTINGS`](super::CODEX_SETTINGS) here are the keys `validate`
    /// checks strictly (Decision D140's first tier). `CC_SETTINGS` and
    /// `CODEX_SETTINGS` in `src/harness-cc.ts` and `src/harness-codex.ts` are
    /// those same two lists declared again for the runtime, where `passthrough`
    /// reads them to hold a curated key back from the SDK because the driver
    /// beside it maps that key by name. Two hand-maintained copies of one
    /// document drift in silence — the argument
    /// `the_harness_model_settings_table_is_one_table` in `src/codegen/graph.rs`
    /// is written under, put on the other pair.
    ///
    /// Read in **three** directions, because a curated key has to be in three
    /// places and a lapse in any one of them is silent:
    ///
    ///  1. every key this compiler checks strictly is in the emitted list. Drop
    ///     it there and `passthrough` stops holding the key back, so the value
    ///     travels to the SDK *beside* the option the driver already set from
    ///     it — two spellings of one option, which is the collision the curated
    ///     list exists to prevent.
    ///  2. the emitted list holds no key this compiler does not check. That is
    ///     the direction which makes `validate` **lie**: a key the runtime
    ///     treats as curated and this table never learned earns
    ///     `unknown-harness-setting` — "its value is unverified" — while the
    ///     driver reads it and applies it all the same.
    ///  3. each key is really read by that harness's driver, which is the whole
    ///     reason either list exists: a key checked strictly here, filtered out
    ///     of the passthrough there, and mapped by nobody is a setting
    ///     `validate` verifies and the run then silently drops.
    #[test]
    fn the_curated_settings_table_is_one_table() {
        for (harness, declaration, driver, module) in [
            (
                Harness::Cc,
                "CC_SETTINGS",
                include_str!("../codegen/js/harness-cc.ts"),
                "src/harness-cc.ts",
            ),
            (
                Harness::Codex,
                "CODEX_SETTINGS",
                include_str!("../codegen/js/harness-codex.ts"),
                "src/harness-codex.ts",
            ),
        ] {
            let checked: Vec<String> = table(harness)
                .iter()
                .map(|(key, _)| (*key).to_string())
                .collect();
            assert_eq!(
                emitted(driver, declaration),
                checked,
                "`{declaration}` in `{module}` and `{declaration}` in `src/check/coder.rs` are \
                 two copies of one table and have parted company: a key in one and not the other \
                 is either a setting `validate` checks and the passthrough duplicates, or one it \
                 warns is unverified while the driver applies it"
            );
            for key in &checked {
                assert!(
                    driver.contains(&format!("run.settings[\"{key}\"]")),
                    "`{key}` is a curated `harness: {}` setting and `{module}` never reads it, so \
                     the value is checked strictly, filtered out of the passthrough, and applied \
                     to nothing",
                    harness.as_str()
                );
            }
        }

        // A reserved harness has no driver and no curated table, which is the
        // answer that claims least: `validate` refuses the composition before a
        // settings key of one is ever read.
        for reserved in [Harness::DeepAgents, Harness::Native] {
            assert!(
                table(reserved).is_empty(),
                "a reserved harness has no SDK to take a setting"
            );
        }
    }
}
