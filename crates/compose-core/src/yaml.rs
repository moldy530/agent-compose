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
//!   spec-level processing sees the tree — up to a total expansion budget
//!   (`MAX_NODES`), so a file built to multiply itself is a diagnostic rather
//!   than an out-of-memory kill.
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
    dropped: usize,
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

    /// How many entries the source declared that the loader dropped: a
    /// non-string key, a merge key, or a duplicate (grammar 1.1).
    ///
    /// Each one already carries its own diagnostic, so a rule stated over *how
    /// many* entries a mapping has cannot be answered from what survived —
    /// "must declare at least one node" of a `nodes:` block the loader emptied
    /// would be a second diagnostic for one mistake, and false besides: the
    /// author did declare a node.
    #[must_use]
    pub const fn dropped(&self) -> usize {
        self.dropped
    }

    /// Whether the source declared no entries at all — the author really did
    /// write `{}`, rather than writing entries none of which survived
    /// [`Mapping::dropped`].
    #[must_use]
    pub const fn declares_nothing(&self) -> bool {
        self.entries.is_empty() && self.dropped == 0
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
        text: source,
        source: name.clone(),
        offsets: char_offsets(source),
        end_of_file: source.len(),
        anchors: HashMap::new(),
        depth: 0,
        nodes: 0,
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

/// How many nodes a document may expand to before the loader gives up.
///
/// `MAX_NESTING` bounds *depth*, which says nothing about total size: an alias
/// expands to a copy of its anchor's whole value, so anchoring each level on
/// the one below multiplies fan-out at every step and a few hundred bytes of
/// input reach hundreds of millions of nodes. Anchors are legal (grammar 1.1,
/// Decision D49) and expansion is therefore required, but `validate` reads
/// untrusted spec files inside a millisecond budget (PRD 5.12), so the total is
/// capped rather than the depth alone.
///
/// The bound is deliberately far above any hand-written file: the largest
/// example in this repo is under a thousand nodes, and a 10,000-line spec would
/// still sit an order of magnitude below it.
const MAX_NODES: usize = 100_000;

struct Loader<'src, 'dx> {
    parser: Parser<'src, saphyr_parser::StrInput<'src>>,
    /// The file being loaded. A collection's span is read back off it to find
    /// where the collection's own text stops ([`Loader::collection_span`]).
    text: &'src str,
    source: SourceName,
    offsets: Vec<usize>,
    end_of_file: usize,
    anchors: HashMap<usize, Anchor>,
    depth: usize,
    /// Nodes materialized so far, alias expansions counted at full size.
    nodes: usize,
    failed: bool,
    diagnostics: &'dx mut Diagnostics,
}

/// An anchored value and the size of the tree an alias to it expands to.
struct Anchor {
    node: Node,
    nodes: usize,
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

        // A latched failure means the event stream was cut short — bad syntax,
        // nesting past `MAX_NESTING`, an expansion past `MAX_NODES` — so
        // whatever was built is a truncated prefix of the file. It has already
        // been reported; walking it would bury that one diagnostic under
        // spec-level complaints about the half of the file that never arrived.
        if self.failed {
            return None;
        }

        match root {
            None => {
                self.diagnostics.error(
                    DiagnosticCode::EmptyDocument,
                    Span::file_start(self.source.clone()),
                    "the file declares no YAML document; a spec file is one document with a mapping at its root",
                );
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
        let before = self.nodes;
        match event {
            Event::Scalar(text, style, anchor, tag) => {
                if tag.is_some() {
                    self.reject_tag(span);
                }
                if !self.charge(1, span) {
                    return self.empty(span);
                }
                let node = Node {
                    value: resolve_scalar(&text, style),
                    span: self.span(span),
                };
                self.record_anchor(anchor, &node, self.nodes - before);
                node
            }
            Event::SequenceStart(anchor, tag) => {
                if tag.is_some() {
                    self.reject_tag(span);
                }
                if !self.charge(1, span) {
                    return self.empty(span);
                }
                let node = self.build_sequence(span);
                self.record_anchor(anchor, &node, self.nodes - before);
                node
            }
            Event::MappingStart(anchor, tag) => {
                if tag.is_some() {
                    self.reject_tag(span);
                }
                if !self.charge(1, span) {
                    return self.empty(span);
                }
                let node = self.build_mapping(span);
                self.record_anchor(anchor, &node, self.nodes - before);
                node
            }
            // An alias expands to its anchor's value, re-anchored on the alias
            // site so that a diagnostic about the expansion points at the
            // `*name` the author wrote. Spans inside the expansion keep pointing
            // at the anchor's definition, which is where that text actually
            // lives. The expansion is charged at the size of the tree it copies,
            // which is what makes an anchor/alias bomb a diagnostic rather than
            // an out-of-memory kill.
            Event::Alias(anchor) => {
                match self.anchors.get(&anchor).map(|anchored| anchored.nodes) {
                    Some(cost) => {
                        if !self.charge(cost, span) {
                            return self.empty(span);
                        }
                        let value = self
                            .anchors
                            .get(&anchor)
                            .map_or(Yaml::Null, |anchored| anchored.node.value.clone());
                        Node {
                            value,
                            span: self.span(span),
                        }
                    }
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
                }
            }
            // Structural events never reach `build`: the collection builders
            // consume their own terminators and `load_document` filters the
            // rest. Treat any straggler as an empty value rather than panicking.
            _ => self.empty(span),
        }
    }

    /// Charge `count` nodes against the expansion budget, reporting and latching
    /// the stream shut when it runs out.
    fn charge(&mut self, count: usize, span: RawSpan) -> bool {
        self.nodes = self.nodes.saturating_add(count);
        if self.nodes <= MAX_NODES {
            return true;
        }
        self.failed = true;
        let span = self.span(span);
        self.diagnostics.push(
            Diagnostic::error(
                DiagnosticCode::YamlSyntax,
                span,
                format!("invalid YAML: the document expands to more than {MAX_NODES} nodes"),
            )
            .with_help(
                "an alias copies its anchor's whole value, so anchoring each level on the one below multiplies the document at every step; write the values out instead",
            ),
        );
        false
    }

    fn empty(&self, span: RawSpan) -> Node {
        Node {
            value: Yaml::Null,
            span: self.span(span),
        }
    }

    fn build_sequence(&mut self, start: RawSpan) -> Node {
        self.depth += 1;
        let mut items = Vec::new();
        let mut content = None;
        let mut end = start;
        while let Some((event, span)) = self.next() {
            if matches!(event, Event::SequenceEnd) {
                end = span;
                break;
            }
            let item = self.build(event, span);
            reach(&mut content, &item.span);
            items.push(item);
        }
        self.depth -= 1;
        Node {
            value: Yaml::Sequence(items),
            span: self.collection_span(start, end, content),
        }
    }

    fn build_mapping(&mut self, start: RawSpan) -> Node {
        self.depth += 1;
        let mut mapping = Mapping::default();
        // Where each key already sits in `entries`, so the duplicate check
        // below costs one hash rather than a scan of everything read so far.
        // `Mapping` keeps its ordered vec — declaration order is what the IR
        // and every diagnostic are built on — and this index lives only for
        // the build. Scanning instead makes one large mapping quadratic, which
        // `MAX_NODES` does not bound: the cap limits how many nodes a file may
        // expand to, and a file well inside it would still take seconds, which
        // is not the millisecond budget the cap is there to protect
        // (PRD 5.12).
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut content = None;
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
            reach(&mut content, &key_node.span);
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
                    self.skip_value(&mut content);
                    mapping.dropped += 1;
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
                self.skip_value(&mut content);
                mapping.dropped += 1;
                continue;
            }

            let value = match self.next() {
                Some((event, span)) => self.build(event, span),
                None => break,
            };
            reach(&mut content, &value.span);

            if let Some(existing) = seen.get(&key.value).map(|index| &mapping.entries[*index]) {
                let first = existing.key.span.clone();
                self.diagnostics.push(
                    Diagnostic::error(
                        DiagnosticCode::DuplicateKey,
                        key.span,
                        format!("duplicate key `{}`", key.value),
                    )
                    .with_label(first, "first declared here")
                    .with_help("keys are declared once; the second declaration never wins"),
                );
                mapping.dropped += 1;
                continue;
            }
            seen.insert(key.value.clone(), mapping.entries.len());
            mapping.entries.push(Entry { key, value });
        }
        self.depth -= 1;
        Node {
            value: Yaml::Mapping(mapping),
            span: self.collection_span(start, end, content),
        }
    }

    /// Consume and discard the value of an entry whose key was rejected. Its
    /// text is still the enclosing mapping's own, so where it reaches to counts
    /// towards that mapping's span even though the value itself is dropped.
    fn skip_value(&mut self, content: &mut Option<(usize, Position)>) {
        if let Some((event, span)) = self.next() {
            let value = self.build(event, span);
            reach(content, &value.span);
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

    fn record_anchor(&mut self, anchor: usize, node: &Node, nodes: usize) {
        if anchor != 0 {
            self.anchors.insert(
                anchor,
                Anchor {
                    node: node.clone(),
                    nodes,
                },
            );
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

    /// The region a collection occupies: from its start event to the end of its
    /// own last piece of text.
    ///
    /// Not to its **end event**, which is where the *next* token begins. A block
    /// collection has no closing delimiter, so YAML ends it at whatever comes
    /// after — the following sibling's key, several lines down and past any
    /// comment between the two — and a span drawn to there underlines a
    /// declaration the diagnostic has nothing to say about, which is a
    /// diagnostic pointing at the wrong line (PRD G3). A flow collection ends at
    /// its `}` or `]` but its end event still runs on to the next token, so a
    /// trailing comment lands inside the span the same way.
    ///
    /// So the span ends at the furthest point the collection's children reach,
    /// extended over whatever separates that from the end event as long as it is
    /// whitespace or a comment. The extension is what keeps the `}` or `]` a
    /// flow collection closes with — the one piece of a collection's own text
    /// that lies past its last child — and a collection holding nothing at all
    /// (`{}`, `[]`) reaches from its opening delimiter by the same rule.
    fn collection_span(
        &self,
        start: RawSpan,
        end: RawSpan,
        content: Option<(usize, Position)>,
    ) -> Span {
        let start_byte = self.byte_of(start.start.index());
        let (floor, at) = match content {
            Some((byte, at)) if byte >= start_byte => (byte, at),
            _ => (
                self.byte_of(start.end.index()).max(start_byte),
                Self::position(start.end),
            ),
        };
        let limit = self.byte_of(end.end.index()).max(floor);
        let (end_byte, end_position) = self.past_trivia(floor, at, limit);
        Span::new(
            self.source.clone(),
            start_byte..end_byte,
            Self::position(start.start),
            end_position,
        )
    }

    /// Where the last thing in `from..limit` that is neither whitespace nor a
    /// comment ends, as a byte offset and the position just past it — `from`
    /// itself where the whole region is one or the other.
    ///
    /// The region is what lies between a collection's own content and the token
    /// that ends it, so a comment here is always a *trailing* comment: `#`
    /// begins one exactly where YAML says it does, after whitespace or at the
    /// start of a line, and a `#` inside a quoted scalar is inside a child's
    /// span and below `from`. The scan costs the region once per enclosing
    /// collection, and `MAX_NESTING` bounds how many of those there can be.
    fn past_trivia(&self, from: usize, at: Position, limit: usize) -> (usize, Position) {
        let Some(text) = self.text.get(from..limit) else {
            return (from, at);
        };
        let (mut byte, mut position) = (from, at);
        let (mut cursor, mut seen) = (from, at);
        let mut previous: Option<char> = None;
        let mut commented = false;
        for character in text.chars() {
            let next = if character == '\n' {
                Position::new(seen.line.saturating_add(1), 1)
            } else {
                Position::new(seen.line, seen.column.saturating_add(1))
            };
            cursor += character.len_utf8();
            if character == '\n' {
                commented = false;
            } else if commented || (character == '#' && previous.is_some_and(char::is_whitespace)) {
                commented = true;
            } else if !character.is_whitespace() {
                byte = cursor;
                position = next;
            }
            previous = Some(character);
            seen = next;
        }
        (byte, position)
    }
}

/// Carry the furthest point a collection's children reach, as the byte just past
/// one and the position just past it.
fn reach(content: &mut Option<(usize, Position)>, span: &Span) {
    if content.is_none_or(|(byte, _)| span.bytes.end > byte) {
        *content = Some((span.bytes.end, span.end));
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
/// rather than silently truncated or demoted to a string — no legal spec value
/// is affected, and the value still reports as a number.
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
        return value;
    }
    if let Some(value) = parse_core_float(text) {
        return Yaml::Float(value);
    }
    Yaml::String(text.to_owned())
}

/// `[-+]?[0-9]+`, `0o[0-7]+`, or `0x[0-9a-fA-F]+`.
///
/// Matching the production is what decides that the scalar is a number; whether
/// it *fits* an `i64` only decides which number. One too wide stays a number, as
/// a float — falling through to a string would make `123…456` a legal value
/// wherever a string is expected, and report "found a string" wherever a number
/// is.
fn parse_core_int(text: &str) -> Option<Yaml> {
    if let Some(digits) = text.strip_prefix("0x") {
        return radix_int(digits, 16);
    }
    if let Some(digits) = text.strip_prefix("0o") {
        return radix_int(digits, 8);
    }
    let digits = text.strip_prefix(['-', '+']).unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let text = text.trim_start_matches('+');
    Some(match text.parse::<i64>() {
        Ok(value) => Yaml::Int(value),
        // Wider than an `i64`. `f64` accepts every decimal digit string, and the
        // digits were validated above, so the fallback cannot itself fail.
        Err(_) => Yaml::Float(text.parse::<f64>().unwrap_or(f64::NAN)),
    })
}

/// A `0x`/`0o` integer body, once its prefix is stripped.
fn radix_int(digits: &str, radix: u32) -> Option<Yaml> {
    if digits.is_empty() || !digits.chars().all(|digit| digit.is_digit(radix)) {
        return None;
    }
    Some(match i64::from_str_radix(digits, radix) {
        Ok(value) => Yaml::Int(value),
        Err(_) => Yaml::Float(digits.chars().fold(0.0_f64, |value, digit| {
            value * f64::from(radix) + f64::from(digit.to_digit(radix).unwrap_or_default())
        })),
    })
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
    fn an_integer_wider_than_i64_stays_a_number() {
        // Matching the integer production is what makes the scalar a number;
        // the width of an `i64` only decides which kind of number it becomes.
        assert_eq!(
            resolve_scalar("123456789012345678901234567890", ScalarStyle::Plain),
            Yaml::Float(1.234_567_890_123_456_8e29)
        );
        assert_eq!(
            resolve_scalar("-99999999999999999999", ScalarStyle::Plain),
            Yaml::Float(-1e20)
        );
        // Eighteen `F`s: 2^72 - 1.
        assert_eq!(
            resolve_scalar("0xFFFFFFFFFFFFFFFFFF", ScalarStyle::Plain),
            Yaml::Float(4.722_366_482_869_645e21)
        );
        assert_eq!(
            resolve_scalar(&i64::MAX.to_string(), ScalarStyle::Plain),
            Yaml::Int(i64::MAX)
        );
        // Still not integers, and still not numbers.
        assert_eq!(
            resolve_scalar("0x", ScalarStyle::Plain),
            Yaml::String("0x".into())
        );
        assert_eq!(
            resolve_scalar("0o99", ScalarStyle::Plain),
            Yaml::String("0o99".into())
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

    /// A block collection has no closing delimiter, so YAML ends it where the
    /// next token begins — the following sibling's key, past any comment
    /// between the two. A span drawn to *there* underlines a declaration the
    /// diagnostic naming it has nothing to say about, so a collection's span
    /// stops at its own last value instead (PRD G3).
    #[test]
    fn a_block_collection_ends_at_its_own_last_value() {
        let source = "a:\n  b: 1\n  c: 2\n\n# a comment about d\nd: 3\n";
        let (node, diagnostics) = load_ok(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let root = node.unwrap();
        let block = root.as_mapping().unwrap().get("a").unwrap();
        assert_eq!(&source[block.span.bytes.clone()], "b: 1\n  c: 2");
        assert_eq!(block.span.start, Position::new(2, 3));
        assert_eq!(block.span.end, Position::new(3, 7));

        // The root runs to its own last value the same way, dropping the
        // trailing newline the file ends with.
        assert_eq!(root.span.end, Position::new(6, 5));
        assert_eq!(&source[root.span.bytes.clone()], source.trim_end());

        // A sequence is the same shape.
        let source = "a:\n  - one\n  - two\nb: 3\n";
        let (node, _) = load_ok(source);
        let items = node
            .unwrap()
            .as_mapping()
            .unwrap()
            .get("a")
            .unwrap()
            .span
            .clone();
        assert_eq!(&source[items.bytes.clone()], "- one\n  - two");
        assert_eq!(items.end, Position::new(3, 8));
    }

    /// A flow collection *does* close itself, and its `}` or `]` is the one
    /// piece of its text past its last child — so the span keeps that and stops
    /// there, whatever trailing comment the end event runs on to.
    #[test]
    fn a_flow_collection_keeps_its_delimiter_and_drops_a_trailing_comment() {
        let source = "a: { b: [1, 2] }   # trailing\nc: 3\n";
        let (node, diagnostics) = load_ok(source);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        let root = node.unwrap();
        let mapping = root.as_mapping().unwrap().get("a").unwrap();
        assert_eq!(&source[mapping.span.bytes.clone()], "{ b: [1, 2] }");
        assert_eq!(mapping.span.end, Position::new(1, 17));
        let sequence = mapping.as_mapping().unwrap().get("b").unwrap();
        assert_eq!(&source[sequence.span.bytes.clone()], "[1, 2]");

        // A collection that holds nothing reaches from its opening delimiter.
        let source = "a: {}\nb: []\n";
        let (node, _) = load_ok(source);
        let root = node.unwrap();
        let root = root.as_mapping().unwrap();
        assert_eq!(&source[root.get("a").unwrap().span.bytes.clone()], "{}");
        assert_eq!(&source[root.get("b").unwrap().span.bytes.clone()], "[]");
    }

    /// An entry the loader drops still leaves its text inside the mapping that
    /// declared it: the span reaches over a duplicate rather than stopping at
    /// the last entry that survived.
    #[test]
    fn a_dropped_entry_still_counts_towards_the_mapping_it_was_written_in() {
        let source = "a:\n  b: 1\n  b: 2\nc: 3\n";
        let (node, diagnostics) = load_ok(source);
        assert_eq!(codes(&diagnostics), ["duplicate-key"]);
        let root = node.unwrap();
        let block = root.as_mapping().unwrap().get("a").unwrap();
        assert_eq!(&source[block.span.bytes.clone()], "b: 1\n  b: 2");
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

    /// A dropped entry is counted, so a rule stated over how many entries a
    /// mapping has can tell "the author wrote `{}`" from "the author wrote one
    /// entry that did not survive" — the second already has its own diagnostic.
    #[test]
    fn a_dropped_entry_is_not_an_empty_mapping() {
        for source in ["1: a\n", "<<: a\n"] {
            let (node, diagnostics) = load_ok(source);
            assert_eq!(diagnostics.len(), 1, "{source:?}");
            let mapping = node.unwrap();
            let mapping = mapping.as_mapping().unwrap();
            assert!(mapping.is_empty(), "{source:?}");
            assert_eq!(mapping.dropped(), 1, "{source:?}");
            assert!(!mapping.declares_nothing(), "{source:?}");
        }

        let (node, diagnostics) = load_ok("a: 1\na: 2\n");
        assert_eq!(codes(&diagnostics), ["duplicate-key"]);
        let mapping = node.unwrap();
        let mapping = mapping.as_mapping().unwrap();
        assert_eq!(mapping.len(), 1);
        assert_eq!(mapping.dropped(), 1);

        let (node, diagnostics) = load_ok("a: {}\n");
        assert!(diagnostics.is_empty());
        let root = node.unwrap();
        let empty = root.as_mapping().unwrap().get("a").unwrap();
        assert!(empty.as_mapping().unwrap().declares_nothing());
    }

    /// `level` levels of eight-way aliasing, each anchored on the one below.
    fn alias_pyramid(levels: usize) -> String {
        let mut source = String::from("l0: &l0 [x, x, x, x, x, x, x, x]\n");
        for level in 1..=levels {
            let alias = format!("*l{}", level - 1);
            let row = [alias.as_str(); 8].join(", ");
            source.push_str(&format!("l{level}: &l{level} [{row}]\n"));
        }
        source
    }

    #[test]
    fn caps_the_total_expansion_of_aliases() {
        // Each level copies the whole level below it, so this multiplies by
        // eight per line: seven levels is 2.4M nodes and eight is 19M, from
        // under 400 bytes of input. `MAX_NESTING` never fires — the document is
        // two levels deep — so only the expansion budget stands between a
        // hostile file and an out-of-memory kill.
        let (node, diagnostics) = load_ok(&alias_pyramid(8));
        assert!(
            node.is_none(),
            "a truncated tree is not handed to the parser"
        );
        assert_eq!(codes(&diagnostics), ["yaml-syntax"]);
        assert!(
            diagnostics[0].message.contains("expands to more than"),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn leaves_room_for_aliases_used_as_aliases() {
        // Four levels is 42,797 nodes — far more repetition than a hand-written
        // spec carries, and well inside the budget. The cap exists to stop
        // multiplication, not to make anchors unusable (grammar 1.1, D49).
        let (node, diagnostics) = load_ok(&alias_pyramid(4));
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert!(node.is_some());
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

    /// A flat mapping of `keys` distinct entries, its last one repeating its
    /// first so the duplicate check is exercised rather than merely skipped.
    fn flat_mapping(keys: usize) -> String {
        let mut source = String::from("version: \"0.1\"\n");
        for index in 0..keys {
            source.push_str(&format!("k{index}: 1\n"));
        }
        source.push_str("k0: 2\n");
        source
    }

    /// The duplicate-key check reports the *first* occurrence, whatever else it
    /// has read: a lookup index has to keep pointing at the earlier entry, not
    /// at whichever one it happens to find.
    #[test]
    fn a_duplicate_names_the_first_declaration_in_a_large_mapping() {
        let (node, diagnostics) = load_ok(&flat_mapping(5_000));
        assert_eq!(codes(&diagnostics), ["duplicate-key"]);
        assert_eq!(diagnostics[0].message, "duplicate key `k0`");
        assert_eq!(diagnostics[0].labels[0].span.start.line, 2);
        assert_eq!(
            node.expect("a large mapping still loads")
                .as_mapping()
                .expect("the root is a mapping")
                .len(),
            5_001
        );
    }

    /// …and it must not cost a scan of everything read so far to do it.
    ///
    /// `MAX_NODES` bounds how far a document may *expand*, not how long reading
    /// one takes, and the two are not the same guarantee: a flat mapping well
    /// inside the cap took seconds when every insert rescanned its
    /// predecessors, which is not the millisecond budget the cap exists to
    /// protect (PRD 5.12).
    ///
    /// The measurement is a *ratio* rather than a wall-clock bound, because a
    /// bound tight enough to catch quadratic growth on a fast machine is one a
    /// loaded CI machine trips on for no reason. Quadrupling the key count
    /// quadruples linear work and multiplies quadratic work by sixteen, so a
    /// threshold of eight sits a clear factor of two from either. The one-second
    /// floor keeps a fast host, where both readings are noise, from failing on
    /// the ratio of two noise samples.
    #[test]
    fn a_large_mapping_does_not_cost_a_scan_per_key() {
        fn read(keys: usize) -> std::time::Duration {
            let source = flat_mapping(keys);
            let started = std::time::Instant::now();
            let (node, _) = load_ok(&source);
            assert!(node.is_some(), "{keys} keys must still load");
            started.elapsed()
        }

        let small = read(8_000);
        let large = read(32_000);
        let budget = (small * 8).max(std::time::Duration::from_secs(1));
        assert!(
            large < budget,
            "8,000 keys took {small:?} and 32,000 took {large:?}: four times the keys \
             should cost about four times the work, so the duplicate check has gone \
             quadratic again"
        );
    }
}
