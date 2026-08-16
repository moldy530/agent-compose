//! How the IR writes the leaves: source coordinates, lexical forms, closed
//! vocabularies, and literal data.
//!
//! The IR reuses the AST's lexical types rather than re-declaring them
//! ([`Ident`], [`Address`], [`Cel`], [`PathExpr`], [`Duration`], [`EnvRef`],
//! [`Interpolated`], [`Literal`], and every closed keyword enum). They carry no
//! `Invalid` variant and no required-but-absent field, so there is nothing for a
//! lowering step to prove about them and a parallel set of types would only be a
//! place for the two to drift. What this module owns is the *encoding*: the
//! JSON shape each of them takes. Keeping those `impl`s here rather than on the
//! AST is what leaves the AST free of the artifact's contract — the two are in
//! one crate, so coherence allows it.
//!
//! # Source coordinates
//!
//! A [`Span`] is written as one string:
//!
//! ```text
//! flows/review_loop.yml:14:5..14:22
//! ```
//!
//! — the file, then the start and end positions as `line:column`, both 1-based,
//! columns in characters. Three properties decide the form:
//!
//! * **The file is the composition-relative path** the resolver read it under,
//!   `/`-separated, so the same spelling appears in the IR, on a command line,
//!   and in a diagnostic on every host (grammar 1.4). An absolute path would
//!   make the artifact reproduce differently on two machines, which PRD 5.12
//!   forbids.
//! * **Byte offsets are not written.** They shift with every edit above a span,
//!   so an artifact carrying them would diff wholesale on an unrelated one-line
//!   insertion — and the artifact is the thing PRD 5.1 asks to be diffable. The
//!   in-memory [`Span`] keeps its byte range, so the pass that reads the IR
//!   inside this compiler still has it; only the emitted document drops it.
//! * **One string rather than a nested object.** Nearly every node of the IR
//!   carries a span, so the difference between five keys and one is the
//!   difference between an artifact a reader can follow and one buried in
//!   coordinates.
//!
//! [`parse_span`] is the inverse, and `span_round_trips` pins the pair.
//!
//! # Environment references
//!
//! Env refs survive **unresolved** into the IR and are never substituted here
//! (PRD 5.8, 5.9; grammar 4.3). Both forms are written as objects rather than as
//! bare strings, so a consumer never has to decide whether a string might carry
//! one:
//!
//! ```json
//! { "env_ref": "ANTHROPIC_API_KEY" }
//! { "text": "https://${SEARCH_HOST}/v1/search", "env_refs": ["SEARCH_HOST"] }
//! ```
//!
//! `text` is the string **exactly as written**, escapes included: `$${` is the
//! escape for a literal `${`, and it survives into `text` unexpanded. So
//! `"https://example.test/$${NOT_A_REF}"` is written with its `$${` intact and
//! *no* `env_refs` key at all. `env_refs` — not a scan of `text` — is the
//! authoritative list of what a consumer substitutes: it holds every name the
//! string really carries, and is omitted when the string carries none.

use serde::ser::{SerializeMap, SerializeStruct};
use serde::{Serialize, Serializer};

use crate::ast::binding::HttpMethod;
use crate::ast::common::{
    Address, Cel, ControlTarget, Duration, EdgeSource, EdgeTarget, EnvRef, Ident, Interpolated,
    Literal, PathExpr,
};
use crate::ast::definition::{AgentAccess, ProviderKind, RouteCondition, StoreKind, StoreScope};
use crate::ast::deploy::{BackendProvider, EventSourceKind, Network, PluginValue, Runtime};
use crate::ast::document::Reduce;
use crate::ast::flow::{FlowContext, StoreOp};
use crate::ast::schema::{Number, ScalarKind, StringFormat, Surface};
use crate::ast::trigger::{Respond, TriggerMethod};
use crate::diag::{Position, Span, Spanned};

/// Write a span the way the IR records it: `<file>:<line>:<col>..<line>:<col>`.
#[must_use]
pub fn encode_span(span: &Span) -> String {
    format!(
        "{}:{}:{}..{}:{}",
        span.source, span.start.line, span.start.column, span.end.line, span.end.column
    )
}

/// Read back a span written by [`encode_span`], with an empty byte range.
///
/// The byte range is not part of the encoding (see the module docs), so a span
/// recovered from an emitted artifact carries `0..0`. Nothing inside this
/// compiler round-trips through the text — the IR is held as a value — so this
/// exists for consumers of the document and for the test that pins the format.
#[must_use]
pub fn parse_span(text: &str) -> Option<Span> {
    // Split from the right: a file name may itself contain a colon, while the
    // four coordinates never do.
    let (start, end) = text.rsplit_once("..")?;
    let (start, start_column) = start.rsplit_once(':')?;
    let (file, start_line) = start.rsplit_once(':')?;
    let (end_line, end_column) = end.split_once(':')?;
    if file.is_empty() {
        return None;
    }
    Some(Span::new(
        file.into(),
        0..0,
        Position::new(start_line.parse().ok()?, start_column.parse().ok()?),
        Position::new(end_line.parse().ok()?, end_column.parse().ok()?),
    ))
}

