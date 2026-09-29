//! A target's `harnesses:` entries, read against this compiler release and
//! against the composition the target deploys (grammar 14.8, Decision D151,
//! PRD resolved q66).
//!
//! PRD 5.12 pins a harness SDK per compiler release because a harness SDK *is*
//! what a coder node's behaviour is, and resolved q60 gives the second reason:
//! the reserved `settings:` list is an audit of **one** release's option
//! surface, and an unaudited release may carry an option that reaches around a
//! bound the node states. Resolved q66 lifts the first reason and keeps the
//! second: a target may move a harness's SDK, but only inside the range the
//! compiler audited it for ([`crate::codegen::harness::AUDITED`]).
//!
//! Every entry is judged here, in one place, in the order a reader would fix
//! it — and each stops at its first finding, because a second diagnostic about
//! an entry already refused would be a complaint about a key its author is
//! about to rewrite:
//!
//! 1. **the key names a harness** of the grammar's `harness:` enum (Decision
//!    D136) — `unknown-variant`, with a suggestion;
//! 2. **one this release lowers** — `deepagents` and `native` are refused as
//!    they are on a node, `unsupported-harness`: a reserved harness has no
//!    driver, so no project installs an SDK for it;
//! 3. **the version is a literal** — a `${…}` reference is `unexpected-env-ref`:
//!    a version is a build fact, not a secret, and `validate` has to be able to
//!    read it to hold it to the range;
//! 4. **an exact one** — a range, a tag or a specifier is `invalid-value`: the
//!    range is the compiler's to declare, not the deployment's (PRD 5.12);
//! 5. **inside the audited range** — `harness-sdk-outside-audited-range`, above
//!    it or below it;
//! 6. **and bound by some `coder:` node** — otherwise a *warning*,
//!    `unbound-harness-sdk`: the entry emits nothing, and a deploy file outlives
//!    the compositions it serves, so an entry waiting for the one that binds the
//!    harness is a key that does nothing today rather than a mistake.
//!
//! The first five need nothing but the entry and the compiler's tables; they
//! are here rather than in the parser so that one entry is judged in one pass
//! and a deploy file carrying a mistake still resolves into an artifact the
//! sixth — which needs the composition — can read.

use crate::ast::flow::Harness;
use crate::codegen::harness::{
    RangePlacement, audited_range, bound, package_of, placement_of, version_of,
};
use crate::diag::{Diagnostic, DiagnosticCode};
use crate::ir::deploy::HarnessSdk;
use crate::parse::lexical::env_references;
use crate::parse::reader::{list, suggest};

use super::Ctx;

/// Judge every entry of the active target's `harnesses:`.
pub(crate) fn check(ctx: &mut Ctx) {
    let Some(section) = ctx.ir.deploy.harnesses.as_ref() else {
        return;
    };
    let bound = bound(ctx.ir);
    for entry in section.entries.values() {
        let Some(harness) = lowered(ctx, entry) else {
            continue;
        };
        if !version_is_audited(ctx, entry, harness) {
            continue;
        }
        if !bound.contains(&harness) {
            ctx.push(
                Diagnostic::warning(
                    DiagnosticCode::UnboundHarnessSdk,
                    entry.name.span.clone(),
                    format!(
                        "`harnesses.{}` pins an SDK for a harness no `coder:` node binds, so it \
                         emits nothing",
                        entry.name.value
                    ),
                )
                .with_help(format!(
                    "a project installs a harness's SDK only where a `coder:` node binds that \
                     harness, so this entry changes nothing `build` writes; it is a warning rather \
                     than an error because a deploy file outlives the compositions it serves — \
                     keep it for the composition that binds `{}`, or drop it (grammar 14.8, PRD \
                     resolved q66)",
                    entry.name.value
                )),
            );
        }
    }
}

/// The harnesses this release lowers, by keyword — the keys `harnesses:` takes.
fn shipping() -> Vec<&'static str> {
    Harness::ALL
        .iter()
        .filter(|harness| harness.ships_in_v1())
        .map(|harness| harness.as_str())
        .collect()
}

