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
//! for two reasons. An import that cannot be read is a mistake in the
//! **entrypoint**, so its diagnostic belongs on the `imports:` entry that named
//! it, not at the top of a file that does not exist. And this pass knows what
//! every file it reads *is*, so it tells the parser
//! ([`FileRole`](crate::parse::FileRole)) rather than leaving it to infer a
//! role from one file and assert something this pass can already disprove.

use std::path::{Path, PathBuf};

use crate::ast::document::{DeployFile, Document, SpecFile};
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, SourceName, Span};
use crate::ir::SourceRole;
use crate::parse::{FileRole, parse_as};

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
    let Parsed { document, clean } = read(
        entrypoint,
        &name,
        FileRole::Entrypoint,
        &Unreadable {
            // The path as the command line typed it, directory and all. Every
            // span that reaches the artifact is project-relative (grammar 1.4,
            // PRD 5.12) — but an entrypoint that could not be read produces no
            // artifact, so nothing is at stake in naming it the way the author
            // named it, and a bare file name never says which directory the
            // compiler looked in.
            at: Span::file_start(SourceName::new(entrypoint.display().to_string())),
            called: entrypoint.display().to_string(),
            help: None,
        },
        diagnostics,
    )?;
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
                    format!("`{written}` climbs out of the project root"),
                )
                .with_help(
                    "a path may contain `..`, but every file of a composition sits inside the project root — the entrypoint's own directory — so no part of an entry may climb out of it, not even one that climbs back in: whether `../<dir>/f.yml` came back would depend on what the checkout happens to be named, and a composition means the same thing on every host (grammar 1.4, PRD 5.12)",
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
            FileRole::Import,
            &Unreadable {
                at: span.clone(),
                called: normalized.clone(),
                help: Some(
                    "imports are relative paths resolved against the entrypoint's directory, and there is no directory scanning: the file has to be there (grammar 1.4)",
                ),
            },
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

    let Parsed { document, .. } = read(
        &path,
        &name,
        FileRole::Deploy,
        &Unreadable {
            at: blame,
            called: name.clone(),
            help: None,
        },
        diagnostics,
    )?;
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

/// Where a file that could not be read at all is reported, and what it is
/// called there.
///
/// Kept apart from the project-relative name for one reason: a file that never
/// opened contributes no span to the artifact, so this diagnostic is free to
/// name it however the reader will recognise it. An import is named at the
/// `imports:` entry that asked for it, under the path written there; the
/// entrypoint is named by the path the command line typed.
struct Unreadable<'a> {
    /// The span the diagnostic lands on.
    at: Span,
    /// What the diagnostic calls the file.
    called: String,
    /// Why it might not be there, when there is something to say.
    help: Option<&'a str>,
}

/// Read and parse one file under its project-relative name, in a known role.
///
/// `name` is what every span the file contributes carries, and `role` is what
/// this pass already knows the file to be (grammar 1.2, and see
/// [`FileRole`](crate::parse::FileRole)). `unreadable` is consulted only where
/// the file never opened, so it has neither spans nor a role to speak of.
fn read(
    path: &Path,
    name: &str,
    role: FileRole,
    unreadable: &Unreadable<'_>,
    diagnostics: &mut Diagnostics,
) -> Option<Parsed> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            diagnostics.push(
                Diagnostic::error(
                    DiagnosticCode::IoError,
                    unreadable.at.clone(),
                    format!("cannot read `{}`: {error}", unreadable.called),
                )
                .with_optional_help(unreadable.help.map(str::to_string)),
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
                    unreadable.at.clone(),
                    format!(
                        "`{}` is not valid UTF-8: invalid byte at offset {}",
                        unreadable.called,
                        error.utf8_error().valid_up_to()
                    ),
                )
                .with_help("spec files are UTF-8 without a byte-order mark"),
            );
            return None;
        }
    };
    let parsed = parse_as(&text, name, role);
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
///
/// The root is a floor rather than a fence: a `..` at position 0 fails the path
/// even when a later segment would climb back in. Deciding `../proj/f.yml`
/// would mean comparing `proj` against the name of the directory the project
/// was checked out into — so the same composition would resolve on one machine
/// and not on another, which is the thing grammar 1.4's one-portable-spelling
/// rule and PRD 5.12 both exist to prevent. It cannot even be asked uniformly:
/// an entrypoint given as a bare `main.yml` has `""` for a root, and `""` has
/// no name to compare against.
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

    /// A path that climbs out and back in is refused too: whether it landed
    /// inside would depend on the name of the directory the project was checked
    /// out into, and a bare `main.yml` entrypoint has no such name at all.
    #[test]
    fn a_path_that_climbs_out_and_back_in_is_still_out() {
        assert_eq!(normalize("../project/models.yml"), None);
        assert_eq!(normalize("a/../../a/models.yml"), None);
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
