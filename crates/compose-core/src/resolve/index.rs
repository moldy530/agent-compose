//! The composition's name table: one entry per typed address, one site per
//! singleton section, and one authoritative spec version (grammar 1.3, 1.5,
//! 2.2).
//!
//! Both uniqueness rules are stated the same way and reported the same way: the
//! **second** site carries the diagnostic and the **first** carries a label, so
//! a reader sees both places at once. Which of two sites is "first" is decided
//! by the composition's canonical file order — the entrypoint, then its imports
//! sorted by path — never by the order the entrypoint happens to list them in,
//! because import order does not affect semantics (grammar 1.4) and a diagnostic
//! that moved when an import list was reordered would say otherwise.

use std::collections::BTreeMap;

use crate::ast::common::{Address, Namespace};
use crate::ast::definition::Definition;
use crate::ast::document::StateSection;
use crate::ast::policy::PolicyBlock;
use crate::ast::trigger::TriggersSection;
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Span, Spanned};

use super::files::Composition;

/// One definition, with the file that declares it.
pub(crate) struct Declared<'a> {
    /// The project-relative name of the declaring file.
    pub(crate) file: &'a str,
    /// The definition itself.
    pub(crate) definition: &'a Definition,
}

/// A singleton section, with the file that declares it (grammar 1.5, D2).
pub(crate) struct Section<'a, T> {
    /// The project-relative name of the declaring file.
    pub(crate) file: &'a str,
    /// The section.
    pub(crate) value: &'a T,
}

/// Everything the composition declares, addressable by name.
pub(crate) struct Index<'a> {
    /// Every definition, by typed address. Sorted, which is also the order the
    /// IR writes them in (Decision D55).
    pub(crate) definitions: BTreeMap<String, Declared<'a>>,
    /// `defaults:` — at most one in the composition.
    pub(crate) defaults: Option<Section<'a, Spanned<PolicyBlock>>>,
    /// `state:` — at most one in the composition.
    pub(crate) state: Option<Section<'a, StateSection>>,
    /// `triggers:` — at most one in the composition.
    pub(crate) triggers: Option<Section<'a, TriggersSection>>,
    /// The composition's spec version, taken from the entrypoint.
    pub(crate) version: Option<String>,
}

impl<'a> Index<'a> {
    /// The definition at this address, if the composition declares one.
    pub(crate) fn get(&self, address: &Address) -> Option<&Declared<'a>> {
        self.definitions.get(&address.to_string())
    }

    /// Every defined address in these namespaces, for a "did you mean".
    pub(crate) fn addresses_in(&self, namespaces: &[Namespace]) -> Vec<&str> {
        self.definitions
            .iter()
            .filter(|(_, declared)| {
                namespaces.contains(&declared.definition.address.value.namespace)
            })
            .map(|(address, _)| address.as_str())
            .collect()
    }
}

/// Build the index, reporting the two uniqueness rules and the version rules.
pub(crate) fn build<'a>(composition: &'a Composition, diagnostics: &mut Diagnostics) -> Index<'a> {
    let mut index = Index {
        definitions: BTreeMap::new(),
        defaults: None,
        state: None,
        triggers: None,
        version: None,
    };

    version(composition, &mut index, diagnostics);

    for source in &composition.files {
        for definition in &source.file.definitions {
            let address = definition.address.value.to_string();
            match index.definitions.get(&address) {
                Some(first) => {
                    diagnostics.push(
                        Diagnostic::error(
                            DiagnosticCode::DuplicateDefinition,
                            definition.address.span.clone(),
                            format!("`{address}` is defined twice in this composition"),
                        )
                        .with_label(
                            first.definition.address.span.clone(),
                            format!("first defined here, in `{}`", first.file),
                        )
                        .with_help(
                            "a typed address is global across the composition, whichever file declares it: rename one of the two, or drop the file that duplicates the other (grammar 2.2)",
                        ),
                    );
                }
                None => {
                    index.definitions.insert(
                        address,
                        Declared {
                            file: &source.name,
                            definition,
                        },
                    );
                }
            }
        }

        singleton(
            &mut index.defaults,
            source.file.defaults.as_ref(),
            &source.name,
            "defaults",
            |section| section.span.clone(),
            diagnostics,
        );
        singleton(
            &mut index.state,
            source.file.state.as_ref(),
            &source.name,
            "state",
            |section| section.span.clone(),
            diagnostics,
        );
        singleton(
            &mut index.triggers,
            source.file.triggers.as_ref(),
            &source.name,
            "triggers",
            |section| section.span.clone(),
            diagnostics,
        );
    }

    index
}

