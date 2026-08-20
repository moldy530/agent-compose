//! The parser: one file in, a typed [`Document`] plus diagnostics out.
//!
//! # What the parser decides
//!
//! Everything the parser reports is decidable from a **single file**. That is
//! the same line `docs/grammar.md` Appendix B draws for the published JSON
//! Schema, and the parser decides every per-file rule the grammar states: the
//! YAML profile (grammar 1.1), section placement (1.5), every construct's key
//! table, the closed vocabularies, the lexical forms of grammar 2 and 4, and
//! the structural rules whose deciding value is a literal in the same object —
//! a node's kind key, a trigger's `type:`, a store op's parameter row, the
//! direct-XOR-route split on models, the `node:` XOR `route_by:`+`routes:`
//! split on maps.
//!
//! The published schema is meant to be the looser, editor-facing approximation
//! of this pass, and `tests/parse_invalid.rs` replays its negative corpus here
//! to keep the two from drifting. The direction of the relationship is fixed by
//! Appendix B and it is one-directional: a file the schema rejects must fail
//! `validate`, never the reverse. `validate` is more than this pass, though —
//! it parses, resolves, and then runs the static checks — so the invariant
//! *this module* can be held to is the same claim narrowed to its own scope:
//! **on every rule this pass owns, it is at least as strict as the schema, and
//! it may be stricter.** Two rules where it used to be looser are closed. The
//! `imports:` pattern the schema publishes is grammar 1.4's own segment charset
//! (Decision D80) rather than a constraint of its own, and `section.rs`
//! enforces exactly that charset, so a relative path carrying a space is
//! refused by both. And `cron:` is anchored here the way the schema anchors it
//! (`^\S+(\s+\S+){4}$`), so a trailing space is refused rather than swallowed
//! by a field count that ignores it.
//!
//! Where the schema is tighter than this pass, the rule belongs to a later one
//! by Appendix B's own assignment, and `settings:` is the case to know. The
//! schema types the nine keys it recognises — `max_tokens` at least 1, a
//! numeric `temperature`, `stop` as an array of strings — while this pass reads
//! the block as an open literal and checks only what grammar 4.3 makes its
//! business, that no value inside it reaches for an environment reference.
//! Appendix B gives provider settings and capability checks to the validator,
//! because whether `thinking:` is even a key depends on the provider the model
//! names, which is another file. So `model.m: { …, settings: { max_tokens: 0 } }`
//! parses clean and fails later: the replay above says nothing about `settings:`
//! shape, and neither does anything else here.
//!
//! What it deliberately leaves alone:
//!
//! * **`imports:` is a list of strings.** Following it is the resolver's job;
//!   the parser only rejects the shapes grammar 1.4 calls parse errors —
//!   absolute paths, URLs, globs, non-`.yml` files, and a path outside the
//!   portable segment charset the same section fixes (Decision D80).
//! * **References are not resolved.** `model: model.smart` is checked for
//!   *namespace* — the grammar's "expected a `model.*` reference, found
//!   `tool.web_search`" — but whether `model.smart` exists is a cross-file
//!   question.
//! * **CEL stays raw.** Its roots depend on the surface and its types on the
//!   resolved schemas (grammar 4.1). Rules that turn on what an expression
//!   *reads* rather than on its shape are the validator's for the same reason:
//!   a `method: GET` trigger reading through `payload.body` (grammar 13.3,
//!   Decision D117), and the item-derivation of a store key (grammar 11.4,
//!   Decision D83).
//! * **Nothing graph-shaped.** Exhaustiveness, SCC termination, fan-out
//!   bounding through a `map.over` path, `map.over` dominance, reducer-write
//!   rules, the injectivity of a node's effective write map, balanced
//!   convergence, and both reachability relations all need more than a key and
//!   its value.
//!
//! # Where the line falls, and why
//!
//! The grammar's own conformance language draws it: a *parse error* is a
//! rejection "before resolution", a *compile error* is anything `validate`
//! refuses. So the question this pass asks of every rule is **does deciding it
//! need another file** — not whether the published schema happens to express
//! it. Appendix B's "the validator owns it" marks the rules JSON Schema cannot
//! reach, and several of those need nothing but the file in hand:
//!
//! * `optional:` entries naming declared properties (grammar 3.4, Decision
//!   D89) — one type node's own two keys;
//! * a `when:`-guarded sibling for every `else: true` edge (grammar 7.3,
//!   Decision D107) — two items of one `edges:` array;
//! * at most one `else: true` edge per source node (grammar 7.3 rule 4) — the
//!   same two items, related the same way. Appendix B does not list this one at
//!   all: `uniqueItems` on `edges:` catches byte-identical edge objects, never
//!   two that agree on `from` and `else` alone, so it would otherwise fall
//!   between the schema, this pass, and PRD §7 M0's static-check list.
//!
//! All three are decided here. Being stricter than the schema is always safe:
//! the invariant Appendix B closes with is one-directional, so a file the schema
//! rejects must fail `validate`, and never the reverse.
//!
//! Grammar 7.6.3's three no-dead-end rules then split on the same question.
//! Rule 2 — some edge leaving `start` is unconditional or carries `else: true`
//! — is stated over the `edges:` array, which is one value, so it is decided
//! here. Its two siblings are not: "every node has an outgoing edge" and the
//! `on_error: skip` escape each relate an edge to a *node*, which is the graph
//! the validator builds.
//!
//! The rules that stay out are the ones that genuinely need more: a
//! `method: GET` trigger reading through `payload.body` (Decision D117) needs
//! the CEL grammar rather than a regex over the string, and the item-derivation
//! of a store key (Decision D83) is a path through map bindings in other files.
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
    parse_as(source, name, FileRole::Unknown)
}

