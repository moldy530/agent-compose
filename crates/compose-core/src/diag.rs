//! Diagnostics: the compiler's error contract.
//!
//! Every failure the compiler can report — from an unreadable file to a
//! cycle-termination proof that does not close — is a [`Diagnostic`]. The type
//! is deliberately inert data: it carries a stable machine-readable
//! [`DiagnosticCode`], a severity, a primary message anchored to a [`Span`],
//! optional labelled secondary spans, and optional help text. Rendering (ANSI
//! snippets, JSON, LSP payloads) belongs to the layer that reports them, not
//! here, so nothing in this module knows about terminals or serialization
//! formats.
//!
//! # The code scheme
//!
//! Codes are lowercase kebab-case strings drawn from the closed
//! [`DiagnosticCode`] enum: `unknown-key`, `invalid-duration`, `reserved-name`.
//! They are the stable identity of a failure class — test fixtures, editors,
//! and downstream tooling match on them, so a code is never renamed or reused
//! for a different meaning once it ships; a new failure class gets a new
//! variant. Messages, by contrast, are free to improve: the fixture corpus in
//! `tests/fixtures/invalid-parse/` pins the exact text so an improvement is a
//! reviewed diff rather than a silent regression (PRD G3 — error UX is a
//! product feature).
//!
//! Codes are kebab strings rather than `E0001`-style numbers because the
//! compiler is read by coding agents as much as by humans: `unknown-key` is
//! self-describing in a terminal, in a test fixture header, and in a grep.

use std::fmt;
use std::ops::Range;
use std::sync::Arc;

/// The name of a source file, as spans and diagnostics refer to it.
///
/// This is the path exactly as the caller supplied it (typically relative to
/// the project root), interned behind an [`Arc`] so that the thousands of
/// spans one file produces share one allocation.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct SourceName(Arc<str>);

impl SourceName {
    /// Intern a source name.
    pub fn new(name: impl AsRef<str>) -> Self {
        Self(Arc::from(name.as_ref()))
    }

    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for SourceName {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for SourceName {
    fn from(name: String) -> Self {
        Self::new(name)
    }
}

impl fmt::Display for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Debug for SourceName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&self.0, f)
    }
}

/// A 1-based line/column position.
///
/// Columns count characters, not bytes: they are what an editor shows.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number, in characters.
    pub column: u32,
}

impl Position {
    /// A position at `line`:`column`, both 1-based.
    #[must_use]
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

impl fmt::Debug for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// A region of one source file.
///
/// Carries both a byte range (for slicing the source) and 1-based line/column
/// endpoints (for reporting), because recomputing one from the other at report
/// time would mean keeping every source string alive.
#[derive(Clone, PartialEq, Eq)]
pub struct Span {
    /// The file this region belongs to.
    pub source: SourceName,
    /// Byte offsets into that file, `start..end`, end-exclusive.
    pub bytes: Range<usize>,
    /// Position of the first character.
    pub start: Position,
    /// Position just past the last character.
    pub end: Position,
}

impl Span {
    /// Build a span from its parts.
    #[must_use]
    pub fn new(source: SourceName, bytes: Range<usize>, start: Position, end: Position) -> Self {
        Self {
            source,
            bytes,
            start,
            end,
        }
    }

    /// An empty span at the very beginning of a file, used when a diagnostic is
    /// about the file as a whole (an empty document, a byte-order mark).
    #[must_use]
    pub fn file_start(source: SourceName) -> Self {
        Self::new(source, 0..0, Position::new(1, 1), Position::new(1, 1))
    }

    /// A zero-width span collapsed onto this one's start.
    #[must_use]
    pub fn collapsed(&self) -> Self {
        Self::new(
            self.source.clone(),
            self.bytes.start..self.bytes.start,
            self.start,
            self.start,
        )
    }
}

/// `Debug` renders `line:col..line:col` — position only, no byte offsets and no
/// file name. AST snapshots are per file and byte offsets shift with every
/// unrelated edit above them, so the compact form keeps snapshot diffs about
/// the change under review.
impl fmt::Debug for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..{}", self.start, self.end)
    }
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.source, self.start)
    }
}

