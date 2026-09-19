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

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::{Cel, Interpolated, Literal};
use crate::ast::flow::{Harness, PermissionMode, WorkspaceAccess};
use crate::cel::Read;
use crate::cel::ty::Type;
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::harness::{
    Answered, ConnectionFact, ReservedHit, ReservedSpelling, Slot, admitted, declared,
    harnesses_with_a_permission_axis, has_permission_axis, kinds_of, levels_admitting,
    permission_level, provider_of, reserved_reached, slot_of, speaks, variables_read,
    variables_set,
};
use crate::ir::definition::{DefinitionBody, Model, Provider};
use crate::ir::flow::{Coder, Flow, Node};
use crate::parse::reader::{article, list, suggest};

use super::channels;
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
///
/// One thing is decided around the front-end rather than by it, and it is the
/// **migration**: a value that is a path rather than an expression is what every
/// composition written before resolved q61 holds, and it is answered with the
/// rewrite rather than with a message about the expression it was mistaken for
/// ([`written_as_a_path`], [`a_bare_relative_path`], PRD G3).
///
/// A value this refuses is **rejected** for the two rules stated over the same
/// expression ([`dispatched_workspaces`], [`concurrent_workspaces`]), exactly as
/// [`edge_guard`](super::expr::edge_guard) rejects a guard the routing analyses
/// must not read again. Those two ask what the expression *depends on*, and
/// `cel::analyze` answers "nothing" — or a handful of roots nobody declared —
/// for a source that is not one, so a `workspace:` already reported as a path
/// would come back a second time as one directory for every dispatch, telling an
/// author to make a repair they have just been told to make, about an expression
/// they did not write (PRD G3, resolved q22).
fn workspace(ctx: &mut Ctx<'_>, cx: &FlowCx<'_>, subject: &str, coder: &Coder) {
    let Some(expression) = coder.workspace.value.expression() else {
        return;
    };
    if written_as_a_path(expression.as_str()) {
        a_path_where_an_expression_goes(ctx, subject, coder, expression);
        ctx.reject_workspace(&coder.workspace.span);
        return;
    }
    let written = Spanned::new(Cel::new(expression.as_str()), coder.workspace.span.clone());
    let scope = super::expr::flow_scope(ctx, cx, "a `coder:` node's `workspace:`");
    // Run before anything is reported, because the second migration spelling is
    // decided on what the front-end made of the value rather than on its first
    // character ([`a_bare_relative_path`]).
    let analysis = crate::cel::analyze(written.value.as_str(), &scope);
    if a_bare_relative_path(expression.as_str(), &analysis) {
        a_path_where_an_expression_goes(ctx, subject, coder, expression);
        ctx.reject_workspace(&coder.workspace.span);
        return;
    }
    super::expr::report(ctx, &written, &analysis);
    super::expr::expect(
        ctx,
        &written,
        &analysis,
        &Type::String,
        &format!("`workspace:` of {subject}"),
    );
    if !analysis.problems.is_empty() || !analysis.ty.assignable_to(&Type::String) {
        ctx.reject_workspace(&coder.workspace.span);
    }
}

/// Whether a `workspace:` value is an **anchored** path rather than an
/// expression — three of the four spellings every composition written before
/// PRD resolved q61 has.
///
/// Decided on the first character, and the set is the one CEL cannot start an
/// expression with: `/` and `~` open an absolute path, `$` opens an env
/// reference (`${REPO_ROOT}`, `${REPO_ROOT}/sub`), `\` opens a Windows one, and
/// a `.` that starts no number opens a relative path. Nothing in that set is
/// the beginning of any expression this grammar admits, so a value holding one
/// is a path with certainty rather than by guess — which is what lets the
/// refusal below say so and name the rewrite.
///
/// **Every anchored migration spelling lands here**, and that is the point: an
/// author upgrading `workspace: ${REPO_ROOT}` and one upgrading
/// `workspace: /srv/repo` made the same mistake and get the same answer. The
/// fourth spelling — a path anchored at nothing, `checkouts/main` — is legal CEL
/// to the character and is decided by [`a_bare_relative_path`] instead.
fn written_as_a_path(value: &str) -> bool {
    let mut characters = value.chars();
    match characters.next() {
        Some('/' | '~' | '$' | '\\') => true,
        Some('.') => !characters.next().is_some_and(|next| next.is_ascii_digit()),
        _ => false,
    }
}

/// Whether a `workspace:` value is an **un-anchored** relative path —
/// `checkouts/main`, the pre-q61 spelling [`written_as_a_path`] cannot see.
///
/// This one is not a mistake the front-end stops at: `checkouts/main` is a legal
/// CEL division of two identifiers, so it parses, and what comes back is one
/// `unknown-root` per segment. That is the message resolved q22 exists to
/// prevent — an author migrating a relative checkout root is told about roots
/// they never wrote, twice, and the rewrite that would fix it is named nowhere.
///
/// So the value is read *through* what the front-end made of it, which is the
/// only evidence that separates it from a real expression. Three things have to
/// hold together, and each rules out a shape the first two would take:
///
/// * it holds a `/` — a path segment separator, and the operator a path is
///   mistaken for;
/// * **everything** the walk objected to is a root that does not exist. A
///   genuine expression over the roots this surface exposes —
///   `"'/srv/' + input.branch"`, or even a nonsensical `"input.a / input.b"` —
///   objects about something else or about nothing, and keeps the front-end's
///   own answer, which is the better one for it. So does a value with a real
///   syntax error, which parses to nothing and objects about no root at all;
/// * every path it reads is a **bare** root — no member selected off it, no
///   index. This is the one that tells `checkouts/main` from a typo:
///   `"stat.checkout + '/x'"` is an expression whose author meant `state`, and
///   everything it objects to is an unknown root too, but it *selects a member*
///   off that root, which no path segment does. Told apart the other way it
///   would be answered with a rewrite that quotes the typo (PRD G3).
fn a_bare_relative_path(value: &str, analysis: &crate::cel::Analysis) -> bool {
    value.contains('/')
        && !analysis.problems.is_empty()
        && analysis
            .problems
            .iter()
            .all(|problem| problem.code == DiagnosticCode::UnknownRoot)
        && analysis
            .reads
            .iter()
            .all(|read| read.path.is_empty() && !read.indexed)
}

