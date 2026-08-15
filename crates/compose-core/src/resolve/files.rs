//! Finding the composition: the entrypoint, its imports, and the active
//! target's deploy file (grammar 1.2, 1.4, 14).
//!
//! Every file is read under its **project-relative** name — the project root is
//! the entrypoint's own directory — and that name is what every span in the
//! composition, in a diagnostic, and in the IR carries. Grammar 1.4 asks for
//! exactly that: one portable spelling, identical in the IR, on a command line,
//! and in a diagnostic on every host. An absolute path would also make the
//! artifact reproduce differently on two machines, against PRD 5.12.
//!
//! Reading is done here rather than through [`parse_file`](crate::parse_file)
//! for one reason: an import that cannot be read is a mistake in the
//! **entrypoint**, so its diagnostic belongs on the `imports:` entry that named
//! it, not at the top of a file that does not exist.

use std::path::{Path, PathBuf};

use crate::ast::document::{DeployFile, Document, SpecFile};
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, SourceName, Span};
use crate::ir::SourceRole;
use crate::parse::parse_str;

/// Every file of one composition, in canonical order.
pub(crate) struct Composition {
    /// The entrypoint's project-relative name.
    pub(crate) entrypoint: String,
    /// The target this composition was resolved for.
    pub(crate) target: String,
    /// The spec files: the entrypoint, then its imports sorted by path.
    pub(crate) files: Vec<SpecSource>,
    /// The active target's deploy file, when there is one.
    pub(crate) deploy: Option<DeploySource>,
    /// Whether every `imports:` entry contributed a spec file.
    ///
    /// When one did not — it named a file that is not there, is not readable,
    /// is outside the project root, or is a deploy file — the composition is
    /// **incomplete**, and every definition that file would have declared is
    /// missing along with it. Resolving names against what is left would then
    /// report one diagnostic per reference into it, which is a page of
    /// consequences for one cause. The cause has already been reported, so the
    /// consequences are not.
    pub(crate) complete: bool,
}

impl Composition {
    /// The entrypoint's parsed document.
    pub(crate) fn entry(&self) -> Option<&SpecSource> {
        self.files.first()
    }
}

/// One parsed spec file with the name it was read under.
pub(crate) struct SpecSource {
    /// The project-relative name.
    pub(crate) name: String,
    /// How it entered the composition.
    pub(crate) role: SourceRole,
    /// The parsed document.
    pub(crate) file: SpecFile,
    /// Whether the parser rejected anything in this file. A file that did not
    /// parse cleanly has had its say; the resolver's own file-level rules stay
    /// quiet about it rather than reporting a second diagnostic for one mistake.
    pub(crate) clean: bool,
}

/// The active target's parsed deploy file with the name it was read under.
pub(crate) struct DeploySource {
    /// The project-relative name, always `deploy/<target>.yml`.
    pub(crate) name: String,
    /// The parsed document. A file carrying nothing but `version:` parses as a
    /// spec file with no sections, and is read here as an empty deploy layer.
    pub(crate) file: DeployFile,
}

/// Load the entrypoint, everything it imports, and the target's deploy file.
///
/// Returns `None` only when the entrypoint itself could not be read or is not a
/// spec file — there is no composition to resolve without one.
pub(crate) fn load(
    entrypoint: &Path,
    target: &str,
    diagnostics: &mut Diagnostics,
) -> Option<Composition> {
    let Some(name) = entrypoint.file_name().and_then(|name| name.to_str()) else {
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::IoError,
                Span::file_start(SourceName::new(entrypoint.display().to_string())),
                "the entrypoint does not name a file",
            )
            .with_help("`validate` takes the path of a spec file, conventionally `main.yml`"),
        );
        return None;
    };
    let root = entrypoint.parent().unwrap_or(Path::new(""));
    let name = name.to_string();

    let blame = Span::file_start(SourceName::new(&name));
    let Parsed { document, clean } = read(entrypoint, &name, &blame, None, diagnostics)?;
    let entry = match document {
        Document::Spec(file) => file,
        Document::Deploy(file) => {
            let at = deploy_section_span(&file).unwrap_or(blame);
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::MisplacedSection,
                    at,
                    format!("the entrypoint `{name}` is a deploy file"),
                )
                .with_help(
                    "spec files and deploy files are disjoint document kinds: a deploy file is selected with `--target <name>` and is never the entrypoint (grammar 1.2, 14)",
                ),
            );
            return None;
        }
    };

    let mut composition = Composition {
        entrypoint: name.clone(),
        target: target.to_string(),
        files: vec![SpecSource {
            name: name.clone(),
            role: SourceRole::Entrypoint,
            clean,
            file: entry,
        }],
        deploy: None,
        complete: true,
    };

    let declared = composition.files[0]
        .file
        .imports
        .as_ref()
        .map_or(0, |section| section.paths.len());
    imports(root, &name, &mut composition, diagnostics);
    composition.complete = composition.files.len() - 1 == declared;
    composition.files[1..].sort_by(|left, right| left.name.cmp(&right.name));
    composition.deploy = deploy(root, target, &name, diagnostics);
    Some(composition)
}

