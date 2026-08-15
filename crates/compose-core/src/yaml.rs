//! The YAML profile of grammar 1.1, as a spanned tree.
//!
//! The compiler does not deserialize YAML into typed structures directly: it
//! loads it into the [`Node`] tree below, where every node — including every
//! mapping *key* — carries the source region it came from, and then walks that
//! tree in [`crate::parse`]. Spans recorded here are what every later
//! diagnostic in the compiler points at.
//!
//! Loading enforces the profile the grammar fixes (grammar 1.1, Decision D49):
//!
//! * exactly one document per file;
//! * a mapping at the root;
//! * string keys only — `1:`, `[a]:`, `? complex` are errors;
//! * no duplicate keys, ever (never last-wins);
//! * no merge keys (`<<:`), a YAML 1.1 extension outside YAML 1.2 core;
//! * no tags (`!!str`, `!custom`);
//! * anchors and aliases are permitted and expanded here, before any
//!   spec-level processing sees the tree.
//!
//! Plain scalars resolve through the YAML 1.2 **core schema**: `true` is a
//! boolean, `0.1` is a float, `~` is null, and anything quoted is a string.
//! That is what makes `version: 0.1` a type error naming a float rather than a
//! silently accepted "0.1" (grammar 1.3).

use std::collections::HashMap;

use saphyr_parser::{Event, Marker, Parser, ScalarStyle, Span as RawSpan};

use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Position, SourceName, Span, Spanned};

/// A YAML value, resolved through the YAML 1.2 core schema.
#[derive(Clone, Debug, PartialEq)]
pub enum Yaml {
    /// `~`, `null`, or an empty plain scalar.
    Null,
    /// `true` / `false`.
    Bool(bool),
    /// An integer that fits in an `i64`.
    Int(i64),
    /// A float, or an integer too large for an `i64`.
    Float(f64),
    /// A quoted scalar, a block scalar, or a plain scalar that is none of the
    /// above.
    String(String),
    /// A sequence.
    Sequence(Vec<Node>),
    /// A mapping, in declaration order.
    Mapping(Mapping),
}

impl Yaml {
    /// How this value is named in a diagnostic, with its article: "a string",
    /// "an integer", "null".
    #[must_use]
    pub const fn description(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "a boolean",
            Self::Int(_) => "an integer",
            Self::Float(_) => "a number",
            Self::String(_) => "a string",
            Self::Sequence(_) => "a sequence",
            Self::Mapping(_) => "a mapping",
        }
    }
}

/// A YAML value paired with its source region.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// The value.
    pub value: Yaml,
    /// Where it was written.
    pub span: Span,
}

impl Node {
    /// How this node is named in a diagnostic ("a mapping").
    #[must_use]
    pub const fn description(&self) -> &'static str {
        self.value.description()
    }

    /// The mapping this node holds, if it is one.
    #[must_use]
    pub const fn as_mapping(&self) -> Option<&Mapping> {
        match &self.value {
            Yaml::Mapping(mapping) => Some(mapping),
            _ => None,
        }
    }

    /// The sequence this node holds, if it is one.
    #[must_use]
    pub fn as_sequence(&self) -> Option<&[Node]> {
        match &self.value {
            Yaml::Sequence(items) => Some(items),
            _ => None,
        }
    }

    /// The string this node holds, if it is one.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Yaml::String(text) => Some(text),
            _ => None,
        }
    }
}

/// A YAML mapping: ordered, duplicate-free entries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mapping {
    entries: Vec<Entry>,
}

/// One key/value pair of a [`Mapping`].
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// The key, with the span of the key token itself.
    pub key: Spanned<String>,
    /// The value.
    pub value: Node,
}

impl Mapping {
    /// The entries, in declaration order.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The entry with this key, if any.
    #[must_use]
    pub fn entry(&self, key: &str) -> Option<&Entry> {
        self.entries.iter().find(|entry| entry.key.value == key)
    }

    /// The value stored under this key, if any.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entry(key).map(|entry| &entry.value)
    }

    /// Whether this key is present.
    #[must_use]
    pub fn contains_key(&self, key: &str) -> bool {
        self.entry(key).is_some()
    }

    /// How many entries the mapping has.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the mapping has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Load one file's YAML into a spanned tree.