/// The refusal [`written_as_a_path`] names, with the rewrite in it (PRD
/// resolved q61 ruling a, G3).
///
/// The CEL front-end could only report either spelling as a syntax error naming
/// a character, and a message an author cannot act on is the failure mode
/// resolved q22 is about. So the three spellings the key now has are named
/// instead — the same directory as a CEL string, the per-dispatch form the
/// ruling exists for, and `fresh` — and the first is built out of the path the
/// author already wrote, escaped for the literal it is being put inside.
fn a_path_where_an_expression_goes(
    ctx: &mut Ctx<'_>,
    subject: &str,
    coder: &Coder,
    expression: &Interpolated,
) {
    let path = expression.as_str();
    let quoted = format!("'{}'", cel_literal(path));
    let per_item = format!("'{}/'", cel_literal(path.trim_end_matches('/')));
    // Only a value that *has* references can carry one through the rewrite, and
    // a sentence about the environment on a value with none would be noise.
    let environment = if expression.references.is_empty() {
        String::new()
    } else {
        ", and the reference still resolves from the environment".to_string()
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidExpression,
            coder.workspace.span.clone(),
            format!("`workspace: {path}` of {subject} is a path where an expression is written"),
        )
        .with_help(format!(
            "`workspace:` is evaluated in this node's input scope at each dispatch, so a path \
             has to be written as one: `workspace: \"{quoted}\"` is that same directory as a \
             CEL string{environment}. What the expression buys is the dispatch — \
             `workspace: \"{per_item} + input.branch\"` gives a map item its own checkout, and \
             `workspace: fresh` takes one the runtime provisions per dispatch (grammar 4.1, \
             8.9, Decision D147, PRD resolved q61 ruling a)"
        )),
    );
}

