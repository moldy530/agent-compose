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

#[cfg(test)]
mod tests {
    use crate::ast::DEPLOY_SECTIONS;
    use crate::prose::enumeration;

    /// The only deploy section `local` refuses, which is what [`super::check`]
    /// reports: `local` substitutes local storage for every store
    /// unconditionally, so the block could only be an inert key whose author
    /// expected a substitution that never happens (grammar 14, Decision D87).
    const REFUSED_UNDER_LOCAL: &[&str] = &["storage_backends"];

    /// `version:` belongs to both document kinds (grammar 1.5), so it is never
    /// what a "what `deploy/local.yml` may declare" sentence is enumerating.
    const SHARED_BY_BOTH_DOCUMENT_KINDS: &[&str] = &["version"];

    /// **What `deploy/local.yml` may declare is published in three places, and
    /// they must agree with the mechanism.**
    ///
    /// `local` admits every deploy section except the one [`super::check`]
    /// refuses, so [`DEPLOY_SECTIONS`] minus [`REFUSED_UNDER_LOCAL`] is the
    /// list — and three documents state it in prose: `docs/grammar.md` §14's
    /// `local` bullet, `docs/plan.md` §11, which is why a plan compares those
    /// sections and no others, and `agent-compose docs targets`, which is the
    /// copy an author on a laptop actually reads.
    ///
    /// A section made live under `local` and written into only two of them
    /// leaves the third telling that author the key they just wrote is not
    /// allowed in the file the grammar says it belongs in — error UX (PRD G3)
    /// failing where it is being read, and with nothing else in the suite
    /// disagreeing, because a topic doc is prose and prose does not compile.
    /// `package_registry:` (PRD resolved q59) went live under `local` in the
    /// grammar, in the plan document and in this module while the topic's list
    /// still named four sections.
    #[test]
    fn what_deploy_local_yml_may_declare_is_the_same_list_wherever_it_is_published() {
        // All three sentences wrap, so each document is read with its newlines
        // flattened: a list broken across lines is the same list.
        let grammar = include_str!("../../../../docs/grammar.md").replace('\n', " ");
        let plan = include_str!("../../../../docs/plan.md").replace('\n', " ");
        let targets = include_str!("../../../../docs/topics/targets.md").replace('\n', " ");
        let published: [(&str, &str); 3] = [
            (
                "docs/grammar.md §14",
                enumeration(
                    "docs/grammar.md",
                    &grammar,
                    "When present it MAY declare",
                    ".",
                ),
            ),
            (
                "docs/plan.md §11",
                enumeration("docs/plan.md", &plan, "admit —", "— are"),
            ),
            (
                "`agent-compose docs targets`",
                enumeration(
                    "docs/topics/targets.md",
                    &targets,
                    "When present it may declare",
                    ".",
                ),
            ),
        ];
        for (place, sentence) in published {
            for section in DEPLOY_SECTIONS {
                if SHARED_BY_BOTH_DOCUMENT_KINDS.contains(section) {
                    continue;
                }
                let named = sentence.contains(&format!("`{section}:`"));
                if REFUSED_UNDER_LOCAL.contains(section) {
                    assert!(
                        !named,
                        "{place} lists `{section}` among the sections `deploy/local.yml` may \
                         declare: `{sentence}`. `local` refuses that section — \
                         `resolve::target::check` reports `misplaced-section` for it — so an \
                         author who writes it because this sentence said to is refused by the \
                         very compiler the sentence is describing."
                    );
                } else {
                    assert!(
                        named,
                        "{place} does not name `{section}` among the sections \
                         `deploy/local.yml` may declare: `{sentence}`. `local` admits every \
                         deploy section but {REFUSED_UNDER_LOCAL:?}, and all three of \
                         `docs/grammar.md` §14, `docs/plan.md` §11 and `agent-compose docs \
                         targets` publish that list, so a reader of this one is told the key \
                         is unavailable on the target they develop on."
                    );
                }
            }
        }
    }
}