///
/// Returns the document's root node, or `None` when the file carries no usable
/// document (unreadable YAML, no document at all, a byte-order mark). Every
/// failure path pushes a [`Diagnostic`]; none of them panics, whatever the
/// input.
pub fn load(source: &str, name: &SourceName, diagnostics: &mut Diagnostics) -> Option<Node> {
    if source.starts_with('\u{feff}') {
        diagnostics.error(
            DiagnosticCode::InvalidEncoding,
            Span::file_start(name.clone()),
            "the file starts with a byte-order mark; spec files are UTF-8 without a BOM",
        );
        return None;
    }

    let mut loader = Loader {
        parser: Parser::new_from_str(source),
        source: name.clone(),
        offsets: char_offsets(source),
        end_of_file: source.len(),
        anchors: HashMap::new(),
        depth: 0,
        failed: false,
        diagnostics,
    };
    loader.load_document()
}

/// How deeply collections may nest before the loader gives up.
///
/// A spec file nests a couple of dozen levels at the very most (grammar 3.4
/// caps schema nesting at 8), so this only ever fires on input built to blow
/// the recursive loader's stack.
const MAX_NESTING: usize = 100;

struct Loader<'src, 'dx> {
    parser: Parser<'src, saphyr_parser::StrInput<'src>>,
    source: SourceName,
    offsets: Vec<usize>,
    end_of_file: usize,
    anchors: HashMap<usize, Node>,
    depth: usize,
    failed: bool,
    diagnostics: &'dx mut Diagnostics,
}