/// Parse one file whose place in a composition the caller already knows.
///
/// The resolver reads every file this way. See [`FileRole`] for what the extra
/// knowledge buys.
pub(crate) fn parse_as(source: &str, name: impl Into<SourceName>, role: FileRole) -> ParsedFile {
    let name = name.into();
    let mut diagnostics = Diagnostics::new();
    let root = yaml::load(source, &name, &mut diagnostics);
    let document = root.and_then(|root| {
        let mut cx = Cx::new(&mut diagnostics);
        document(&root, name, role, &mut cx)
    });
    diagnostics.sort();
    ParsedFile {
        document,
        diagnostics: diagnostics.into_vec(),
    }
}

/// What the caller already knows about a file's place in a composition
/// (grammar 1.2).
///
/// Grammar 1.3 states its two `version:` rules over **roles** — REQUIRED in the
/// entrypoint and in every deploy file, OPTIONAL in an imported file — and this
/// pass decides everything from one file, so on its own it can only infer the
/// role from what the file itself declares: a file carrying `imports:` must be
/// the entrypoint, a file carrying a deploy section must be a deploy file.
/// Appendix B blesses that approximation for the published JSON Schema, which
/// also sees one file at a time.
///
/// The resolver does not have to approximate — it is holding the composition —
/// and `validate` is the authority, so it must not contradict itself: telling
/// an author that `mid.yml` "is the entrypoint" while the next line refuses it
/// *for being imported* is one mistake reported as two, and one of the two is
/// false. Passing the role keeps each rule where its premise is true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileRole {
    /// Nothing is known: the file is being read on its own, so both rules are
    /// approximated from it.
    Unknown,
    /// The spec file named on the command line — the only one that may declare
    /// `imports:` (Decision D1).
    Entrypoint,
    /// A spec file reached from the entrypoint's `imports:`. Its `version:` is
    /// optional, and a role-shaped rule stated against it would be false.
    Import,
    /// The active target's `deploy/<target>.yml`, which is selected rather than
    /// imported (grammar 14).
    Deploy,
}

impl FileRole {
    /// Whether this file could be the entrypoint — the premise of grammar 1.3's
    /// "a file that declares `imports:` must declare `version:`".
    const fn may_be_the_entrypoint(self) -> bool {
        matches!(self, Self::Unknown | Self::Entrypoint)
    }

    /// Whether this file could be the target's deploy file — the premise of
    /// grammar 1.3's "every deploy file declares `version:`".
    const fn may_be_the_deploy_file(self) -> bool {
        matches!(self, Self::Unknown | Self::Deploy)
    }
}

/// Sections that only a deploy file may carry (grammar 1.5).
const DEPLOY_ONLY: &[&str] = &["placements", "storage_backends", "event_sources"];
/// Sections that only a spec file may carry (grammar 1.5).
const SPEC_ONLY: &[&str] = &["imports", "defaults", "state", "triggers"];