/// One string as the body of a single-quoted CEL literal.
///
/// A path holding a quote or a backslash is a path, and a rewrite this compiler
/// suggests has to be one an author can paste: `'/srv/o\'brien'` is that
/// directory and `'/srv/o'brien'` is a syntax error with our name on it.
fn cel_literal(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
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
/// The first test is [`reach::is_item_derived`], the same predicate a store key
/// is held to (Decision D83): an expression reads the per-dispatch scope when it
/// reads the dispatch's own `execution.item_index`, or an `input.<field>` the
/// map bound from the item.
///
/// # A `state` channel is per-dispatch exactly when the reader can observe a
/// write of it
///
/// The second test is [`channels::writers_within`], and it is the half a store
/// key does not need. A dispatched flow instance is a separate run of a separate
/// compiled graph: the channel set is composition-global in *shape* and
/// per-instance in *value*, seeded at each channel's `default:` with nothing
/// crossing the boundary but `inputs:` (grammar 10.1, 7.6.4 rules 2 and 3, PRD
/// 5.7). So `state.<channel>` is two different things depending on one fact
/// about the instance, and the rule reads that fact rather than assuming either
/// answer:
///
/// * a channel **a node that dominates this one writes** holds that instance's
///   own value, produced by that instance's own run, and holds it by the time
///   this node runs. `prepare` — an `exec:` node running `git worktree add` —
///   writing `checkout`, and `implement` reading `workspace: "state.checkout"`
///   downstream of it, is four dispatches preparing four directories, and it is
///   the supported shape ruling c names in as many words: "an upstream
///   `exec:`/`tool.*` step cloning or worktree-ing into per-item paths".
///   Refusing it would refuse the repair, and there is no in-instance way around
///   the refusal — a coder node's `workspace:` cannot read another node's output
///   (Decision D42), so a channel is the only way to carry that answer to it.
/// * a channel **nothing upstream of this node writes** holds its `default:`
///   here, which is one directory for the whole fan-out as surely as a literal
///   path is. Three shapes land in it and they are one fact: no node of the
///   instance writes the channel at all; the only writer runs *after* this node,
///   so every dispatch reads the `default:` and the write is a record nobody in
///   the instance ever reads back; or the only writer sits on a branch this node
///   may reach `start` without passing through.
///
/// **Upstream is dominance, and it is dominance because the value's existence has
/// to be a guarantee rather than a hope.** Channel values are ordered by step
/// (grammar 7.6.4), so the question "does this node see this instance's own
/// value" is the question grammar 8.6 rule 11 asks of `map.over`, answered the
/// way Decision D76 answers it: every path from `start` to the reader passes
/// through a writer. Mere path-existence would accept a writer on a guarded
/// sibling branch that did not run, and a rule that accepted it would be
/// exempting a composition from a *race refusal* on the strength of a write that
/// may never have happened.
///
/// Whether two instances that each observe a write of a channel write the *same*
/// string is a runtime fact, and it is the same one ruling b already declines to
/// refuse over where it withholds the error from [`concurrent_workspaces`]: this
/// compiler states what it can see rather than pretending to see more.
///
/// Everything left — a literal path, an `${ENV}` reference, a channel nothing
/// upstream writes — is one directory for the whole fan-out, which is the
/// field report this ruling comes from: the dispatches clobber each other's
/// checkout and `max_concurrency` was a lie.
///
/// Two more ways out of the rule, and both are statements rather than escapes.
/// `max_concurrency: 1` says the runs are serial, and the refusal lifts because
/// there is no longer a race to have. `workspace: fresh` satisfies it by
/// construction — the runtime provisions a directory per dispatch — and never
/// reaches this check at all, having no expression to read.
///
/// **The boundary is [`reach::frames`]'s own**, and it has two edges worth
/// naming rather than leaving to be found.
///
/// Derivation travels inward through `flow:` nodes and **stops at an agent's
/// `flow.*` tool**, because a model decides whether and when to call one and no
/// static rule can put that call inside a dispatch. A coder node reached only
/// that way is outside this rule, exactly as a store node reached only that way
/// is outside grammar 11.4's — and outside
/// [`concurrent_workspaces`]'s as well, which draws the same boundary.
///
/// And a **nested** map is read at the looser of its own bound and the fan-out
/// it is inside, which is [`reach::frames`]' own doing. A map declaring
/// `max_concurrency: 1` inside a flow an outer map fans four ways runs serially
/// *within each of four concurrent instances*, so four harness runs are in one
/// directory at once — the field report's race, one construct deeper — and
/// reading the inner 1 would accept it. The frame carries both numbers, and the
/// message below quotes the map's own while naming the fan-out that raised it,
/// because `max_concurrency: 1` is already written on the line a message quoting
/// four would send the author to.
///
/// **The bound is not the only thing nesting changes**, and the second half is
/// what makes the first half honest. Item-derivation is one map's dispatches
/// against each other: `input.file` bound from the inner map's item is a
/// directory per dispatch *within one instance*, and the other three instances
/// iterate lists of their own — so a file name two checkouts share is one
/// directory holding two runs, which is the very race read at the outer bound to
/// catch. A dispatch inside a fan-out therefore answers for both axes: this
/// map's dispatches ([`reach::is_item_derived`] over [`reach::Frame::derived`]),
/// and the instances issuing them ([`reach::reads_a_varying_field`] over
/// [`reach::Frame::per_instance`], which carries inward what the enclosing
/// instance itself varies). Neither the item nor `execution.item_index` settles
/// the second — the index is the innermost dispatch's and repeats across outer
/// items (grammar 4.1, Decision D115) — and the refusal says which of the two
/// failed, because sending an author who bound the item's path to bind it again
/// is sending them to the line they wrote correctly (PRD G3).
///
/// # One diagnostic per `workspace:`
///
/// A node is reported **once**, whatever number of frames reach it. A routed map
/// naming one flow from two routes with two different item-derivations seeds two
/// frames that no dedup in [`reach::frames`] merges — they are two sites — but
/// the mistake is one line and the repair is one edit, so a second byte-identical
/// sentence at the same span would only bury the first. That is the policy
/// [`concurrent_workspaces`] states for its own quadratic, kept here: the frame
/// that speaks is the one with the **loosest** bound, which is the worst case the
/// author has to answer for.
pub(crate) fn dispatched_workspaces<'a>(ctx: &mut Ctx<'a>) {
    // Keyed by the node and the position of its `workspace:` — one key per line
    // an author would have to edit.
    let mut reported: BTreeMap<(String, String, usize), Shared> = BTreeMap::new();
    // What an instance of each flow holds for a channel name, at each of its
    // nodes ([`Instance`]). A property of the flow rather than of the site, and
    // one flow is commonly a dispatch target several times over, so it is
    // answered once per address: `validate` is held to a millisecond budget over
    // a frame count nothing bounds (`tests/check_scale.rs`).
    let mut instances: BTreeMap<String, Instance<'a>> = BTreeMap::new();
    for frame in reach::frames(ctx) {
        // A serial dispatch is a statement the author made, and it is the
        // second repair this rule's message offers: one run at a time is one
        // run in the directory at a time (grammar 8.6 rule 1).
        if frame.concurrency <= 1 {
            continue;
        }
        for (at, node) in frame.flow.nodes.iter().enumerate() {
            let crate::ir::flow::NodeKind::Coder { coder } = &node.kind else {
                continue;
            };
            let Some(expression) = coder.workspace.value.expression() else {
                continue;
            };
            // An expression the CEL front-end already refused reads nothing
            // because it has no parse, not because it names one directory
            // (see `workspace`).
            if !ctx.workspace_type_checked(&coder.workspace.span) {
                continue;
            }
            // The two axes a dispatch can be concurrent along, and the value has
            // to tell the runs on both of them apart (grammar 8.6 rule 1,
            // Decision D147): whether it distinguishes this map's own
            // dispatches, and — where the map is itself inside a fan-out —
            // whether it distinguishes the instances issuing them. `input.file`
            // off the inner map's item answers the first and not the second:
            // four instances stepping their own files serially still put two
            // runs in `/srv/README.md` the moment two of them name that file.
            let per_dispatch = reach::is_item_derived(expression.as_str(), None, &frame.derived);
            let instances_of_it = frame.instances();
            let by_instance = instances_of_it <= 1
                || reach::reads_a_varying_field(expression.as_str(), &frame.per_instance);
            if per_dispatch && by_instance {
                continue;
            }
            // The instance is consulted only where the expression reads
            // `state` at all, which is what keeps the dominance walks off the
            // clock for the `'${ROOT}'` shape the refusal is mostly about:
            // `validate` is held to a millisecond budget (`tests/check_scale.rs`).
            //
            // A channel the instance writes **itself** answers both axes at
            // once, which is why it is asked whichever of the two failed: a
            // dispatched instance is a separate run of a separate compiled graph
            // (grammar 10.1), so its own `prepare` step's answer is that
            // dispatch's and no other's, in whatever fan-out it is nested.
            let reads = state_reads(expression.as_str());
            let unobserved = if reads.is_empty() {
                None
            } else {
                let instance = instances
                    .entry(frame.address.clone())
                    .or_insert_with(|| Instance::of(ctx, frame.flow));
                if reads_an_instance_channel(&reads, instance.observed_at(at)) {
                    continue;
                }
                instance.unobserved(&reads, at)
            };
            // Which of the two decided it, because the repairs differ: an
            // expression the enclosing fan-out cannot tell apart is one whose
            // author already bound the item's own path, and sending them to bind
            // it again would be sending them to the line they wrote correctly
            // (PRD G3).
            let per_dispatch_only = per_dispatch && !by_instance;
            // The coder node and the map are both innocent: the edit is at the
            // binding that fed this `workspace:` a value the failing axis cannot
            // tell apart, which may be several instantiations away from either —
            // a `flow:` node between them, or the map's own `input:` entry. Name
            // it, for each `input.<field>` the expression reads, exactly as the
            // sibling rule over these frames does (`check::stores`, grammar 11.4's
            // worked example, PRD G3).
            //
            // **Of the axis that decided the refusal**, because the two answer
            // for different fields and the wrong one would label a line that is
            // correct: a field the map's item derives is a value per dispatch,
            // and four concurrent instances of that map still share it.
            let answers = if per_dispatch_only {
                &frame.per_instance
            } else {
                &frame.derived
            };
            let bound = reach::input_fields(expression.as_str())
                .into_iter()
                .filter_map(|field| {
                    if answers.get(&field) != Some(&false) {
                        return None;
                    }
                    let at = frame.bound_at.get(&field)?;
                    Some((field, at.clone()))
                })
                .collect();
            let subject = format!("`{}` node `{}`", frame.address, node.id.value);
            let key = (
                subject.clone(),
                coder.workspace.span.source.as_str().to_string(),
                coder.workspace.span.bytes.start,
            );
            let shared = Shared {
                subject,
                flow: frame.address.clone(),
                // A channel is not what decided a refusal the item-derivation
                // already satisfied, and naming one there would point a reader
                // at a second subject.
                unobserved: if per_dispatch_only { None } else { unobserved },
                at: coder.workspace.span.clone(),
                bound,
                dispatcher: frame.dispatcher.clone(),
                dispatched_at: frame.span.clone(),
                declared: frame.declared,
                concurrency: frame.concurrency,
                instances: instances_of_it,
                per_dispatch_only,
                enclosing: frame.enclosing.clone(),
                expression: expression.as_str().to_string(),
            };
            match reported.get(&key) {
                // The loosest bound wins, and a tie keeps the frame walked
                // first — which `reach::frames` fixes to the composition's own
                // order rather than to any loop's.
                Some(held) if held.concurrency >= shared.concurrency => {}
                _ => {
                    reported.insert(key, shared);
                }
            }
        }
    }
    for shared in reported.into_values() {
        ctx.push(shared.diagnostic());
    }
}

