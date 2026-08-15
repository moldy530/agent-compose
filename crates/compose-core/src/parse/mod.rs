//! The parser: one file in, a typed [`Document`] plus diagnostics out.
//!
//! # What the parser decides
//!
//! Everything the parser reports is decidable from a **single file**. That is
//! the same line `docs/grammar.md` Appendix B draws for the published JSON
//! Schema, and the parser is at least as strict as the schema everywhere: the
//! YAML profile (grammar 1.1), section placement (1.5), every construct's key
//! table, the closed vocabularies, the lexical forms of grammar 2 and 4, and
//! the structural rules whose deciding value is a literal in the same object —
//! a node's kind key, a trigger's `type:`, a store op's parameter row, the
//! direct-XOR-route split on models.
//!
//! What it deliberately leaves alone:
//!
//! * **`imports:` is a list of strings.** Following it is the resolver's job;
//!   the parser only rejects the shapes grammar 1.4 calls parse errors
//!   (absolute paths, URLs, globs, non-`.yml` files).
//! * **References are not resolved.** `model: model.smart` is checked for
//!   *namespace* — the grammar's "expected a `model.*` reference, found
//!   `tool.web_search`" — but whether `model.smart` exists is a cross-file
//!   question.
//! * **CEL stays raw.** Its roots depend on the surface and its types on the
//!   resolved schemas (grammar 4.1).
//! * **Nothing graph-shaped.** Exhaustiveness, SCC termination, fan-out
//!   bounding through a `map.over` path, reducer-write rules, reachability:
//!   all need the whole composition.
//!
//! # Recovery
//!
//! Parsing never panics and never stops at the first error. The YAML profile
//! violations that are recoverable — a duplicate key, a tag, a non-string key —
//! drop the offending entry and carry on; a malformed construct yields a
//! partial AST node so its siblings still parse. Only an unreadable file (bad
//! YAML syntax, no document, a non-mapping root) ends the pass, because there
//! is nothing left to walk.

pub(crate) mod binding;
pub(crate) mod definition;
pub(crate) mod deploy;
pub(crate) mod flow;
pub(crate) mod lexical;
pub(crate) mod policy;
pub(crate) mod reader;
pub(crate) mod schema;
pub(crate) mod section;

use std::path::Path;

use crate::SUPPORTED_SPEC_VERSIONS;
use crate::ast::document::{
    DEPLOY_SECTIONS, DeployFile, Document, DocumentKind, SPEC_SECTIONS, SpecFile,
};
use crate::ast::{Definition, Namespace};
use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, SourceName, Span, Spanned};
use crate::yaml::{self, Entry, Mapping, Node, Yaml};

use reader::{Cx, list, suggest};

/// The result of parsing one file.
#[derive(Clone, Debug)]
pub struct ParsedFile {
    /// The document, when the file held one that could be walked. Present even
    /// when there are errors: a partial tree is the normal outcome of a file
    /// with a bad node in it.
    pub document: Option<Document>,
    /// Everything the parser found, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

impl ParsedFile {
    /// Whether any diagnostic rejects the file.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// The parsed document, if there is one.
    #[must_use]
    pub const fn document(&self) -> Option<&Document> {
        self.document.as_ref()
    }

    /// Split into the document and its diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<Document>, Vec<Diagnostic>) {
        (self.document, self.diagnostics)
    }
}

/// Parse one file from disk.
///
/// An unreadable file or one that is not UTF-8 yields no document and a single
/// diagnostic; it never panics and never returns an `Err` the caller has to
/// translate into one.
pub fn parse_file(path: impl AsRef<Path>) -> ParsedFile {
    let path = path.as_ref();
    let source = SourceName::new(path.display().to_string());
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return ParsedFile {
                document: None,
                diagnostics: vec![Diagnostic::error(
                    DiagnosticCode::IoError,
                    Span::file_start(source),
                    format!("cannot read the file: {error}"),
                )],
            };
        }
    };
    match String::from_utf8(bytes) {
        Ok(text) => parse_str(&text, source),
        Err(error) => ParsedFile {
            document: None,
            diagnostics: vec![
                Diagnostic::error(
                    DiagnosticCode::InvalidEncoding,
                    Span::file_start(source),
                    format!(
                        "the file is not valid UTF-8: invalid byte at offset {}",
                        error.utf8_error().valid_up_to()
                    ),
                )
                .with_help("spec files are UTF-8 without a byte-order mark"),
            ],
        },
    }
}