impl<'src> Loader<'src, '_> {
    fn load_document(&mut self) -> Option<Node> {
        let mut root = None;
        while let Some((event, span)) = self.next() {
            match event {
                Event::StreamStart | Event::DocumentEnd | Event::Nothing => {}
                Event::StreamEnd => break,
                Event::DocumentStart(_) => {
                    if root.is_some() {
                        self.diagnostics.error(
                            DiagnosticCode::MultipleDocuments,
                            self.span(span),
                            "a spec file holds exactly one YAML document; `---` starts a second one",
                        );
                        break;
                    }
                    let Some((event, span)) = self.next() else {
                        break;
                    };
                    root = Some(self.build(event, span));
                }
                // The parser only produces node events inside a document.
                other => {
                    let node = self.build(other, span);
                    root.get_or_insert(node);
                }
            }
        }

        match root {
            None => {
                if !self.failed {
                    self.diagnostics.error(
                        DiagnosticCode::EmptyDocument,
                        Span::file_start(self.source.clone()),
                        "the file declares no YAML document; a spec file is one document with a mapping at its root",
                    );
                }
                None
            }
            Some(node) if matches!(node.value, Yaml::Mapping(_)) => Some(node),
            Some(node) => {
                self.diagnostics.error(
                    DiagnosticCode::RootNotMapping,
                    node.span.clone(),
                    format!(
                        "the document root must be a mapping of sections and definitions, found {}",
                        node.description()
                    ),
                );
                None
            }
        }
    }

    /// Pull the next event, converting a scan error into a diagnostic and
    /// latching the stream shut.
    fn next(&mut self) -> Option<(Event<'src>, RawSpan)> {
        if self.failed {
            return None;
        }
        match self.parser.next_event() {
            None => None,
            Some(Ok(pair)) => Some(pair),
            Some(Err(error)) => {
                self.failed = true;
                let span = self.marker_span(*error.marker());
                self.diagnostics.error(
                    DiagnosticCode::YamlSyntax,
                    span,
                    format!("invalid YAML: {}", error.info()),
                );
                None
            }
        }
    }

    fn build(&mut self, event: Event<'src>, span: RawSpan) -> Node {
        if matches!(event, Event::SequenceStart(..) | Event::MappingStart(..))
            && self.depth >= MAX_NESTING
        {
            self.failed = true;
            let span = self.span(span);
            self.diagnostics.error(
                DiagnosticCode::YamlSyntax,
                span.clone(),
                format!("invalid YAML: collections nest more than {MAX_NESTING} levels deep"),
            );
            return Node {
                value: Yaml::Null,
                span,
            };
        }
        match event {
            Event::Scalar(text, style, anchor, tag) => {
                if tag.is_some() {
                    self.reject_tag(span);
                }
                let node = Node {
                    value: resolve_scalar(&text, style),
                    span: self.span(span),
                };
                self.record_anchor(anchor, &node);
                node
            }
            Event::SequenceStart(anchor, tag) => {
                if tag.is_some() {
                    self.reject_tag(span);
                }
                let node = self.build_sequence(span);
                self.record_anchor(anchor, &node);
                node
            }
            Event::MappingStart(anchor, tag) => {
                if tag.is_some() {
                    self.reject_tag(span);
                }
                let node = self.build_mapping(span);
                self.record_anchor(anchor, &node);
                node
            }
            Event::Alias(anchor) => match self.anchors.get(&anchor) {
                // An alias expands to its anchor's value, re-anchored on the
                // alias site so that a diagnostic about the expansion points at
                // the `*name` the author wrote. Spans inside the expansion keep
                // pointing at the anchor's definition, which is where that text
                // actually lives.
                Some(node) => Node {
                    value: node.value.clone(),
                    span: self.span(span),
                },
                None => {
                    let span = self.span(span);
                    self.diagnostics.error(
                        DiagnosticCode::YamlSyntax,
                        span.clone(),
                        "invalid YAML: alias refers to an anchor that is not defined",
                    );
                    Node {
                        value: Yaml::Null,
                        span,
                    }
                }
            },
            // Structural events never reach `build`: the collection builders
            // consume their own terminators and `load_document` filters the
            // rest. Treat any straggler as an empty value rather than panicking.
            _ => Node {
                value: Yaml::Null,
                span: self.span(span),
            },
        }
    }

    fn build_sequence(&mut self, start: RawSpan) -> Node {
        self.depth += 1;
        let mut items = Vec::new();
        let mut end = start;
        while let Some((event, span)) = self.next() {
            if matches!(event, Event::SequenceEnd) {
                end = span;
                break;
            }
            items.push(self.build(event, span));
        }
        self.depth -= 1;
        Node {
            value: Yaml::Sequence(items),
            span: self.joined_span(start, end),
        }
    }

    fn build_mapping(&mut self, start: RawSpan) -> Node {
        self.depth += 1;
        let mut mapping = Mapping::default();
        let mut end = start;
        loop {
            let Some((event, span)) = self.next() else {
                break;
            };
            if matches!(event, Event::MappingEnd) {
                end = span;
                break;
            }

            let key_node = self.build(event, span);
            let key = match key_node.value {
                Yaml::String(text) => Spanned::new(text, key_node.span),
                other => {
                    self.diagnostics.error(
                        DiagnosticCode::NonStringKey,
                        key_node.span,
                        format!(
                            "mapping keys must be strings, found {}",
                            other.description()
                        ),
                    );
                    self.skip_value();
                    continue;
                }
            };

            if key.value == "<<" {
                self.diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::MergeKey,
                        key.span,
                        "merge keys (`<<:`) are not supported; they are a YAML 1.1 extension outside YAML 1.2 core",
                    )
                    .with_help("repeat the keys, or alias the whole value with `*anchor`"),
                );
                self.skip_value();
                continue;
            }

            let value = match self.next() {
                Some((event, span)) => self.build(event, span),
                None => break,
            };

            if let Some(existing) = mapping.entry(&key.value) {
                self.diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::DuplicateKey,
                        key.span,
                        format!("duplicate key `{}`", key.value),
                    )
                    .with_label(existing.key.span.clone(), "first declared here")
                    .with_help("keys are declared once; the second declaration never wins"),
                );
                continue;
            }
            mapping.entries.push(Entry { key, value });
        }
        self.depth -= 1;
        Node {
            value: Yaml::Mapping(mapping),
            span: self.joined_span(start, end),
        }
    }

    /// Consume and discard the value of an entry whose key was rejected.
    fn skip_value(&mut self) {
        if let Some((event, span)) = self.next() {
            let _ = self.build(event, span);
        }
    }

    fn reject_tag(&mut self, span: RawSpan) {
        let span = self.span(span);
        self.diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::YamlTag,
                span,
                "YAML tags are not supported; the spec carries no language-level type directives",
            )
            .with_help("drop the tag: types come from the schema the value is declared against"),
        );
    }

    fn record_anchor(&mut self, anchor: usize, node: &Node) {
        if anchor != 0 {
            self.anchors.insert(anchor, node.clone());
        }
    }

    fn byte_of(&self, char_index: usize) -> usize {
        self.offsets
            .get(char_index)
            .copied()
            .unwrap_or(self.end_of_file)
    }

    fn position(marker: Marker) -> Position {
        // saphyr reports 1-based lines and 0-based columns.
        Position::new(
            u32::try_from(marker.line()).unwrap_or(u32::MAX),
            u32::try_from(marker.col().saturating_add(1)).unwrap_or(u32::MAX),
        )
    }

    fn span(&self, span: RawSpan) -> Span {
        Span::new(
            self.source.clone(),
            self.byte_of(span.start.index())..self.byte_of(span.end.index()),
            Self::position(span.start),
            Self::position(span.end),
        )
    }

    fn marker_span(&self, marker: Marker) -> Span {
        let byte = self.byte_of(marker.index());
        let position = Self::position(marker);
        Span::new(self.source.clone(), byte..byte, position, position)
    }

    /// The region running from the start of one event to the end of another.
    fn joined_span(&self, start: RawSpan, end: RawSpan) -> Span {
        let start_byte = self.byte_of(start.start.index());
        let end_byte = self.byte_of(end.end.index()).max(start_byte);
        Span::new(
            self.source.clone(),
            start_byte..end_byte,
            Self::position(start.start),
            Self::position(end.end),
        )
    }
}