/// One dispatched flow instance, as the channel half of the rule reads it: its
/// graph, who writes what in it, and what each of its nodes can therefore
/// observe (grammar 10.1, 7.6.4, PRD resolved q61 ruling b).
///
/// Built once per flow address and memoized per node, because one flow is
/// commonly a dispatch target several times over and dominance costs a walk
/// each time it is asked (`tests/check_scale.rs`).
struct Instance<'a> {
    /// The instance's graph, which is where "can this node observe that write"
    /// is decided.
    graph: Graph<'a>,
    /// Every channel a node of it writes, paired with the nodes that write it
    /// ([`channels::writers_within`]).
    writers: BTreeMap<String, Vec<usize>>,
    /// Per node, the channels that node reads this instance's **own** value of.
    observed: BTreeMap<usize, BTreeSet<String>>,
}

impl<'a> Instance<'a> {
    fn of(ctx: &Ctx<'a>, flow: &'a Flow) -> Self {
        Self {
            graph: Graph::new(flow),
            writers: channels::writers_within(ctx, flow),
            observed: BTreeMap::new(),
        }
    }

    /// The channels one node of this instance reads the instance's own value of:
    /// those a node that **dominates** it writes (Decision D76's reading of the
    /// same question, grammar 8.6 rule 11).
    ///
    /// A node's own write is not one of them, and that is the ordering rather
    /// than a special case: a node's outputs land in its channels when it
    /// completes, so what it reads while it runs is what entered its step
    /// (grammar 7.6.4).
    fn observed_at(&mut self, at: usize) -> &BTreeSet<String> {
        let graph = &self.graph;
        let writers = &self.writers;
        self.observed.entry(at).or_insert_with(|| {
            writers
                .iter()
                .filter(|(_, wrote)| {
                    wrote
                        .iter()
                        .any(|writer| *writer != at && graph.dominates(*writer, at))
                })
                .map(|(channel, _)| channel.clone())
                .collect()
        })
    }

    /// The first channel a `workspace:` expression reads that the node at `at`
    /// does **not** observe this instance's own value of — the fact that decided
    /// the refusal where a `state` read is what it turned on, and the one the
    /// help has to state.
    ///
    /// A read that names no channel — the whole `state` object, or an index whose
    /// key this compiler could not resolve — answers nothing here: there is no
    /// channel to point an author at, and the general repairs are the whole of
    /// what can be offered.
    fn unobserved(&mut self, reads: &[Read], at: usize) -> Option<Unobserved> {
        let named: Vec<String> = reads
            .iter()
            .filter_map(|read| read.path.first().cloned())
            .collect();
        let observed = self.observed_at(at);
        let channel = named.into_iter().find(|name| !observed.contains(name))?;
        Some(match self.writers.get(&channel) {
            None => Unobserved::NeverWritten { channel },
            Some(wrote) => Unobserved::NotUpstream {
                writers: wrote
                    .iter()
                    .map(|writer| self.graph.id(*writer).to_string())
                    .collect(),
                channel,
            },
        })
    }
}