/// Parse one file from memory, reporting diagnostics against `name`.
pub fn parse_str(source: &str, name: impl Into<SourceName>) -> ParsedFile {
    let name = name.into();
    let mut diagnostics = Diagnostics::new();
    let root = yaml::load(source, &name, &mut diagnostics);
    let document = root.and_then(|root| {
        let mut cx = Cx::new(&mut diagnostics);
        document(&root, name, &mut cx)
    });
    diagnostics.sort();
    ParsedFile {
        document,
        diagnostics: diagnostics.into_vec(),
    }
}

/// Sections that only a deploy file may carry (grammar 1.5).
const DEPLOY_ONLY: &[&str] = &["placements", "storage_backends", "event_sources"];
/// Sections that only a spec file may carry (grammar 1.5).
const SPEC_ONLY: &[&str] = &["imports", "defaults", "state", "triggers"];

fn document(root: &Node, source: SourceName, cx: &mut Cx) -> Option<Document> {
    let mapping = root.as_mapping()?;

    // A file is a deploy file when it carries a deploy section and no section
    // that only a spec file may carry. *Sections* decide the kind, not
    // definitions: that way a `main.yml` which grew a `placements:` block is
    // reported as a spec file with one misplaced section, while a file that is
    // plainly a deploy target with a definition pasted into it is reported the
    // other way round (grammar 1.2, Decision D3).
    let spec_evidence = mapping
        .entries()
        .iter()
        .find(|entry| SPEC_ONLY.contains(&entry.key.value.as_str()));
    let has_deploy = mapping
        .entries()
        .iter()
        .any(|entry| DEPLOY_ONLY.contains(&entry.key.value.as_str()));

    if has_deploy && spec_evidence.is_none() {
        Some(Document::Deploy(deploy_file(mapping, root, source, cx)))
    } else {
        Some(Document::Spec(spec_file(
            mapping,
            root,
            source,
            spec_evidence,
            cx,
        )))
    }
}

fn is_definition_key(key: &str) -> bool {
    key.split_once('.')
        .and_then(|(prefix, _)| Namespace::from_prefix(prefix))
        .is_some()
}

fn spec_file(
    mapping: &Mapping,
    root: &Node,
    source: SourceName,
    spec_evidence: Option<&Entry>,
    cx: &mut Cx,
) -> SpecFile {
    let mut file = SpecFile {
        source,
        version: None,
        imports: None,
        defaults: None,
        state: None,
        triggers: None,
        definitions: Vec::new(),
        span: root.span.clone(),
    };

    for entry in mapping.entries() {
        let key = entry.key.value.as_str();
        match key {
            "version" => file.version = version(&entry.value, cx),
            "imports" => file.imports = section::imports(&entry.value, cx),
            "defaults" => {
                file.defaults = policy::policy_block(&entry.value, "the `defaults` section", cx);
            }
            "state" => file.state = section::state(&entry.value, cx),
            "triggers" => file.triggers = section::triggers(&entry.value, cx),
            _ if DEPLOY_ONLY.contains(&key) => {
                misplaced(entry, DocumentKind::Deploy, spec_evidence, cx);
            }
            _ => {
                if let Some(definition) = definition(entry, cx) {
                    file.definitions.push(definition);
                }
            }
        }
    }

    // The entrypoint is the only file that may declare `imports:`, and the
    // entrypoint must declare `version:` — the best per-file approximation of
    // Decision D4's rule (the resolver knows which file is the entrypoint).
    // A `version:` that was declared but rejected has already been reported;
    // saying it is missing as well would be two diagnostics for one mistake.
    if !mapping.contains_key("version")
        && let Some(imports) = mapping.entry("imports")
    {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                imports.key.span.clone(),
                "a file that declares `imports:` is the entrypoint and must declare `version:`",
            )
            .with_help(format!(
                "write `version: \"{}\"` — quoted, since an unquoted `0.1` is a YAML float",
                SUPPORTED_SPEC_VERSIONS[0]
            )),
        );
    }

    file
}