/// Follow the entrypoint's `imports:` (grammar 1.4).
fn imports(
    root: &Path,
    entrypoint: &str,
    composition: &mut Composition,
    diagnostics: &mut Diagnostics,
) {
    let entries: Vec<(String, Span)> = composition.files[0]
        .file
        .imports
        .iter()
        .flat_map(|section| section.paths.iter())
        .map(|path| (path.value.as_str().to_string(), path.span.clone()))
        .collect();

    let mut seen: Vec<(String, Span)> = Vec::new();
    for (written, span) in entries {
        let Some(normalized) = normalize(&written) else {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    span,
                    format!("`{written}` resolves outside the project root"),
                )
                .with_help(
                    "a path may contain `..`, but every file of a composition sits inside the project root — the entrypoint's own directory — so an entry may not climb out of it (grammar 1.4)",
                ),
            );
            continue;
        };
        if normalized == entrypoint {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    span,
                    format!("`{written}` imports the entrypoint itself"),
                )
                .with_help(
                    "the entrypoint is already part of the composition; an import list names the *other* files (grammar 1.4)",
                ),
            );
            continue;
        }
        if normalized
            .strip_prefix("deploy/")
            .is_some_and(|rest| !rest.is_empty())
        {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    span,
                    format!("`{written}` is a deploy file and may not be imported"),
                )
                .with_help(
                    "deploy files are selected with `--target <name>`, which loads `deploy/<name>.yml`; only that layer forks per environment (grammar 14)",
                ),
            );
            continue;
        }
        if let Some((_, first)) = seen.iter().find(|(other, _)| *other == normalized) {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    span,
                    format!("`{written}` names a file that is already imported"),
                )
                .with_label(first.clone(), "first imported here")
                .with_help(format!(
                    "both entries resolve to `{normalized}`, and import order does not affect semantics, so the second adds nothing: drop it",
                )),
            );
            continue;
        }
        seen.push((normalized.clone(), span.clone()));

        let path = join(root, &normalized);
        let Some(Parsed { document, clean }) = read(
            &path,
            &normalized,
            &span,
            Some(
                "imports are relative paths resolved against the entrypoint's directory, and there is no directory scanning: the file has to be there (grammar 1.4)",
            ),
            diagnostics,
        ) else {
            continue;
        };
        match document {
            Document::Spec(file) => {
                if let Some(section) = file.imports.as_ref() {
                    diagnostics.push(
                        Diagnostic::error(
                            DiagnosticCode::MisplacedSection,
                            section.span.clone(),
                            format!("`{normalized}` is imported and may not declare `imports:`"),
                        )
                        .with_label(span.clone(), "imported here")
                        .with_help(
                            "imports are not transitive: the entrypoint's list is the whole answer to what is in this graph, which is what keeps it greppable (grammar 1.4, Decision D1)",
                        ),
                    );
                }
                composition.files.push(SpecSource {
                    name: normalized,
                    role: SourceRole::Import,
                    clean,
                    file,
                });
            }
            Document::Deploy(file) => {
                let at = deploy_section_span(&file).unwrap_or_else(|| span.clone());
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::MisplacedSection,
                        at,
                        format!("`{normalized}` is a deploy file and may not be imported"),
                    )
                    .with_label(span.clone(), "imported here")
                    .with_help(
                        "deploy files are selected with `--target <name>`; only that layer forks per environment (grammar 1.2, 14)",
                    ),
                );
            }
        }
    }
}