/// A value paired with the source region it was written in.
#[derive(Clone, PartialEq)]
pub struct Spanned<T> {
    /// The value.
    pub value: T,
    /// Where it was written.
    pub span: Span,
}

impl<T> Spanned<T> {
    /// Pair a value with a span.
    pub const fn new(value: T, span: Span) -> Self {
        Self { value, span }
    }

    /// Apply `f` to the value, keeping the span.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned {
            value: f(self.value),
            span: self.span,
        }
    }

    /// Borrow the value.
    pub const fn as_ref(&self) -> &T {
        &self.value
    }
}

/// `Debug` renders `value @ line:col..line:col`, and honours the alternate
/// flag so that a spanned struct still pretty-prints inside `{:#?}` output.
impl<T: fmt::Debug> fmt::Debug for Spanned<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if f.alternate() {
            write!(f, "{:#?} @ {:?}", self.value, self.span)
        } else {
            write!(f, "{:?} @ {:?}", self.value, self.span)
        }
    }
}

/// How bad a diagnostic is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// The composition is rejected.
    Error,
    /// The composition is accepted, with a caveat worth surfacing.
    Warning,
}

impl Severity {
    /// The lowercase name of this severity.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The stable identity of a failure class.
///
/// See the module documentation for the scheme. Variants are grouped by the
/// layer that raises them; the parser raises all of the ones below.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DiagnosticCode {
    // --- file level -------------------------------------------------------
    /// The file could not be read.
    IoError,
    /// The file is not valid UTF-8, or starts with a byte-order mark.
    InvalidEncoding,
    /// The YAML itself does not parse.
    YamlSyntax,
    /// The file contains no YAML document.
    EmptyDocument,
    /// The file contains more than one YAML document.
    MultipleDocuments,
    /// The document's root node is not a mapping.
    RootNotMapping,

    // --- YAML profile (grammar 1.1) ---------------------------------------
    /// A mapping key is not a string.
    NonStringKey,
    /// A mapping declares the same key twice.
    DuplicateKey,
    /// A mapping uses the YAML 1.1 merge key `<<:`.
    MergeKey,
    /// A node carries a YAML tag.
    YamlTag,

    // --- document model ---------------------------------------------------
    /// A section appears in a document kind that may not carry it.
    MisplacedSection,
    /// The `version:` value is outside this build's supported set.
    UnsupportedVersion,
    /// An `imports:` entry is not a relative path to a spec file.
    InvalidImportPath,

    // --- construct shape --------------------------------------------------
    /// A mapping carries a key the construct does not define.
    UnknownKey,
    /// A required key is absent.
    MissingKey,
    /// A value has the wrong YAML kind.
    WrongType,
    /// A value is well-typed but not legal here.
    InvalidValue,
    /// A numeric value is outside its declared range.
    ValueOutOfRange,
    /// A value is not one of a closed set of variants.
    UnknownVariant,
    /// Two keys that may not appear together both appear.
    ConflictingKeys,

    // --- lexical forms ----------------------------------------------------
    /// A string is not a legal identifier (grammar 2.1).
    InvalidIdentifier,
    /// A string is not a legal typed address, or names the wrong namespace.
    InvalidReference,
    /// A string is not a legal duration (grammar 4.4).
    InvalidDuration,
    /// A string is not a legal path expression (grammar 4.2).
    InvalidPathExpression,
    /// An environment reference is malformed, or a secret-bearing field holds a
    /// literal instead of one.
    InvalidEnvRef,
    /// An environment reference appears on a surface that never interpolates.
    UnexpectedEnvRef,
    /// An identifier is reserved for another purpose.
    ReservedName,
}