/// Why a `state` channel a refused `workspace:` reads holds one value for the
/// whole fan-out — the two shapes, because they take two different repairs.
enum Unobserved {
    /// No node of the dispatched flow writes the channel at all, so every
    /// instance reads its `default:`.
    NeverWritten { channel: String },
    /// Some node writes it, and none of them dominates the reader: the write
    /// lands after this node runs, or on a branch it can reach `start` without.
    NotUpstream {
        channel: String,
        /// The nodes that do write it, in declaration order.
        writers: Vec<String>,
    },
}

/// Every `state` path one `workspace:` expression reads, which is the whole of
/// what the channel half of the rule looks at.
///
/// Walked against an **empty** scope, as [`reach::is_item_derived`] is: the
/// question is what the expression depends on rather than what it evaluates to,
/// and the surface's own scope has already answered the second one
/// ([`workspace`]).
fn state_reads(source: &str) -> Vec<Read> {
    crate::cel::analyze(source, &crate::cel::Scope::default())
        .reads
        .into_iter()
        .filter(|read| read.root == "state")
        .collect()
}

/// Whether one `workspace:` expression reads a channel this node observes the
/// dispatched instance's **own** value of — a value that instance's own run
/// produced before this node ran, and so a directory of its own (grammar 10.1,
/// PRD resolved q61 ruling b).
///
/// Any such read is enough, exactly as [`reach::is_item_derived`] takes any
/// item-derived read: an expression over one per-instance value is per-instance
/// however the rest of it is written, and `state.checkout + '/pkg'` is the same
/// four directories `state.checkout` is.
///
/// A read that names **no** channel is answered the way a refusal has to answer
/// an unknown: a read of the whole `state` object embeds every channel, so one
/// the instance observes is one it carries; but an index whose key this compiler
/// could not resolve — `state[input.which]` — names a channel nobody here can
/// name, and admitting it would exempt a composition from a race refusal on the
/// strength of a fact nothing established.
fn reads_an_instance_channel(reads: &[Read], observed: &BTreeSet<String>) -> bool {
    reads.iter().any(|read| match read.path.first() {
        Some(channel) => observed.contains(channel),
        None if read.indexed => false,
        None => !observed.is_empty(),
    })
}

/// One refusal [`dispatched_workspaces`] has decided on, before it is worded.
struct Shared {
    /// How the coder node is named.
    subject: String,
    /// The dispatched flow's address, for the half of the help that is about
    /// the instance rather than about the node.
    flow: String,
    /// A channel the expression reads and this node does not observe the
    /// instance's own value of, where the expression names one
    /// ([`Instance::unobserved`]).
    unobserved: Option<Unobserved>,
    /// Its `workspace:`, which is the line the refusal anchors at.
    at: Span,
    /// The `input:` entries that fed the expression a value the failing axis
    /// cannot tell apart, paired with the field each one binds — the line an
    /// author actually edits, which is neither this node nor the map.
    bound: Vec<(String, Span)>,
    /// How the dispatching map is named.
    dispatcher: String,
    /// That map node's span.
    dispatched_at: Span,
    /// What the dispatching map's own `max_concurrency:` says.
    declared: i64,
    /// How many runs can really be in flight at once.
    concurrency: i64,
    /// How many instances of the enclosing fan-out issue those runs — 1 where
    /// nothing encloses the dispatching map.
    instances: i64,
    /// Whether the expression *is* per-dispatch and it is the enclosing
    /// fan-out's instances it cannot tell apart, which is a different mistake
    /// with a different repair.
    per_dispatch_only: bool,
    /// The fan-out this dispatch is inside, where it runs more than one instance
    /// at a time.
    enclosing: Option<reach::Enclosing>,
    /// The `workspace:` expression as written.
    expression: String,
}