/// Load `deploy/<target>.yml` (grammar 14, Decision D87).
fn deploy(
    root: &Path,
    target: &str,
    entrypoint: &str,
    diagnostics: &mut Diagnostics,
) -> Option<DeploySource> {
    if !is_portable_target(target) {
        // The target comes from the command line rather than from a file, so
        // there is no `deploy/<target>.yml` to point at — the name never named
        // one. The entrypoint is the composition the command was about, which
        // is the nearest thing to a site this diagnostic has.
        diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                Span::file_start(SourceName::new(entrypoint)),
                format!("`{target}` is not a target name"),
            )
            .with_help(
                "a target names one file, `deploy/<target>.yml`, so it is a single path segment: it starts with a letter, a digit, or `_`, and carries only letters, digits, `_`, `.`, and `-` (grammar 1.4)",
            ),
        );
        return None;
    }

    let name = format!("deploy/{target}.yml");
    let path = join(root, &name);
    let blame = Span::file_start(SourceName::new(&name));
    if !path.exists() {
        // `local` is built in: it needs no deploy file, and it substitutes local
        // storage for every store unconditionally. Every other target is a name
        // the author chose, so a missing file is a typo rather than a
        // fall-back to built-ins (Decision D87).
        if target != super::DEFAULT_TARGET {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::IoError,
                    blame,
                    format!("`--target {target}` requires `{name}`, which does not exist"),
                )
                .with_help(
                    "a named target's deploy file is required: resolving a missing one to built-in backends would deploy against the wrong infrastructure with no diagnostic (grammar 14, Decision D87)",
                ),
            );
        }
        return None;
    }

    let Parsed { document, .. } = read(&path, &name, &blame, None, diagnostics)?;
    match document {
        Document::Deploy(file) => Some(DeploySource { name, file }),
        // A file carrying nothing but `version:` has no section to decide its
        // kind by, so the parser reads it as a spec file. Selected as a deploy
        // file it is an empty deploy layer — unless it carries spec content, in
        // which case it is in the wrong file and says so.
        Document::Spec(file) => {
            let mut misplaced = |span: &Span, what: String| {
                diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::MisplacedSection,
                        span.clone(),
                        format!("{what} may not appear in the deploy file `{name}`"),
                    )
                    .with_help(
                        "spec files and deploy files are disjoint: only the deploy layer forks per environment (grammar 1.2, Decision D3)",
                    ),
                );
            };
            if let Some(section) = file.imports.as_ref() {
                misplaced(&section.span, "the `imports` section".to_string());
            }
            if let Some(section) = file.defaults.as_ref() {
                misplaced(&section.span, "the `defaults` section".to_string());
            }
            if let Some(section) = file.state.as_ref() {
                misplaced(&section.span, "the `state` section".to_string());
            }
            if let Some(section) = file.triggers.as_ref() {
                misplaced(&section.span, "the `triggers` section".to_string());
            }
            for definition in &file.definitions {
                misplaced(
                    &definition.address.span,
                    format!("`{}` is a definition and", definition.address.value),
                );
            }
            Some(DeploySource {
                name,
                file: DeployFile {
                    source: file.source,
                    version: file.version,
                    placements: None,
                    storage_backends: None,
                    event_sources: None,
                    span: file.span,
                },
            })
        }
    }
}

/// One parsed file, with whether the parser accepted it.
struct Parsed {
    document: Document,
    clean: bool,
}

/// Read and parse one file under its project-relative name.
fn read(
    path: &Path,
    name: &str,
    blame: &Span,
    unreadable_help: Option<&str>,
    diagnostics: &mut Diagnostics,
) -> Option<Parsed> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::IoError,
                    blame.clone(),
                    format!("cannot read `{name}`: {error}"),
                )
                .with_optional_help(unreadable_help.map(str::to_string)),
            );
            return None;
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidEncoding,
                    blame.clone(),
                    format!(
                        "`{name}` is not valid UTF-8: invalid byte at offset {}",
                        error.utf8_error().valid_up_to()
                    ),
                )
                .with_help("spec files are UTF-8 without a byte-order mark"),
            );
            return None;
        }
    };
    let parsed = parse_str(&text, name);
    let clean = !parsed.has_errors();
    diagnostics.extend(parsed.diagnostics);
    parsed.document.map(|document| Parsed { document, clean })
}

/// Normalize a `/`-separated relative path lexically, or `None` when it climbs
/// out of the project root.
///
/// Lexical rather than filesystem-based on purpose: resolving symlinks would
/// make the composition depend on the machine it is resolved on, which PRD 5.12
/// forbids. The parser has already fixed the charset (grammar 1.4), so the only
/// segments here are `.`, `..`, and portable names.
fn normalize(path: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                segments.pop()?;
            }
            name => segments.push(name),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

/// Join a normalized project-relative name onto the project root.
fn join(root: &Path, name: &str) -> PathBuf {
    let mut path = root.to_path_buf();
    for segment in name.split('/') {
        path.push(segment);
    }
    path
}

/// Whether a target names one path segment under `deploy/`.
fn is_portable_target(target: &str) -> bool {
    if target == "." || target == ".." {
        return false;
    }
    let mut bytes = target.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// The span of whichever deploy section decided a file's kind, for the
/// diagnostics that have to point at the evidence.
fn deploy_section_span(file: &DeployFile) -> Option<Span> {
    file.placements
        .as_ref()
        .map(|section| section.span.clone())
        .or_else(|| {
            file.storage_backends
                .as_ref()
                .map(|section| section.span.clone())
        })
        .or_else(|| {
            file.event_sources
                .as_ref()
                .map(|section| section.span.clone())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_normalize_lexically() {
        assert_eq!(
            normalize("agents/reviewer.yml").as_deref(),
            Some("agents/reviewer.yml")
        );
        assert_eq!(normalize("./models.yml").as_deref(), Some("models.yml"));
        assert_eq!(normalize("a/./b.yml").as_deref(), Some("a/b.yml"));
        assert_eq!(normalize("a/../b.yml").as_deref(), Some("b.yml"));
        assert_eq!(normalize("a/b/../../c.yml").as_deref(), Some("c.yml"));
        assert_eq!(normalize("../outside.yml"), None);
        assert_eq!(normalize("a/../../outside.yml"), None);
    }

    #[test]
    fn target_names_are_one_portable_segment() {
        assert!(is_portable_target("local"));
        assert!(is_portable_target("staging"));
        assert!(is_portable_target("eu-west-1"));
        assert!(!is_portable_target(""));
        assert!(!is_portable_target(".."));
        assert!(!is_portable_target("a/b"));
        assert!(!is_portable_target("-leading"));
    }
}