impl DiagnosticCode {
    /// The stable kebab-case spelling of this code.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::IoError => "io-error",
            Self::InvalidEncoding => "invalid-encoding",
            Self::YamlSyntax => "yaml-syntax",
            Self::EmptyDocument => "empty-document",
            Self::MultipleDocuments => "multiple-documents",
            Self::RootNotMapping => "root-not-mapping",
            Self::NonStringKey => "non-string-key",
            Self::DuplicateKey => "duplicate-key",
            Self::MergeKey => "merge-key",
            Self::YamlTag => "yaml-tag",
            Self::MisplacedSection => "misplaced-section",
            Self::UnsupportedVersion => "unsupported-version",
            Self::InvalidImportPath => "invalid-import-path",
            Self::UnknownKey => "unknown-key",
            Self::MissingKey => "missing-key",
            Self::WrongType => "wrong-type",
            Self::InvalidValue => "invalid-value",
            Self::ValueOutOfRange => "value-out-of-range",
            Self::UnknownVariant => "unknown-variant",
            Self::ConflictingKeys => "conflicting-keys",
            Self::InvalidIdentifier => "invalid-identifier",
            Self::InvalidReference => "invalid-reference",
            Self::InvalidDuration => "invalid-duration",
            Self::InvalidPathExpression => "invalid-path-expression",
            Self::InvalidEnvRef => "invalid-env-ref",
            Self::UnexpectedEnvRef => "unexpected-env-ref",
            Self::ReservedName => "reserved-name",
        }
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A secondary span attached to a diagnostic, with the role it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// Where the related source is.
    pub span: Span,
    /// What it contributes ("first defined here").
    pub message: String,
}

/// One compiler diagnostic.
#[derive(Clone, Debug, PartialEq)]
pub struct Diagnostic {
    /// Stable machine-readable identity of the failure class.
    pub code: DiagnosticCode,
    /// How bad it is.
    pub severity: Severity,
    /// One-line statement of what is wrong, in lowercase, without a trailing
    /// period.
    pub message: String,
    /// The source region the message is about.
    pub span: Span,
    /// Related regions, each with its own note.
    pub labels: Vec<Label>,
    /// Optional actionable follow-up ("did you mean `retry`?").
    pub help: Option<String>,
}

impl Diagnostic {
    /// Start an error diagnostic.
    pub fn error(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Self {
        Self {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            labels: Vec::new(),
            help: None,
        }
    }

    /// Start a warning diagnostic.
    pub fn warning(code: DiagnosticCode, span: Span, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, span, message)
        }
    }

    /// Attach a labelled secondary span.
    #[must_use]
    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    /// Attach help text.
    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Attach help text when there is any.
    #[must_use]
    pub fn with_optional_help(mut self, help: Option<String>) -> Self {
        self.help = help;
        self
    }

    /// Whether this diagnostic rejects the composition.
    #[must_use]
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// An ordered collection of diagnostics.
///
/// Passes push into one of these as they go; [`Diagnostics::sort`] puts them in
/// source order at the end so that a file's diagnostics read top to bottom
/// regardless of the order the passes happened to visit constructs in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// An empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a diagnostic.
    pub fn push(&mut self, diagnostic: Diagnostic) {
        self.items.push(diagnostic);
    }

    /// Record an error diagnostic, returning nothing — the common call shape.
    pub fn error(&mut self, code: DiagnosticCode, span: Span, message: impl Into<String>) {
        self.push(Diagnostic::error(code, span, message));
    }

    /// Whether anything has been recorded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// How many diagnostics have been recorded.
    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// Whether any recorded diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.items.iter().any(Diagnostic::is_error)
    }

    /// The diagnostics, in the order they will be reported.
    #[must_use]
    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    /// Iterate over the diagnostics.
    pub fn iter(&self) -> std::slice::Iter<'_, Diagnostic> {
        self.items.iter()
    }

    /// Put the diagnostics in source order: by file, then by start offset, then
    /// by code, so that output is deterministic for a given input (PRD 5.12).
    pub fn sort(&mut self) {
        self.items.sort_by(|a, b| {
            a.span
                .source
                .as_str()
                .cmp(b.span.source.as_str())
                .then(a.span.bytes.start.cmp(&b.span.bytes.start))
                .then(a.span.bytes.end.cmp(&b.span.bytes.end))
                .then(a.code.cmp(&b.code))
                .then(a.message.cmp(&b.message))
        });
    }

    /// Consume the collection, yielding its diagnostics.
    #[must_use]
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }
}