fn document(root: &Node, source: SourceName, role: FileRole, cx: &mut Cx) -> Option<Document> {
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
        Some(Document::Deploy(deploy_file(
            mapping, root, source, role, cx,
        )))
    } else {
        Some(Document::Spec(spec_file(
            mapping,
            root,
            source,
            spec_evidence,
            role,
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
    role: FileRole,
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
    // Decision D4's rule, made only where its premise can still be true: a
    // caller holding the composition has already said which file this is, and
    // an import or a deploy file is refused for *that* rather than told it is
    // the entrypoint (see `FileRole`).
    // A `version:` that was declared but rejected has already been reported;
    // saying it is missing as well would be two diagnostics for one mistake.
    if role.may_be_the_entrypoint()
        && !mapping.contains_key("version")
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

fn deploy_file(
    mapping: &Mapping,
    root: &Node,
    source: SourceName,
    role: FileRole,
    cx: &mut Cx,
) -> DeployFile {
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

    // Stated only where its premise can still be true: a file the caller is
    // holding as an import is refused for being imported at all, and one it is
    // holding as the entrypoint for being the entrypoint, so a second
    // diagnostic about its `version:` would be noise on a file that is in the
    // wrong place entirely (see `FileRole`).
    if role.may_be_the_deploy_file() && !mapping.contains_key("version") {
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
                "this build supports {}; a spec written for another version has its syntax brought forward, not just this field",
                list(SUPPORTED_SPEC_VERSIONS)
            )),
        );
        return None;
    }
    Some(Spanned::new(text.clone(), node.span.clone()))
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticCode, FileRole, parse_as};

    const IMPORTS_NO_VERSION: &str = "imports:\n  - other.yml\n";
    const DEPLOY_NO_VERSION: &str = "placements:\n  flow.f: { runtime: colocated }\n";
    const ENTRYPOINT_RULE: &str =
        "a file that declares `imports:` is the entrypoint and must declare `version:`";
    const DEPLOY_RULE: &str = "a deploy file must declare `version:`";

    /// Every missing-`version:` diagnostic this file draws in that role.
    fn missing_version(source: &str, role: FileRole) -> Vec<String> {
        parse_as(source, "f.yml", role)
            .diagnostics
            .into_iter()
            .filter(|diagnostic| diagnostic.code == DiagnosticCode::MissingKey)
            .map(|diagnostic| diagnostic.message)
            .collect()
    }

    /// `Unknown` is the standalone reading — `parse_str`, and the published
    /// schema's one-file view — where the role can only be guessed from what
    /// the file declares.
    #[test]
    fn a_file_read_on_its_own_is_held_to_the_role_it_looks_like() {
        assert_eq!(
            missing_version(IMPORTS_NO_VERSION, FileRole::Unknown),
            [ENTRYPOINT_RULE]
        );
        assert_eq!(
            missing_version(DEPLOY_NO_VERSION, FileRole::Unknown),
            [DEPLOY_RULE]
        );
    }

    #[test]
    fn the_entrypoint_is_still_required_to_declare_a_version() {
        assert_eq!(
            missing_version(IMPORTS_NO_VERSION, FileRole::Entrypoint),
            [ENTRYPOINT_RULE]
        );
    }

    #[test]
    fn the_deploy_file_is_still_required_to_declare_a_version() {
        assert_eq!(
            missing_version(DEPLOY_NO_VERSION, FileRole::Deploy),
            [DEPLOY_RULE]
        );
    }

    /// An imported file's `version:` is OPTIONAL (grammar 1.3), so neither rule
    /// may be stated against one. The resolver refuses it for declaring
    /// `imports:`, or for being a deploy file, and a second diagnostic calling
    /// it the entrypoint would contradict the pass raising it.
    #[test]
    fn an_imported_file_is_told_neither_rule() {
        assert!(missing_version(IMPORTS_NO_VERSION, FileRole::Import).is_empty());
        assert!(missing_version(DEPLOY_NO_VERSION, FileRole::Import).is_empty());
    }

    /// The two roles that *are* required to carry a `version:` are each held to
    /// their own rule only: an entrypoint that is a deploy file is refused for
    /// being one, and a deploy file that declares `imports:` for declaring
    /// them, so neither is also told the rule of the kind it is not.
    #[test]
    fn a_file_in_the_wrong_place_is_not_also_told_the_other_rule() {
        assert!(missing_version(DEPLOY_NO_VERSION, FileRole::Entrypoint).is_empty());
        assert!(missing_version(IMPORTS_NO_VERSION, FileRole::Deploy).is_empty());
    }
}