/// Byte offset of every character in `source`, plus a final entry for its end.
fn char_offsets(source: &str) -> Vec<usize> {
    let mut offsets: Vec<usize> = source.char_indices().map(|(index, _)| index).collect();
    offsets.push(source.len());
    offsets
}

/// Resolve a scalar through the YAML 1.2 core schema.
///
/// Quoted and block scalars are always strings; plain scalars are null,
/// boolean, integer, or float when they match the core schema's productions,
/// and strings otherwise. An integer too large for an `i64` is kept as a float
/// rather than silently truncated — no legal spec value is affected, and the
/// value still reports as a number.
fn resolve_scalar(text: &str, style: ScalarStyle) -> Yaml {
    if style != ScalarStyle::Plain {
        return Yaml::String(text.to_owned());
    }
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Yaml::Null,
        "true" | "True" | "TRUE" => return Yaml::Bool(true),
        "false" | "False" | "FALSE" => return Yaml::Bool(false),
        _ => {}
    }
    if let Some(value) = parse_core_int(text) {
        return Yaml::Int(value);
    }
    if let Some(value) = parse_core_float(text) {
        return Yaml::Float(value);
    }
    Yaml::String(text.to_owned())
}

/// `[-+]?[0-9]+`, `0o[0-7]+`, or `0x[0-9a-fA-F]+`.
fn parse_core_int(text: &str) -> Option<i64> {
    if let Some(digits) = text.strip_prefix("0x") {
        return (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| i64::from_str_radix(digits, 16).ok())
            .flatten();
    }
    if let Some(digits) = text.strip_prefix("0o") {
        return (!digits.is_empty() && digits.bytes().all(|b| (b'0'..=b'7').contains(&b)))
            .then(|| i64::from_str_radix(digits, 8).ok())
            .flatten();
    }
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.trim_start_matches('+').parse::<i64>().ok()
}