fn deploy_file(mapping: &Mapping, root: &Node, source: SourceName, cx: &mut Cx) -> DeployFile {
    let mut file = DeployFile {
        source,
        version: None,
        placements: None,
        storage_backends: None,
        event_sources: None,
        span: root.span.clone(),
    };

    for entry in mapping.entries() {
        let key = entry.key.value.as_str();
        match key {
            "version" => file.version = version(&entry.value, cx),
            "placements" => file.placements = deploy::placements(&entry.value, cx),
            "storage_backends" => {
                file.storage_backends = deploy::storage_backends(&entry.value, cx);
            }
            "event_sources" => file.event_sources = deploy::event_sources(&entry.value, cx),
            _ if SPEC_ONLY.contains(&key) || is_definition_key(key) => {
                misplaced(entry, DocumentKind::Spec, None, cx);
            }
            _ => unknown_top_level_key(entry, DocumentKind::Deploy, cx),
        }
    }

    if !mapping.contains_key("version") {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                root.span.clone(),
                "a deploy file must declare `version:`",
            )
            .with_help(format!(
                "write `version: \"{}\"` — quoted, since an unquoted `0.1` is a YAML float",
                SUPPORTED_SPEC_VERSIONS[0]
            )),
        );
    }

    file
}

/// A section that belongs to the other document kind (grammar 1.2, 1.5).
fn misplaced(entry: &Entry, belongs_to: DocumentKind, evidence: Option<&Entry>, cx: &mut Cx) {
    let what = if is_definition_key(&entry.key.value) {
        format!("`{}` is a definition and", entry.key.value)
    } else {
        format!("the `{}` section", entry.key.value)
    };
    let mut diagnostic = Diagnostic::error(
        DiagnosticCode::MisplacedSection,
        entry.key.span.clone(),
        format!("{what} may only appear in a {}", belongs_to.as_str()),
    )
    .with_help(
        "spec files and deploy files are disjoint: only the deploy layer forks per environment, and `deploy/<target>.yml` is selected with `--target`",
    );
    if let Some(evidence) = evidence {
        diagnostic = diagnostic.with_label(
            evidence.key.span.clone(),
            format!("`{}` here makes this a spec file", evidence.key.value),
        );
    }
    cx.push(diagnostic);
}

fn unknown_top_level_key(entry: &Entry, kind: DocumentKind, cx: &mut Cx) {
    let key = entry.key.value.as_str();
    let sections: &[&str] = match kind {
        DocumentKind::Spec => SPEC_SECTIONS,
        DocumentKind::Deploy => DEPLOY_SECTIONS,
    };

    // `agnt.reviewer` is a misspelled namespace, not an unknown section.
    let help = match key.split_once('.') {
        Some((prefix, name)) if !name.is_empty() => definition::suggest_namespace(prefix)
            .map(|namespace| format!("did you mean `{namespace}.{name}`?"))
            .or_else(|| {
                Some(format!(
                    "definition keys are `<namespace>.<name>`; the namespaces are {}",
                    list(Namespace::ALL.iter().map(|n| n.as_str()))
                ))
            }),
        _ => suggest(key, sections)
            .map(|section| format!("did you mean `{section}`?"))
            .or_else(|| Some(format!("a {} carries {}", kind.as_str(), list(sections)))),
    };

    cx.push(
        Diagnostic::error(
            DiagnosticCode::UnknownKey,
            entry.key.span.clone(),
            format!("unknown top-level key `{key}`"),
        )
        .with_optional_help(help),
    );
}

fn definition(entry: &Entry, cx: &mut Cx) -> Option<Definition> {
    let Some((prefix, _)) = entry.key.value.split_once('.') else {
        unknown_top_level_key(entry, DocumentKind::Spec, cx);
        return None;
    };
    if Namespace::from_prefix(prefix).is_none() {
        unknown_top_level_key(entry, DocumentKind::Spec, cx);
        return None;
    }
    let address = lexical::address(&entry.key, "a definition key", Namespace::ALL, cx)?;
    let body = definition::definition_body(&address, &entry.value, cx)?;
    Some(Definition {
        span: entry.key.span.joined(&entry.value.span),
        address,
        body,
    })
}

/// Read `version:` (grammar 1.3).
fn version(node: &Node, cx: &mut Cx) -> Option<Spanned<String>> {
    let Yaml::String(text) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::WrongType,
                node.span.clone(),
                format!(
                    "expected a string for `version`, found {}",
                    node.description()
                ),
            )
            .with_help("the value must be quoted: an unquoted `0.1` is a YAML float"),
        );
        return None;
    };
    if !SUPPORTED_SPEC_VERSIONS.contains(&text.as_str()) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::UnsupportedVersion,
                node.span.clone(),
                format!("`{text}` is not a spec version this compiler supports"),
            )
            .with_help(format!(
                "this build supports {}; run `agent-compose migrate` to update a spec written for another version",
                list(SUPPORTED_SPEC_VERSIONS)
            )),
        );
        return None;
    }
    Some(Spanned::new(text.clone(), node.span.clone()))
}