impl Shared {
    fn diagnostic(self) -> Diagnostic {
        let Self {
            subject,
            flow,
            unobserved,
            at,
            bound,
            dispatcher,
            dispatched_at,
            declared,
            concurrency,
            instances,
            per_dispatch_only,
            enclosing,
            expression,
        } = self;
        // The per-instance half can only have decided a refusal where there is
        // an enclosing fan-out to have instances of, and the three places it is
        // worded below all name that fan-out — so the two facts are tied here
        // rather than left to agree.
        let per_dispatch_only = per_dispatch_only && enclosing.is_some();
        // The enclosing fan-out is named where it is what the refusal is about:
        // because it **raised** the bound the map declares, or because it is the
        // axis the expression failed on. A fan-out no tighter than the map
        // inside it, over an expression that is not per-dispatch at all, is a
        // sentence about the map alone.
        let outer = enclosing
            .as_ref()
            .filter(|outer| per_dispatch_only || outer.concurrency > declared);
        let message = match outer {
            // The expression tells the map's own dispatches apart and the
            // runs that overlap are not only those: the subject is the fan-out
            // around it, so that is what the sentence is about.
            Some(outer) if per_dispatch_only => format!(
                "{subject} is dispatched by {dispatcher}, and `workspace:` tells that map's own \
                 dispatches apart but not the {instances} instances of it {} runs at once",
                outer.dispatcher
            ),
            // The map's own bound *and* the one it really runs under, because
            // quoting either alone misreads the composition: the first says
            // `max_concurrency: 1` about a node four runs share, and the second
            // attributes a four to a map that declares one.
            Some(outer) => format!(
                "{subject} is dispatched by {dispatcher} with `max_concurrency: {declared}`, and \
                 that dispatch is itself inside a fan-out of {} issued by {}, so its \
                 `workspace:` is one directory for up to {concurrency} runs at once",
                outer.concurrency, outer.dispatcher
            ),
            None => format!(
                "{subject} is dispatched by {dispatcher} with `max_concurrency: {concurrency}`, \
                 and its `workspace:` is one directory for every dispatch"
            ),
        };
        // Two repairs where the map's own bound is the whole of it, and the same
        // two where it is not — but the serial one moves to the map that states
        // the bound, because the inner map already says `max_concurrency: 1`
        // and saying it again is not an edit.
        let serial = match outer {
            Some(outer) if per_dispatch_only => format!(
                "or declare `max_concurrency: 1` on {}, which leaves this map's own dispatches \
                 the only runs that overlap — and `{expression}` already tells those apart",
                outer.dispatcher
            ),
            Some(outer) => format!(
                "or declare `max_concurrency: 1` on {} as well, which says the whole fan-out is \
                 serial — the bound on {dispatcher} holds inside one instance and says nothing \
                 about the others",
                outer.dispatcher
            ),
            None => "or declare `max_concurrency: 1` on the map, which says these runs are serial"
                .to_string(),
        };
        // Where a `state` read is what the refusal turned on, the fact that
        // decided it is said, and the two shapes are said apart: a channel is
        // per-dispatch exactly when a node **upstream of this one** writes it,
        // so an author whose graph already has the upstream step would otherwise
        // be sent to restructure a composition one `writes:` destination — or
        // one edge — away from legal (grammar 10.1, 7.6.4, PRD G3).
        let instance = unobserved.map_or_else(String::new, |unobserved| match unobserved {
            Unobserved::NeverWritten { channel } => format!(
                " `{channel}` is a channel no node of `{flow}` writes, so every instance reads its \
                 `default:` — one directory for the whole fan-out. A channel the instance writes \
                 **itself** holds that instance's own value (grammar 10.1), which is how an \
                 upstream `exec:` or `tool.*` step that clones or worktrees per dispatch carries \
                 the path to this node."
            ),
            Unobserved::NotUpstream { channel, writers } => format!(
                " `{channel}` is written by {} of `{flow}`, but not by a node this one runs after \
                 on every path from `start`: a channel's writes land when their writer completes \
                 (grammar 7.6.4), so what this node reads is still the `default:` every instance \
                 starts at — one directory for the whole fan-out. The write has to **dominate** \
                 this node the way `map.over`'s producer dominates its map (grammar 8.6 rule 11), \
                 which is what an upstream `exec:` or `tool.*` step that clones or worktrees per \
                 dispatch does.",
                list(&writers)
            ),
        });
        let mut diagnostic = Diagnostic::error(DiagnosticCode::SharedWorkspace, at, message)
            .with_label(dispatched_at, "the fan-out is issued here");
        if let Some(outer) = outer {
            diagnostic =
                diagnostic.with_label(outer.span.clone(), "and that dispatch is inside this one");
        }
        // The line that actually has to change, where the value arrived through
        // an `input.<field>`: neither this node nor the map, but the `input:`
        // entry that fixed the field — which at depth is a `flow:` node's own,
        // several instantiations from either (grammar 11.4's worked example,
        // PRD G3). The two axes are labelled apart because they are two claims:
        // a field the map's item derives is per-dispatch and still one value all
        // of the enclosing instances share.
        let carried = list(bound.iter().map(|(field, _)| field));
        for (field, at) in bound {
            let shares = if per_dispatch_only {
                "to a value every instance of this map shares"
            } else {
                "to a value every dispatch shares"
            };
            diagnostic = diagnostic.with_label(at, format!("`{field}` is bound here, {shares}"));
        }
        // Named in the help as well as labelled, because the repair sentences
        // below quote an `input:` entry as the shape of the edit and a reader
        // has to know *which* entry theirs is: with a `flow:` node between the
        // map and this node, it is that node's, not the map's.
        let at_the_binding = if carried.is_empty() {
            String::new()
        } else {
            format!(
                " The line that repair edits is the binding labelled above ({carried}): that is \
                 where the value this `workspace:` reads is fixed, however many dispatches read it."
            )
        };
        if per_dispatch_only {
            return diagnostic.with_help(format!(
                "a harness run is contained by its `workspace:`, and `{expression}` is a \
                 directory per dispatch **within one instance** only: the {instances} instances \
                 running beside this one evaluate it over items of their own, and two of them \
                 reaching the same value — the same file name under two repositories — are two \
                 coding agents editing one checkout. Two repairs: carry the enclosing item's own \
                 directory in alongside this map's item, so the expression reads a value the \
                 outer fan-out makes distinct (bind it at {dispatcher} — `input: {{ dir: \
                 \"input.root + '/' + item.path\" }}` — and read it here as \
                 `workspace: \"input.dir\"`), or write `workspace: fresh` for a directory the \
                 runtime provisions per dispatch — {serial}.{at_the_binding} An item is a value drawn from a \
                 list, and two instances drawing from two lists can draw the same one, which is \
                 why the map's own item does not settle this (grammar 8.6, 8.9, Decision D147, \
                 PRD resolved q61 ruling b)"
            ));
        }
        diagnostic.with_help(format!(
            "a harness run is contained by its `workspace:`, so {concurrency} dispatches sharing \
             `{expression}` are {concurrency} coding agents editing one checkout and overwriting \
             each other's work.{instance} Two repairs, and each says something different about \
             the graph: bind the item's own path — carry it on the dispatch (`input: {{ worktree: \
             \"item.worktree\" }}`) and read it here (`workspace: \"input.worktree\"`), or write \
             `workspace: fresh` for a directory the runtime provisions per dispatch — {serial}.\
             {at_the_binding} `workspace:` is evaluated in this node's input scope at every \
             dispatch precisely so the first repair is writable (grammar 8.6, 8.9, Decision \
             D147, PRD resolved q61 ruling b)"
        ))
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
///
/// **One diagnostic per directory, not per pair of nodes that collide on it**,
/// which is the policy [`convergence`](super::convergence) states for its own
/// quadratic and keeps for the same reason: a fan of *w* concurrent coder nodes
/// writing one value collides w(w-1)/2 ways and is one mistake, and a report
/// that repeated the same sentence about the same directory once per pair would
/// bury it. The pair that names it is the one whose nodes come first in
/// declaration order; every pair is still *decided*, because a pair a later one
/// repeats is still what proves the collision (PRD G3).
///
/// **A concurrent step is a `flow:` node or a `map` node as well as a coder
/// node**, which is what [`reach::coders_within`] is for. An author who splits
/// work across two coder nodes, one who factors the same work into a subflow and
/// instantiates it twice, and one who fans it out of a `map` have written the
/// same graph — two harness runs in flight at once — and the last two used to get
/// silence. So the relation is stated over the runs a step *contains* rather than
/// over the coder nodes a flow declares, and one coder node instantiated twice is
/// two runs.
///
/// The `map` case is the one the other half of this rule cannot reach.
/// [`dispatched_workspaces`] decides the dispatches of a map *against each
/// other* and passes over every fan-out bounded at 1 — and `max_concurrency: 1`
/// is a repair its own help offers. That bound holds **inside** the map and says
/// nothing about the branch beside it, so a serial map and a sibling coder node
/// naming one directory are two harness agents in one checkout that the refusal
/// is right to stay silent about and this warning is what covers.
///
/// Two instances are not one scope, though, and the rule says so: a run reached
/// through a `flow:` or `map` node is compared only where its expression reads
/// **nothing** ([`reach::reads_nothing`]). `workspace: "input.worktree"` in a
/// subflow instantiated twice is two directories exactly when the two
/// instantiations bind two paths — which is the repair this ruling exists for —
/// and warning about it would be warning about the fix. The same expression under
/// a map is per-item for the same reason, and decided there by
/// [`dispatched_workspaces`]. A closed expression is where that difference cannot
/// arise, and it is also the shape the field report had.
pub(crate) fn concurrent_workspaces<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'_>,
    graph: &Graph<'_>,
    forks: &[super::convergence::Pair],
    memo: &mut reach::Coders<'a>,
) {
    // The steps that *could* hold a run, decided on the node's kind alone —
    // which is a read rather than a walk. Enumerating the runs first would
    // answer the same question more precisely and walk the whole composition
    // below every `flow:` node to do it, including the overwhelmingly common
    // case where the flow has no fork at all and no pair can exist: `validate`
    // is held to a millisecond budget over a nesting depth nothing bounds
    // (`tests/check_scale.rs`). A step admitted here that turns out to hold no
    // run costs a pair the loops below drop, and never a diagnostic.
    let of_interest: Vec<usize> = (0..graph.nodes().len())
        .filter(|at| {
            matches!(
                graph.node(*at).kind,
                crate::ir::flow::NodeKind::Coder { .. }
                    | crate::ir::flow::NodeKind::Flow { .. }
                    | crate::ir::flow::NodeKind::Map { .. }
            )
        })
        .collect();
    // The fork enumeration is the one `convergence::check` just ran over this
    // same graph, handed in rather than repeated: this rule contributes no pair
    // of its own, and re-deriving them would re-read every guard of every fork
    // — the per-edge pass `convergence::possible` exists to avoid
    // (`tests/check_scale.rs`, PRD 5.12).
    let concurrent = super::convergence::concurrent_among(graph, forks, &of_interest);
    // …so the walk happens for the steps a pair actually named, once each.
    let mut runs: BTreeMap<usize, Vec<Run>> = BTreeMap::new();
    for (one, other) in &concurrent {
        for at in [*one, *other] {
            runs.entry(at)
                .or_insert_with(|| runs_of(ctx, cx, graph, at, memo));
        }
    }
    // The chosen pair per directory, ordered by the two steps and then by the
    // two runs inside them, so which repetition is reported is a property of
    // the composition rather than of this loop.
    let mut collided: BTreeMap<String, (usize, usize, usize, usize)> = BTreeMap::new();
    for (one, other) in concurrent {
        for (first, run) in runs[&one].iter().enumerate() {
            for (second, against) in runs[&other].iter().enumerate() {
                if run.written != against.written {
                    continue;
                }
                // Two runs of one flow's own graph share that flow's scope;
                // anything reached through a `flow:` node does not, so there
                // the expressions have to read nothing to be one directory.
                let one_scope = run.local && against.local;
                let closed = run.closed && against.closed;
                if !one_scope && !closed {
                    continue;
                }
                let at = (one, other, first, second);
                match collided.get(&run.written) {
                    Some(held) if *held <= at => {}
                    _ => {
                        collided.insert(run.written.clone(), at);
                    }
                }
            }
        }
    }
    for (expression, (one, other, first, second)) in collided {
        let run = &runs[&one][first];
        let against = &runs[&other][second];
        let local = run.local && against.local;
        let message = if local {
            format!(
                "nodes `{}` and `{}` of `{}` can run concurrently and their `workspace:` is \
                 written the same way",
                graph.id(one),
                graph.id(other),
                cx.address
            )
        } else {
            format!(
                "{} and {} can run concurrently and their `workspace:` is written the same way",
                run.subject, against.subject
            )
        };
        // What the two runs share, said for the shape they are: one scope where
        // both are this flow's own nodes, and no scope at all where one is
        // inside an instance — which is the condition that admitted the pair.
        let shared = if local {
            format!(
                "`{expression}` is one expression over one scope, so both runs resolve it to one path"
            )
        } else {
            format!(
                "`{expression}` reads nothing from either run's own scope, so it is one path \
                 however the instances are bound"
            )
        };
        // …and the repair the same way: a run inside an instance has a place to
        // take a directory *from* that a node of this flow's own graph has not.
        let repair = if local {
            "Give one of them a directory of its own".to_string()
        } else {
            "Give one of them a directory of its own: carry it in on the `input:` of the step \
             that starts the instance — a `flow:` node or a map dispatch — and read it here \
             (`workspace: \"input.worktree\"`)"
                .to_string()
        };
        let mut diagnostic = Diagnostic::warning(
            DiagnosticCode::SharedWorkspace,
            against.span.clone(),
            message,
        );
        // One `workspace:` instantiated twice is one span, and a second label
        // on it would say the same thing twice about the same line. The two
        // **steps** are always two, so a pair the node ids cannot tell apart is
        // told apart by where each run is reached.
        if run.span != against.span {
            diagnostic = diagnostic.with_label(run.span.clone(), "the other run is contained here");
        }
        if !local {
            diagnostic = diagnostic
                .with_label(run.step.clone(), "one of the two runs is inside this step")
                .with_label(against.step.clone(), "and the other is inside this one");
        }
        ctx.push(diagnostic.with_help(format!(
            "two edges of one fork that are not provably exclusive can both fire, so the \
                 branches they start are concurrent — and two harness runs inside one directory \
                 edit each other's files. {shared}. {repair}, or write \
                 `workspace: fresh` on each for a directory the runtime provisions per \
                 dispatch. A **warning** rather than a refusal because equality here is a \
                 launch fact: a value bearing an `${{ENV}}` reference resolves on the machine \
                 that runs it, so two values this compiler reads as different may still be one \
                 directory, and two it reads as the same is the half it can see (grammar 7.6.1, \
                 8.9, Decision D147, PRD resolved q61 ruling b)"
        )));
    }
}

/// One harness run a step of this flow's graph can contain
/// ([`concurrent_workspaces`]).
struct Run {
    /// The `workspace:` expression as written.
    written: String,
    /// Where it is written, which is the site a diagnostic anchors at.
    span: Span,
    /// The step of *this* flow's graph the run is inside: the coder node
    /// itself, or the `flow:` node that starts the instance holding it. Two
    /// runs of one pair always have two of these, which one `workspace:`
    /// instantiated twice does not.
    step: Span,
    /// How a diagnostic names the run, for a pair this flow's own node ids
    /// cannot name on their own.
    subject: String,
    /// Whether the expression reads no root at all ([`reach::reads_nothing`]).
    closed: bool,
    /// Whether this is a coder node of *this* flow, as against one inside an
    /// instance a `flow:` node starts.
    local: bool,
}

/// The runs one step of this flow's graph contains, in declaration order.
///
/// A `workspace:` the CEL front-end refused contributes none: "written the same
/// way" is a statement about two expressions, and a source with no parse is not
/// one of those — it would pair with every other malformed value in the flow and
/// tell both authors their directories collide (see [`workspace`]).
fn runs_of<'a>(
    ctx: &Ctx<'a>,
    cx: &FlowCx<'_>,
    graph: &Graph<'_>,
    at: usize,
    memo: &mut reach::Coders<'a>,
) -> Vec<Run> {
    let node = graph.node(at);
    match &node.kind {
        crate::ir::flow::NodeKind::Coder { coder } => coder
            .workspace
            .value
            .expression()
            .filter(|_| ctx.workspace_type_checked(&coder.workspace.span))
            .map(|expression| Run {
                written: expression.as_str().to_string(),
                span: coder.workspace.span.clone(),
                step: node.span.clone(),
                subject: format!("`{}` node `{}`", cx.address, graph.id(at)),
                closed: reach::reads_nothing(expression.as_str()),
                local: true,
            })
            .into_iter()
            .collect(),
        // A `flow:` node starts one instance and a `map` node starts one per
        // item, and a run inside either is in flight while the step is — which
        // is the whole of what this rule asks. The two differ only in the verb a
        // diagnostic names them by, and in how many addresses the step reaches:
        // a routed map reaches one per route, and two routes onto one flow are
        // one containment said twice.
        crate::ir::flow::NodeKind::Flow { flow, .. } => within(
            ctx,
            graph,
            at,
            memo,
            "instantiated",
            &[flow.value.to_string()],
        ),
        crate::ir::flow::NodeKind::Map { map } => {
            let mut addresses: Vec<String> = Vec::new();
            for target in reach::targets(&map.dispatch) {
                // A dispatch target that is not a `flow.*` holds no coder node
                // (grammar 8.9).
                if target.value.namespace != crate::ast::common::Namespace::Flow {
                    continue;
                }
                let address = target.value.to_string();
                if !addresses.contains(&address) {
                    addresses.push(address);
                }
            }
            within(ctx, graph, at, memo, "dispatched", &addresses)
        }
        _ => Vec::new(),
    }
}