/// `[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?`, `[-+]?.inf`, `.nan`.
fn parse_core_float(text: &str) -> Option<f64> {
    let unsigned = text.strip_prefix(['-', '+']).unwrap_or(text);
    let negative = text.starts_with('-');
    match unsigned {
        ".inf" | ".Inf" | ".INF" => {
            return Some(if negative {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        ".nan" | ".NaN" | ".NAN" => return (text == unsigned).then_some(f64::NAN),
        _ => {}
    }

    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    if let Some(exponent) = exponent {
        let digits = exponent.strip_prefix(['-', '+']).unwrap_or(exponent);
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let mantissa_ok = match mantissa.split_once('.') {
        // `.5`
        Some(("", fraction)) => !fraction.is_empty() && all_digits(fraction),
        // `1.`, `1.5`
        Some((whole, fraction)) => {
            all_digits(whole) && (fraction.is_empty() || all_digits(fraction))
        }
        // `1e3`
        None => exponent.is_some() && all_digits(mantissa),
    };
    if !mantissa_ok {
        return None;
    }
    text.trim_start_matches('+').parse::<f64>().ok()
}

fn all_digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_ok(source: &str) -> (Option<Node>, Vec<Diagnostic>) {
        let mut diagnostics = Diagnostics::new();
        let node = load(source, &SourceName::new("test.yml"), &mut diagnostics);
        (node, diagnostics.into_vec())
    }

    fn codes(diagnostics: &[Diagnostic]) -> Vec<&'static str> {
        diagnostics.iter().map(|d| d.code.as_str()).collect()
    }

    #[test]
    fn resolves_plain_scalars_through_the_core_schema() {
        assert_eq!(resolve_scalar("0.1", ScalarStyle::Plain), Yaml::Float(0.1));
        assert_eq!(
            resolve_scalar("0.1", ScalarStyle::DoubleQuoted),
            Yaml::String("0.1".into())
        );
        assert_eq!(resolve_scalar("true", ScalarStyle::Plain), Yaml::Bool(true));
        assert_eq!(
            resolve_scalar("true", ScalarStyle::SingleQuoted),
            Yaml::String("true".into())
        );
        assert_eq!(resolve_scalar("~", ScalarStyle::Plain), Yaml::Null);
        assert_eq!(resolve_scalar("", ScalarStyle::Plain), Yaml::Null);
        assert_eq!(resolve_scalar("-12", ScalarStyle::Plain), Yaml::Int(-12));
        assert_eq!(resolve_scalar("0x1f", ScalarStyle::Plain), Yaml::Int(31));
        assert_eq!(resolve_scalar("0o17", ScalarStyle::Plain), Yaml::Int(15));
        assert_eq!(
            resolve_scalar("1e3", ScalarStyle::Plain),
            Yaml::Float(1000.0)
        );
        assert_eq!(resolve_scalar(".5", ScalarStyle::Plain), Yaml::Float(0.5));
        assert_eq!(
            resolve_scalar("30s", ScalarStyle::Plain),
            Yaml::String("30s".into())
        );
        assert_eq!(
            resolve_scalar("1_000", ScalarStyle::Plain),
            Yaml::String("1_000".into())
        );
        assert_eq!(
            resolve_scalar("on", ScalarStyle::Plain),
            Yaml::String("on".into())
        );
    }

    #[test]
    fn records_spans_in_bytes_and_positions_over_multibyte_text() {
        let (node, diagnostics) = load_ok("kéy: värde\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let mapping = node.unwrap();
        let entry = &mapping.as_mapping().unwrap().entries()[0];
        assert_eq!(entry.key.value, "kéy");
        // `kéy` is three characters but four bytes.
        assert_eq!(entry.key.span.bytes, 0..4);
        assert_eq!(entry.key.span.start, Position::new(1, 1));
        assert_eq!(entry.value.span.bytes, 6..12);
        assert_eq!(entry.value.span.start, Position::new(1, 6));
    }

    #[test]
    fn expands_aliases_at_the_alias_site() {
        let (node, diagnostics) = load_ok("base: &b\n  x: 1\nchild: *b\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let root = node.unwrap();
        let child = root.as_mapping().unwrap().get("child").unwrap();
        assert_eq!(
            child.as_mapping().unwrap().get("x").unwrap().value,
            Yaml::Int(1)
        );
        // The expansion is re-anchored on `*b`, not on the anchor's definition.
        assert_eq!(child.span.start, Position::new(3, 8));
    }

    #[test]
    fn rejects_the_yaml_profile_violations() {
        assert_eq!(codes(&load_ok("a: 1\na: 2\n").1), ["duplicate-key"]);
        assert_eq!(
            codes(&load_ok("b: &x {p: 1}\nc:\n  <<: *x\n").1),
            ["merge-key"]
        );
        assert_eq!(codes(&load_ok("a: !!str 5\n").1), ["yaml-tag"]);
        assert_eq!(codes(&load_ok("1: a\n").1), ["non-string-key"]);
        assert_eq!(
            codes(&load_ok("a: 1\n---\nb: 2\n").1),
            ["multiple-documents"]
        );
        assert_eq!(codes(&load_ok("hello\n").1), ["root-not-mapping"]);
        assert_eq!(codes(&load_ok("- 1\n").1), ["root-not-mapping"]);
        assert_eq!(codes(&load_ok("").1), ["empty-document"]);
        assert_eq!(codes(&load_ok("# just a comment\n").1), ["empty-document"]);
        assert_eq!(codes(&load_ok("\u{feff}a: 1\n").1), ["invalid-encoding"]);
        assert_eq!(codes(&load_ok("a: [1, 2\nb: 3\n").1), ["yaml-syntax"]);
        assert_eq!(codes(&load_ok("a:\n\tb: 1\n").1), ["yaml-syntax"]);
    }

    #[test]
    fn keeps_parsing_after_a_recoverable_profile_violation() {
        let (node, diagnostics) = load_ok("a: 1\na: 2\nb: 3\n");
        assert_eq!(codes(&diagnostics), ["duplicate-key"]);
        let root = node.expect("the tree survives a duplicate key");
        let mapping = root.as_mapping().unwrap();
        assert_eq!(mapping.len(), 2);
        assert_eq!(mapping.get("a").unwrap().value, Yaml::Int(1));
        assert_eq!(mapping.get("b").unwrap().value, Yaml::Int(3));
    }

    #[test]
    fn never_panics_on_hostile_input() {
        for source in [
            "\u{0}\u{1}\u{2}",
            "a: *undefined\n",
            "? [a, b]\n: c\n",
            "!!binary |\n  R0lG\n",
            "%YAML 1.3\n---\na: 1\n",
            "a: \"unterminated\n",
            &"[".repeat(200),
            &"a:\n".repeat(500),
        ] {
            let _ = load_ok(source);
        }
    }
}