/// The harness an entry's key names, if it is one this release lowers.
///
/// Rules 1 and 2 of the module header: a name the enum does not have is a typo
/// and is answered with a suggestion, and a name it has and does not lower is a
/// scope statement and is answered with `unsupported-harness`, exactly as the
/// same two spellings are on a node's `harness:` (Decision D136, PRD resolved
/// q57).
fn lowered(ctx: &mut Ctx, entry: &HarnessSdk) -> Option<Harness> {
    let name = entry.name.value.as_str();
    let shipping = shipping();
    let Some(harness) = Harness::from_keyword(name) else {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownVariant,
                entry.name.span.clone(),
                format!("`{name}` is not a valid harness"),
            )
            .with_help(suggest(name, &shipping).map_or_else(
                || {
                    format!(
                        "`harnesses:` is keyed by a harness this release lowers — {} — spelled \
                         as a `coder:` node's `harness:` spells it (grammar 14.8, Decision D136)",
                        shipping
                            .iter()
                            .map(|name| format!("`{name}`"))
                            .collect::<Vec<_>>()
                            .join(" or ")
                    )
                },
                |near| format!("did you mean `{near}`?"),
            )),
        );
        return None;
    };
    if harness.ships_in_v1() {
        return Some(harness);
    }
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnsupportedHarness,
            entry.name.span.clone(),
            format!(
                "`harnesses.{name}` is reserved: this release lowers {} and no other",
                list(&shipping)
            ),
        )
        .with_help(
            "a reserved harness has no driver, so no project installs an SDK for it and there is \
             nothing here to pin: key the entry by a harness this release lowers, or drop it; the \
             set grows by a resolved question rather than by a release adding a name (grammar \
             14.8, PRD resolved q57, q66)",
        ),
    );
    None
}

/// Rules 3 to 5 of the module header: the version is a literal, an exact one,
/// and inside `harness`'s audited range. Answers whether it passed all three.
fn version_is_audited(ctx: &mut Ctx, entry: &HarnessSdk, harness: Harness) -> bool {
    let name = entry.name.value.as_str();
    let version = &entry.sdk_version;
    let range = audited_range(harness).expect("a harness this release lowers has an audited range");
    let floor = range.floor;

    if let Some(reference) = env_references(&version.value).into_iter().next() {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnexpectedEnvRef,
                version.span.clone(),
                format!(
                    "`harnesses.{name}.sdk_version` never interpolates environment references, but \
                     it contains `{reference}`"
                ),
            )
            .with_help(format!(
                "a version is a build fact, not a secret: `build` writes it into `package.json` \
                 and `validate` holds it to the audited range `{range}`, so it is the literal \
                 version itself — write it out (grammar 14.8, §4.3)"
            )),
        );
        return false;
    }

    let Some(placement) = placement_of(harness, &version.value) else {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                version.span.clone(),
                format!(
                    "`harnesses.{name}.sdk_version` is `{}`, which is not an exact version",
                    version.value
                ),
            )
            .with_help(format!(
                "the range is this compiler's to declare and not a deployment's, so a version here \
                 is one exact `MAJOR.MINOR.PATCH` — no `^`, `~`, `>`, `<`, `*` or `x`, no dist-tag, \
                 and no `git`, `file`, `npm` or `workspace` specifier — inside the audited range \
                 `{range}` (grammar 14.8, PRD 5.12, resolved q66)"
            )),
        );
        return false;
    };

    let help = match placement {
        RangePlacement::Inside => return true,
        RangePlacement::Above => format!(
            "past the range, an option the SDK has added since may reach around a bound the node \
             states — the reserved `settings:` list is an inventory of the audited release's \
             option surface, and nothing has read this one's: pin a version inside `{range}`, or \
             drop the entry to install this release's pin, `{}`; reaching further is a compiler \
             release that re-audits, never a deploy key (grammar 14.8, PRD resolved q60, q66)",
            version_of(harness)
        ),
        RangePlacement::Below => format!(
            "an older SDK is not the release the driver's contracts were verified against — its \
             permission modes, its connection surface, the warning a shadowed permission callback \
             raises — and the reason to go backward, a CLI that works, is a reason to move the \
             floor in a compiler release rather than a deployment's to take: pin a version inside \
             `{range}`, or drop the entry to install this release's pin, `{}` (grammar 14.8, PRD \
             resolved q66)",
            version_of(harness)
        ),
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::HarnessSdkOutsideAuditedRange,
            version.span.clone(),
            format!(
                "`harnesses.{name}` asks for `{}` `{}`, outside this release's audited range \
                 `{range}` for `{name}`: its reserved-list audit was performed at `{floor}`, and \
                 that audit is the boundary",
                package_of(harness),
                version.value
            ),
        )
        .with_help(help),
    );
    false
}