/// The runs a step contains through the flows it starts — one address for a
/// `flow:` node, one per distinct dispatch target for a `map`.
///
/// Every such run is inside an instance, so none of them is `local`: two
/// instances are not one scope (grammar 10.1), which is what makes
/// [`reach::reads_nothing`] the condition a pair of them is compared under.
fn within<'a>(
    ctx: &Ctx<'a>,
    graph: &Graph<'_>,
    at: usize,
    memo: &mut reach::Coders<'a>,
    verb: &str,
    addresses: &[String],
) -> Vec<Run> {
    let step = graph.node(at).span.clone();
    let mut runs = Vec::new();
    for address in addresses {
        for held in reach::coders_within(ctx, memo, address).iter() {
            let Some(expression) = held.coder.workspace.value.expression() else {
                continue;
            };
            if !ctx.workspace_type_checked(&held.coder.workspace.span) {
                continue;
            }
            runs.push(Run {
                written: expression.as_str().to_string(),
                span: held.coder.workspace.span.clone(),
                step: step.clone(),
                subject: format!(
                    "`{}` node `{}` {verb} by `{}`",
                    held.address,
                    held.node.id.value,
                    graph.id(at)
                ),
                closed: reach::reads_nothing(expression.as_str()),
                local: false,
            });
        }
    }
    runs
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
