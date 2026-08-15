//! The rules that depend on which target is being resolved (grammar 11.3,
//! 13.5, 14).
//!
//! Three of the four are about a name binding to the deploy layer, and the
//! fourth is about a section that may not be there at all. All four key off the
//! **active** target, so one composition is legal under `--target local` and
//! rejected under `--target staging` — which is the point: only this layer forks
//! per environment (PRD 5.8).
//!
//! `local` is built in and exempt from both binding checks. It substitutes
//! SQLite/local disk for every store unconditionally, so it resolves no alias
//! and can therefore not fail to find one, and it runs no consumer process, so
//! it binds no event source. That is what lets a project with production
//! infrastructure in `deploy/staging.yml` still validate and run with nothing
//! installed — PRD 5.8's zero-infra guarantee (Decision D87).

use std::collections::BTreeSet;

use crate::ast::common::{Ident, Namespace};
use crate::ast::definition::DefinitionBody;
use crate::ast::trigger::TriggerKind;
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Spanned};
use crate::parse::reader::{list, suggest};

use super::files::Composition;
use super::index::Index;
use super::references::Cx;

/// Namespaces a placement key accepts (grammar 14.1).
const COMPONENTS: &[Namespace] = &[Namespace::Agent, Namespace::Tool, Namespace::Flow];

/// Check the deploy layer against the composition it deploys.
pub(crate) fn check(composition: &Composition, index: &Index<'_>, diagnostics: &mut Diagnostics) {
    let local = composition.target == super::DEFAULT_TARGET;
    let Some(deploy) = composition.deploy.as_ref() else {
        // Either the target is `local` and has no deploy file — the zero-config
        // path — or a named target's file is missing, which has already been
        // reported. Neither leaves an alias or a source to resolve against, and
        // under `local` neither is consulted anyway.
        return;
    };

    if local && let Some(section) = deploy.file.storage_backends.as_ref() {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::MisplacedSection,
                section.span.clone(),
                format!("`{}` may not declare `storage_backends:`", deploy.name),
            )
            .with_help(
                "`local` substitutes local storage for every store unconditionally, so no alias and no per-kind default is ever consulted there: the block could only be a key whose author expected a substitution that never happens (grammar 14, Decision D87)",
            ),
        );
    }

    if let Some(section) = deploy.file.placements.as_ref() {
        let mut cx = Cx { index, diagnostics };
        for placement in &section.placements {
            cx.address(&placement.address, COMPONENTS);
        }
    }

    if local {
        return;
    }

    let aliases: BTreeSet<&str> = deploy
        .file
        .storage_backends
        .iter()
        .flat_map(|section| section.aliases.iter())
        .map(|alias| alias.name.value.as_str())
        .collect();
    for declared in index.definitions.values() {
        let DefinitionBody::Store(store) = &declared.definition.body else {
            continue;
        };
        let Some(backend) = store.backend.as_ref() else {
            // No alias: the per-kind `defaults:` and then the target's built-in
            // answer for this store instead, and neither can be undefined
            // (grammar 11.3).
            continue;
        };
        bind(
            backend,
            &aliases,
            &composition.target,
            &deploy.name,
            "storage backend alias",
            "`storage_backends.aliases:`",
            diagnostics,
        );
    }

    let sources: BTreeSet<&str> = deploy
        .file
        .event_sources
        .iter()
        .flat_map(|section| section.sources.iter())
        .map(|source| source.name.value.as_str())
        .collect();
    for trigger in index
        .triggers
        .iter()
        .flat_map(|section| section.value.triggers.iter())
    {
        let Some(TriggerKind::Event(event)) = trigger.kind.as_ref() else {
            continue;
        };
        let Some(source) = event.source.as_ref() else {
            continue;
        };
        bind(
            source,
            &sources,
            &composition.target,
            &deploy.name,
            "event source",
            "`event_sources:`",
            diagnostics,
        );
    }
}

/// Resolve one logical name against what the active target defines.
fn bind(
    reference: &Spanned<Ident>,
    defined: &BTreeSet<&str>,
    target: &str,
    file: &str,
    what: &str,
    section: &str,
    diagnostics: &mut Diagnostics,
) {
    if defined.contains(reference.value.as_str()) {
        return;
    }
    let candidates: Vec<&str> = defined.iter().copied().collect();
    let help = suggest(reference.value.as_str(), &candidates)
        .map(|candidate| format!("did you mean `{candidate}`?"))
        .unwrap_or_else(|| {
            if candidates.is_empty() {
                format!("define it in `{file}` under {section}")
            } else {
                format!("`{file}` defines {} under {section}", list(&candidates))
            }
        });
    diagnostics.push(
        Diagnostic::error(
            DiagnosticCode::UndefinedReference,
            reference.span.clone(),
            format!(
                "the `{target}` target defines no {what} `{}`",
                reference.value
            ),
        )
        .with_help(help),
    );
}