/// Record a singleton section, reporting a second declaration (Decision D2).
fn singleton<'a, T>(
    slot: &mut Option<Section<'a, T>>,
    found: Option<&'a T>,
    file: &'a str,
    name: &str,
    span: impl Fn(&T) -> Span,
    diagnostics: &mut Diagnostics,
) {
    let Some(value) = found else { return };
    match slot {
        Some(first) => diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::DuplicateSection,
                span(value),
                format!("the `{name}` section is declared in two files"),
            )
            .with_label(
                span(first.value),
                format!("first declared here, in `{}`", first.file),
            )
            .with_help(format!(
                "`{name}` may appear in at most one file of a composition: merging two of them implicitly is the action at a distance the deploy layer already refuses (grammar 1.5, Decision D2)"
            )),
        ),
        None => *slot = Some(Section { file, value }),
    }
}

/// Decide the composition's version and check every other file against it
/// (grammar 1.3, Decision D4).
///
/// The mismatch half is dormant in this build and stays implemented anyway.
/// `SUPPORTED_SPEC_VERSIONS` has one member, and the parser refuses a value
/// outside it before the resolver sees the file — so every version that
/// reaches here is that one member, and no two of them can differ. It becomes
/// live the day a second version ships, which is exactly when a composition
/// assembled from files of two vintages becomes possible; the unit tests below
/// are what keep it honest until then.
fn version(composition: &Composition, index: &mut Index<'_>, diagnostics: &mut Diagnostics) {
    let Some(entry) = composition.entry() else {
        return;
    };
    let Some(declared) = entry.file.version.as_ref() else {
        // A file the parser rejected has had its say — including about a
        // `version:` it could not read — so a second diagnostic here would be
        // two for one mistake.
        if entry.clean {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::MissingKey,
                    entry.file.span.clone(),
                    format!("the entrypoint `{}` must declare `version:`", entry.name),
                )
                .with_help(format!(
                    "write `version: \"{}\"` — quoted, since an unquoted `0.1` is a YAML float; it is the one authoritative version of the composition (grammar 1.3)",
                    crate::SUPPORTED_SPEC_VERSIONS[0]
                )),
            );
        }
        return;
    };
    index.version = Some(declared.value.clone());

    // Every other file's `version:` is optional and redundant — a cheap check
    // for a file copied in from another project. Deploy files are held to the
    // same match: the composition has one authoritative version, and nothing in
    // the deploy layer is versioned separately from what it deploys.
    let others = composition
        .files
        .iter()
        .skip(1)
        .map(|source| (source.name.as_str(), source.file.version.as_ref()))
        .chain(
            composition
                .deploy
                .iter()
                .map(|source| (source.name.as_str(), source.file.version.as_ref())),
        );
    for (name, version) in others {
        let Some(version) = version else { continue };
        if version.value == declared.value {
            continue;
        }
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::VersionMismatch,
                version.span.clone(),
                format!(
                    "`{name}` declares version `{}`, but the composition is version `{}`",
                    version.value, declared.value
                ),
            )
            .with_label(
                declared.span.clone(),
                format!("the entrypoint `{}` declares it here", entry.name),
            )
            .with_help(
                "a file's `version:` is optional, and when present it must be byte-identical to the entrypoint's: one authoritative version per composition (grammar 1.3, Decision D4)",
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::document::SpecFile;
    use crate::diag::{Position, SourceName};
    use crate::ir::SourceRole;
    use crate::resolve::files::SpecSource;

    fn span(file: &str, line: u32) -> Span {
        Span::new(
            SourceName::new(file),
            0..0,
            Position::new(line, 1),
            Position::new(line, 8),
        )
    }

    /// A parsed file carrying nothing but the `version:` under test.
    fn file(name: &str, version: Option<&str>, role: SourceRole, clean: bool) -> SpecSource {
        SpecSource {
            name: name.to_string(),
            role,
            clean,
            file: SpecFile {
                source: SourceName::new(name),
                version: version.map(|value| Spanned::new(value.to_string(), span(name, 1))),
                imports: None,
                defaults: None,
                state: None,
                triggers: None,
                definitions: Vec::new(),
                span: span(name, 1),
            },
        }
    }

    fn composition(files: Vec<SpecSource>) -> crate::resolve::files::Composition {
        crate::resolve::files::Composition {
            entrypoint: files
                .first()
                .map(|file| file.name.clone())
                .unwrap_or_default(),
            target: crate::resolve::DEFAULT_TARGET.to_string(),
            files,
            deploy: None,
            complete: true,
        }
    }

    fn versions(files: Vec<SpecSource>) -> (Option<String>, Vec<Diagnostic>) {
        let composition = composition(files);
        let mut diagnostics = Diagnostics::new();
        let mut index = Index {
            definitions: BTreeMap::new(),
            defaults: None,
            state: None,
            triggers: None,
            version: None,
        };
        version(&composition, &mut index, &mut diagnostics);
        (index.version, diagnostics.into_vec())
    }

    #[test]
    fn the_entrypoints_version_is_the_compositions() {
        let (version, diagnostics) = versions(vec![
            file("main.yml", Some("0.1"), SourceRole::Entrypoint, true),
            file("models.yml", Some("0.1"), SourceRole::Import, true),
            file("agents.yml", None, SourceRole::Import, true),
        ]);
        assert_eq!(version.as_deref(), Some("0.1"));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }

    /// The dormant half of grammar 1.3: an imported file that declares another
    /// version is refused, naming both files. Unreachable through a fixture
    /// while the accepted set has one member, so it is pinned here.
    #[test]
    fn an_imported_files_version_must_match_the_entrypoints() {
        let (_, diagnostics) = versions(vec![
            file("main.yml", Some("0.1"), SourceRole::Entrypoint, true),
            file("models.yml", Some("0.2"), SourceRole::Import, true),
        ]);
        assert_eq!(diagnostics.len(), 1);
        let reported = &diagnostics[0];
        assert_eq!(reported.code, DiagnosticCode::VersionMismatch);
        assert_eq!(
            reported.message,
            "`models.yml` declares version `0.2`, but the composition is version `0.1`"
        );
        assert_eq!(reported.span.source.as_str(), "models.yml");
        assert_eq!(reported.labels.len(), 1);
        assert_eq!(reported.labels[0].span.source.as_str(), "main.yml");
        assert_eq!(
            reported.labels[0].message,
            "the entrypoint `main.yml` declares it here"
        );
    }

    #[test]
    fn an_entrypoint_with_no_version_is_refused() {
        let (version, diagnostics) =
            versions(vec![file("main.yml", None, SourceRole::Entrypoint, true)]);
        assert_eq!(version, None);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, DiagnosticCode::MissingKey);
        assert_eq!(
            diagnostics[0].message,
            "the entrypoint `main.yml` must declare `version:`"
        );
    }

    /// A file the parser rejected has had its say — including about a
    /// `version:` it could not read — so this pass stays quiet rather than
    /// reporting a second diagnostic for one mistake.
    #[test]
    fn an_entrypoint_that_did_not_parse_is_not_told_twice() {
        let (_, diagnostics) =
            versions(vec![file("main.yml", None, SourceRole::Entrypoint, false)]);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
    }
}