impl Serialize for Span {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&encode_span(self))
    }
}

/// A spanned value is `{ "value": …, "span": "…" }` — the value first, because
/// that is what a reader is looking for.
impl<T: Serialize> Serialize for Spanned<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("Spanned", 2)?;
        state.serialize_field("value", &self.value)?;
        state.serialize_field("span", &self.span)?;
        state.end()
    }
}

/// Write these as the keyword that names them: they are closed vocabularies
/// whose spelling in the DSL is the whole of their content.
macro_rules! serialize_as_keyword {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Serialize for $ty {
                fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                    serializer.serialize_str(self.as_str())
                }
            }
        )*
    };
}

serialize_as_keyword!(
    AgentAccess,
    BackendProvider,
    EventSourceKind,
    FlowContext,
    HttpMethod,
    Network,
    ProviderKind,
    Reduce,
    Respond,
    RouteCondition,
    Runtime,
    ScalarKind,
    StoreKind,
    StoreOp,
    StoreScope,
    StringFormat,
    Surface,
    TriggerMethod,
);

/// Written as the text the author wrote: an identifier, an address, and the
/// three expression forms are each exactly their own spelling.
macro_rules! serialize_as_text {
    ($($ty:ty),* $(,)?) => {
        $(
            impl Serialize for $ty {
                fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                    serializer.serialize_str(self.as_str())
                }
            }
        )*
    };
}

serialize_as_text!(Cel, Duration, Ident, PathExpr);

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

/// The three surfaces that name a node write it as the id, `start`, or `end`.
/// A node id may be neither pseudo-node (grammar 2.4), so one string is
/// unambiguous.
impl Serialize for ControlTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Node(id) => serializer.serialize_str(id.as_str()),
            Self::End => serializer.serialize_str("end"),
        }
    }
}

impl Serialize for EdgeSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Node(id) => serializer.serialize_str(id.as_str()),
            Self::Start => serializer.serialize_str("start"),
        }
    }
}

impl Serialize for EdgeTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Node(id) => serializer.serialize_str(id.as_str()),
            Self::End => serializer.serialize_str("end"),
        }
    }
}

impl Serialize for Number {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Int(value) => serializer.serialize_i64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
        }
    }
}

/// An environment reference in value form, kept unresolved: `{"env_ref": NAME}`.
impl Serialize for EnvRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("EnvRef", 1)?;
        state.serialize_field("env_ref", &self.name)?;
        state.end()
    }
}

/// An interpolable string, kept unsubstituted, with the names it embeds:
/// `{"text": …, "env_refs": [NAME, …]}`. `text` is the string as authored, so an
/// escaped `$${NAME}` appears in it verbatim and contributes no name; the list
/// is what says which tokens are real, and is omitted when there are none (see
/// the module docs).
impl Serialize for Interpolated {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let fields = if self.references.is_empty() { 1 } else { 2 };
        let mut state = serializer.serialize_struct("Interpolated", fields)?;
        state.serialize_field("text", self.as_str())?;
        if !self.references.is_empty() {
            state.serialize_field("env_refs", &self.references)?;
        }
        state.end()
    }
}

/// A literal is written as the data it is.
///
/// Nothing inside a `default:` or a `settings:` value is a reference to
/// anything — grammar 4.3 puts both in class 3 — so the sub-spans the parser
/// records for them buy a later pass nothing it cannot get from the literal's
/// own span, and writing them would turn readable data into a tree of
/// two-key objects.
impl Serialize for Literal {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Int(value) => serializer.serialize_i64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::String(value) => serializer.serialize_str(value),
            Self::Sequence(items) => serializer.collect_seq(items.iter().map(|item| &item.value)),
            Self::Mapping(entries) => serializer.collect_map(
                entries
                    .iter()
                    .map(|entry| (&entry.key.value, &entry.value.value)),
            ),
        }
    }
}