impl IntoIterator for Diagnostics {
    type Item = Diagnostic;
    type IntoIter = std::vec::IntoIter<Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a> IntoIterator for &'a Diagnostics {
    type Item = &'a Diagnostic;
    type IntoIter = std::slice::Iter<'a, Diagnostic>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

impl Extend<Diagnostic> for Diagnostics {
    fn extend<T: IntoIterator<Item = Diagnostic>>(&mut self, iter: T) {
        self.items.extend(iter);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(source: &str, bytes: Range<usize>, line: u32, column: u32) -> Span {
        Span::new(
            SourceName::new(source),
            bytes,
            Position::new(line, column),
            Position::new(line, column + 1),
        )
    }

    #[test]
    fn codes_are_kebab_case_and_unique() {
        let codes = [
            DiagnosticCode::IoError,
            DiagnosticCode::InvalidEncoding,
            DiagnosticCode::YamlSyntax,
            DiagnosticCode::EmptyDocument,
            DiagnosticCode::MultipleDocuments,
            DiagnosticCode::RootNotMapping,
            DiagnosticCode::NonStringKey,
            DiagnosticCode::DuplicateKey,
            DiagnosticCode::MergeKey,
            DiagnosticCode::YamlTag,
            DiagnosticCode::MisplacedSection,
            DiagnosticCode::UnsupportedVersion,
            DiagnosticCode::InvalidImportPath,
            DiagnosticCode::UnknownKey,
            DiagnosticCode::MissingKey,
            DiagnosticCode::WrongType,
            DiagnosticCode::InvalidValue,
            DiagnosticCode::ValueOutOfRange,
            DiagnosticCode::UnknownVariant,
            DiagnosticCode::ConflictingKeys,
            DiagnosticCode::InvalidIdentifier,
            DiagnosticCode::InvalidReference,
            DiagnosticCode::InvalidDuration,
            DiagnosticCode::InvalidPathExpression,
            DiagnosticCode::InvalidEnvRef,
            DiagnosticCode::UnexpectedEnvRef,
            DiagnosticCode::ReservedName,
        ];
        let mut seen = std::collections::BTreeSet::new();
        for code in codes {
            let text = code.as_str();
            assert!(
                text.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "{text} is not kebab-case"
            );
            assert!(seen.insert(text), "{text} is used by two variants");
        }
    }

    #[test]
    fn diagnostics_sort_into_source_order() {
        let mut diagnostics = Diagnostics::new();
        diagnostics.error(
            DiagnosticCode::UnknownKey,
            span("b.yml", 0..1, 1, 1),
            "second file",
        );
        diagnostics.error(
            DiagnosticCode::UnknownKey,
            span("a.yml", 20..21, 3, 5),
            "later in the first file",
        );
        diagnostics.error(
            DiagnosticCode::MissingKey,
            span("a.yml", 4..5, 1, 5),
            "earlier in the first file",
        );
        diagnostics.sort();
        let messages: Vec<&str> = diagnostics.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(
            messages,
            [
                "earlier in the first file",
                "later in the first file",
                "second file"
            ]
        );
    }

    #[test]
    fn spans_debug_as_positions_only() {
        let span = Span::new(
            SourceName::new("main.yml"),
            10..14,
            Position::new(3, 5),
            Position::new(3, 9),
        );
        assert_eq!(format!("{span:?}"), "3:5..3:9");
        assert_eq!(span.to_string(), "main.yml:3:5");
        assert_eq!(
            format!("{:?}", Spanned::new("draft", span)),
            "\"draft\" @ 3:5..3:9"
        );
    }
}
