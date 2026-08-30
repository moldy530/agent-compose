//! The rules that depend on which target is being resolved (grammar 11.3,
//! 13.5, 14).
//!
//! Three of the five are about a name binding to the deploy layer, the fourth
//! is about what the name it binds to can actually *do*, and the fifth is about
//! a section that may not be there at all. All five key off the **active**
//! target, so one composition is legal under `--target local` and rejected
//! under `--target staging` — which is the point: only this layer forks per
//! environment (PRD 5.8).
//!
//! `local` is built in and exempt from every rule that reads the deploy layer's
//! bindings. It substitutes SQLite/local disk for every store unconditionally,
//! so it resolves no alias — and therefore can neither fail to find one nor
//! find one that serves the wrong kind — and it runs no consumer process, so it
//! binds no event source. That is what lets a project with production
//! infrastructure in `deploy/staging.yml` still validate and run with nothing
//! installed — PRD 5.8's zero-infra guarantee (Decision D87).

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::{Ident, Namespace};
use crate::ast::definition::{Definition, DefinitionBody, StoreDef};
use crate::ast::deploy::{BackendAlias, BackendProvider};
use crate::ast::trigger::TriggerKind;
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Spanned};
use crate::parse::reader::{list, suggest};

use super::files::Composition;
use super::index::Index;
use super::references::Cx;

/// Namespaces a placement `members:` entry accepts (grammar 14.1).
///
/// `flow.*` is refused by the parser with a message naming the deferral
/// (Decision D129), so nothing that reaches here can be one — but the list
/// stated for `Cx::address` is what a *resolution* failure's message reads
/// from, and it must name the namespaces this position really accepts.
const COMPONENTS: &[Namespace] = &[Namespace::Agent, Namespace::Tool];

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
            for member in &placement.members {
                cx.address(member, COMPONENTS);
            }
        }
    }

    if local {
        return;
    }

    let aliases: BTreeMap<&str, &BackendAlias> = deploy
        .file
        .storage_backends
        .iter()
        .flat_map(|section| section.aliases.iter())
        .map(|alias| (alias.name.value.as_str(), alias))
        .collect();
    let defined: BTreeSet<&str> = aliases.keys().copied().collect();
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
        match aliases.get(backend.value.as_str()) {
            Some(alias) => capability(
                declared.definition,
                store,
                backend,
                alias,
                &composition.target,
                &deploy.name,
                diagnostics,
            ),
            None => bind(
                backend,
                &defined,
                &composition.target,
                &deploy.name,
                "storage backend alias",
                "`storage_backends.aliases:`",
                diagnostics,
            ),
        }
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

/// Check one store-to-alias binding against grammar 14.3's capability rule.
///
/// The rule is one line — a `vector` store bound to a non-vector-capable
/// provider is a compile error (PRD 5.8) — and it has two halves, because a
/// backend config is reached two ways. The parser owns the per-kind
/// `defaults:` half: there the store kind is the key the config sits under, so
/// the rule is decidable from the deploy file alone. This is the other half.
/// An alias declares no kind at all — it is an abstract slot, and which kind it
/// has to serve is whatever the stores that name it are — so the comparison
/// needs the composition and the active target in hand, which is exactly what
/// this pass has.
///
/// The two halves report the same fact from opposite ends, and each is anchored
/// where its author can act: the parser's at the provider it was handed, this
/// one at the `backend:` that chose the alias, with the alias definition
/// labelled so the reader sees both files at once.
fn capability(
    definition: &Definition,
    store: &StoreDef,
    backend: &Spanned<Ident>,
    alias: &BackendAlias,
    target: &str,
    file: &str,
    diagnostics: &mut Diagnostics,
) {
    // A store whose `kind:` the parser could not read, or an alias whose
    // `provider:` it could not, has already been reported. There is nothing
    // left to compare, and a capability diagnostic here would be a second one
    // for a mistake that has been named.
    let (Some(kind), Some(provider)) = (store.kind.as_ref(), alias.config.provider.as_ref()) else {
        return;
    };
    if provider.value.kind() == kind.value {
        return;
    }
    let accepted: Vec<&str> = BackendProvider::ALL
        .iter()
        .filter(|candidate| candidate.kind() == kind.value)
        .map(|candidate| candidate.as_str())
        .collect();
    diagnostics.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            backend.span.clone(),
            format!(
                "the `{target}` target binds `{}` to `{}`, which is not a `{}` storage provider",
                alias.name.value,
                provider.value.as_str(),
                kind.value.as_str()
            ),
        )
        .with_label(
            alias.name.span.clone(),
            format!("`{}` is defined here, in `{file}`", alias.name.value),
        )
        .with_help(format!(
            "`{}` declares `kind: {}`, and a backend serves exactly one kind: the `{}` providers are {} (grammar 14.3)",
            definition.address.value,
            kind.value.as_str(),
            kind.value.as_str(),
            list(&accepted)
        )),
    );
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