/// A plugin-config value is data too, with one difference that is the whole
/// point of the type: its strings are grammar 4.3 class 2, so each one is
/// written as an [`Interpolated`] node carrying the references it embeds rather
/// than as a bare string (grammar 14.2, 14.3).
impl Serialize for PluginValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => serializer.serialize_unit(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Int(value) => serializer.serialize_i64(*value),
            Self::Float(value) => serializer.serialize_f64(*value),
            Self::Text(text) => text.serialize(serializer),
            Self::Sequence(items) => serializer.collect_seq(items.iter().map(|item| &item.value)),
            Self::Mapping(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for entry in entries {
                    map.serialize_entry(&entry.key.value, &entry.value.value)?;
                }
                map.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(source: &str) -> Span {
        Span::new(
            source.into(),
            10..24,
            Position::new(3, 5),
            Position::new(4, 12),
        )
    }

    #[test]
    fn spans_are_one_string_naming_the_file_and_both_ends() {
        assert_eq!(
            encode_span(&span("flows/review_loop.yml")),
            "flows/review_loop.yml:3:5..4:12"
        );
        assert_eq!(
            serde_json::to_string(&span("main.yml")).expect("a span serializes"),
            "\"main.yml:3:5..4:12\""
        );
    }

    /// The encoding round-trips, including through a file name carrying a colon
    /// — which is why [`parse_span`] splits from the right.
    #[test]
    fn span_round_trips() {
        for name in ["main.yml", "a/b/c.yml", "weird:name.yml"] {
            let original = span(name);
            let parsed = parse_span(&encode_span(&original)).expect("the encoding parses");
            assert_eq!(parsed.source.as_str(), name);
            assert_eq!(parsed.start, original.start);
            assert_eq!(parsed.end, original.end);
            assert_eq!(parsed.bytes, 0..0, "byte offsets are not part of the form");
        }
        assert_eq!(parse_span("main.yml:3:5"), None);
        assert_eq!(parse_span(":3:5..4:12"), None);
    }

    #[test]
    fn spanned_values_carry_the_value_first() {
        let value = Spanned::new(Ident::new("draft"), span("main.yml"));
        assert_eq!(
            serde_json::to_string(&value).expect("a spanned identifier serializes"),
            r#"{"value":"draft","span":"main.yml:3:5..4:12"}"#
        );
    }

    #[test]
    fn environment_references_stay_unresolved() {
        let value = EnvRef::new("${ANTHROPIC_API_KEY}", "ANTHROPIC_API_KEY");
        assert_eq!(
            serde_json::to_string(&value).expect("an env ref serializes"),
            r#"{"env_ref":"ANTHROPIC_API_KEY"}"#
        );

        let url = Interpolated::new(
            "https://${SEARCH_HOST}/v1/search",
            vec!["SEARCH_HOST".to_string()],
        );
        assert_eq!(
            serde_json::to_string(&url).expect("an interpolated string serializes"),
            r#"{"text":"https://${SEARCH_HOST}/v1/search","env_refs":["SEARCH_HOST"]}"#
        );

        let plain = Interpolated::new("npm", Vec::new());
        assert_eq!(
            serde_json::to_string(&plain).expect("an interpolated string serializes"),
            r#"{"text":"npm"}"#
        );
    }

    /// `text` is the string as authored, escapes and all: an escaped `$${NAME}`
    /// stays escaped and contributes no name. A consumer that substituted by
    /// scanning `text` for `${…}` would expand the one token the author wrote in
    /// order *not* to expand, which is why `env_refs` is the authoritative list.
    #[test]
    fn an_escaped_token_is_written_as_authored_and_names_nothing() {
        let escaped = Interpolated::new("https://example.test/$${NOT_A_REF}", Vec::new());
        assert_eq!(
            serde_json::to_string(&escaped).expect("an interpolated string serializes"),
            r#"{"text":"https://example.test/$${NOT_A_REF}"}"#
        );
    }

    #[test]
    fn literals_are_written_as_data() {
        let literal = Literal::Mapping(vec![
            crate::ast::common::LiteralEntry {
                key: Spanned::new("budget_tokens".to_string(), span("m.yml")),
                value: Spanned::new(Literal::Int(4000), span("m.yml")),
            },
            crate::ast::common::LiteralEntry {
                key: Spanned::new("enabled".to_string(), span("m.yml")),
                value: Spanned::new(Literal::Bool(true), span("m.yml")),
            },
        ]);
        assert_eq!(
            serde_json::to_string(&literal).expect("a literal serializes"),
            r#"{"budget_tokens":4000,"enabled":true}"#
        );
    }

    #[test]
    fn pseudo_nodes_and_ids_share_one_spelling() {
        assert_eq!(
            serde_json::to_string(&EdgeSource::Start).expect("an edge source serializes"),
            "\"start\""
        );
        assert_eq!(
            serde_json::to_string(&EdgeTarget::Node(Ident::new("review")))
                .expect("an edge target serializes"),
            "\"review\""
        );
        assert_eq!(
            serde_json::to_string(&ControlTarget::End).expect("a control target serializes"),
            "\"end\""
        );
    }
}
