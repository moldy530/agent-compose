//! Reading YAML mappings with diagnostics: the machinery every construct's
//! parser is written against.
//!
//! [`Fields`] wraps one mapping and tracks which keys were consumed, so that
//! [`Fields::finish`] can report the leftovers as unknown keys — with a
//! "did you mean" drawn from the keys the construct actually reads. That is
//! what makes a typo in `retrry:` a diagnostic instead of a silently ignored
//! key (Decision D50, PRD G3).

use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Span, Spanned};
use crate::yaml::{Entry, Mapping, Node, Yaml};

/// Shared parsing state: everywhere a diagnostic can be raised.
pub(crate) struct Cx<'dx> {
    diagnostics: &'dx mut Diagnostics,
}

impl<'dx> Cx<'dx> {
    pub(crate) fn new(diagnostics: &'dx mut Diagnostics) -> Self {
        Self { diagnostics }
    }

    pub(crate) fn push(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn error(&mut self, code: DiagnosticCode, span: &Span, message: impl Into<String>) {
        self.diagnostics
            .push(Diagnostic::error(code, span.clone(), message));
    }

    /// `expected {expectation} for {subject}, found {found}`.
    pub(crate) fn wrong_type(&mut self, node: &Node, subject: &str, expectation: &str) {
        self.error(
            DiagnosticCode::WrongType,
            &node.span,
            format!(
                "expected {expectation} for {subject}, found {}",
                node.description()
            ),
        );
    }
}

/// A mapping being read key by key.
pub(crate) struct Fields<'a> {
    mapping: &'a Mapping,
    /// The mapping's own span.
    pub(crate) span: Span,
    /// How the construct is named in diagnostics ("node `review`").
    context: String,
    consumed: Vec<bool>,
    known: Vec<&'static str>,
}

impl<'a> Fields<'a> {
    pub(crate) fn new(mapping: &'a Mapping, span: Span, context: impl Into<String>) -> Self {
        Self {
            mapping,
            span,
            context: context.into(),
            consumed: vec![false; mapping.len()],
            known: Vec::new(),
        }
    }

    /// Register keys the construct knows about without reading them, so that
    /// [`Self::finish`] does not report them as unknown and can suggest them.
    /// Used where legality is decided elsewhere — a store op's parameter row,
    /// for instance (grammar 11.4).
    pub(crate) fn note_known(&mut self, keys: &[&'static str]) {
        self.known.extend_from_slice(keys);
    }

    /// Whether the key is present, without consuming it.
    pub(crate) fn contains(&self, key: &str) -> bool {
        self.mapping.contains_key(key)
    }

    /// The span of this key, without consuming it.
    ///
    /// For the rules that are about a key having been *written* — a conditional
    /// requirement decided by a sibling — and so read the mapping through
    /// [`Self::contains`] rather than through a parsed value, but still want to
    /// underline the sibling that decided it.
    pub(crate) fn span_of(&self, key: &str) -> Option<Span> {
        self.mapping
            .entries()
            .iter()
            .find(|entry| entry.key.value == key)
            .map(|entry| entry.key.span.clone())
    }

    /// The entry under this key, marking it consumed.
    pub(crate) fn take_entry(&mut self, key: &'static str) -> Option<&'a Entry> {
        if !self.known.contains(&key) {
            self.known.push(key);
        }
        let index = self
            .mapping
            .entries()
            .iter()
            .position(|entry| entry.key.value == key)?;
        self.consumed[index] = true;
        Some(&self.mapping.entries()[index])
    }

    /// The value under this key, marking it consumed.
    pub(crate) fn take(&mut self, key: &'static str) -> Option<&'a Node> {
        self.take_entry(key).map(|entry| &entry.value)
    }

    /// The value under this key, reporting a missing-key diagnostic when it is
    /// absent.
    pub(crate) fn require(&mut self, key: &'static str, cx: &mut Cx) -> Option<&'a Node> {
        let found = self.take(key);
        if found.is_none() {
            cx.error(
                DiagnosticCode::MissingKey,
                &self.span,
                format!("missing required key `{key}` in {}", self.context),
            );
        }
        found
    }

    /// The string under this key.
    pub(crate) fn string(&mut self, key: &'static str, cx: &mut Cx) -> Option<Spanned<String>> {
        let node = self.take(key)?;
        expect_string(node, &format!("`{key}`"), cx)
    }

    /// The integer under this key.
    pub(crate) fn integer(&mut self, key: &'static str, cx: &mut Cx) -> Option<Spanned<i64>> {
        let node = self.take(key)?;
        expect_integer(node, &format!("`{key}`"), cx)
    }

    /// The boolean under this key.
    pub(crate) fn boolean(&mut self, key: &'static str, cx: &mut Cx) -> Option<Spanned<bool>> {
        let node = self.take(key)?;
        match &node.value {
            Yaml::Bool(value) => Some(Spanned::new(*value, node.span.clone())),
            _ => {
                cx.wrong_type(node, &format!("`{key}`"), "a boolean");
                None
            }
        }
    }

    /// Mark every remaining key consumed.
    ///
    /// Used where a diagnostic has already explained the whole mapping — a type
    /// node that declares no type, a store op nobody recognises — and reporting
    /// its keys one by one as "unknown" would bury that explanation.
    pub(crate) fn consume_rest(&mut self) {
        self.consumed.iter_mut().for_each(|seen| *seen = true);
    }

    /// Report every key the construct never read.
    pub(crate) fn finish(self, cx: &mut Cx) {
        for (index, entry) in self.mapping.entries().iter().enumerate() {
            if self.consumed[index] {
                continue;
            }
            let suggestion = suggest(&entry.key.value, &self.known);
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownKey,
                    entry.key.span.clone(),
                    format!("unknown key `{}` in {}", entry.key.value, self.context),
                )
                .with_optional_help(suggestion.map(|key| format!("did you mean `{key}`?"))),
            );
        }
    }
}

/// The string this node holds.
pub(crate) fn expect_string(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<String>> {
    match &node.value {
        Yaml::String(text) => Some(Spanned::new(text.clone(), node.span.clone())),
        _ => {
            cx.wrong_type(node, subject, "a string");
            None
        }
    }
}

/// The integer this node holds.
pub(crate) fn expect_integer(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<i64>> {
    match &node.value {
        Yaml::Int(value) => Some(Spanned::new(*value, node.span.clone())),
        _ => {
            cx.wrong_type(node, subject, "an integer");
            None
        }
    }
}

/// The mapping this node holds.
pub(crate) fn expect_mapping<'a>(
    node: &'a Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<&'a Mapping> {
    match node.as_mapping() {
        Some(mapping) => Some(mapping),
        None => {
            cx.wrong_type(node, subject, "a mapping");
            None
        }
    }
}

/// The sequence this node holds.
pub(crate) fn expect_sequence<'a>(
    node: &'a Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<&'a [Node]> {
    match node.as_sequence() {
        Some(items) => Some(items),
        None => {
            cx.wrong_type(node, subject, "a sequence");
            None
        }
    }
}

/// Check an integer against an inclusive range, reporting when it falls
/// outside.
pub(crate) fn in_range(
    value: &Spanned<i64>,
    subject: &str,
    range: std::ops::RangeInclusive<i64>,
    cx: &mut Cx,
) -> bool {
    if range.contains(&value.value) {
        return true;
    }
    cx.error(
        DiagnosticCode::ValueOutOfRange,
        &value.span,
        format!(
            "{subject} must be between {} and {}, found {}",
            range.start(),
            range.end(),
            value.value
        ),
    );
    false
}

/// Check an integer against a lower bound alone.
///
/// The grammar states several constraints as `integer ≥ 0` or `integer ≥ 1`
/// with no upper bound; saying so beats naming `i64::MAX` as if the width of the
/// representation were the rule (PRD G3).
pub(crate) fn at_least(value: &Spanned<i64>, subject: &str, minimum: i64, cx: &mut Cx) -> bool {
    if value.value >= minimum {
        return true;
    }
    cx.error(
        DiagnosticCode::ValueOutOfRange,
        &value.span,
        format!(
            "{subject} must be at least {minimum}, found {}",
            value.value
        ),
    );
    false
}

/// Check that a number is one a spec file can actually carry.
///
/// YAML's core schema resolves `.inf` and `.nan`, and a decimal literal such as
/// `1e400` — or a plain integer wider than an `f64` — overflows to infinity on
/// the way in. None of the three survive the pipeline: every number a spec
/// declares is lowered into the flat IR and from there into JSON Schema
/// 2020-12 and generated code (grammar 3.8), and JSON has no notation for
/// infinity or NaN. They also slip past the ordinary bound checks — `.inf`
/// satisfies "greater than 0" and NaN satisfies every comparison by failing it
/// — so the parser rejects them where they are written, while there is still a
/// span to point at (PRD G3).
pub(crate) fn expect_finite(value: f64, subject: &str, span: &Span, cx: &mut Cx) -> bool {
    if value.is_finite() {
        return true;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            span.clone(),
            format!(
                "{subject} must be a finite number, found {}",
                non_finite_name(value)
            ),
        )
        .with_help(
            "the compiler lowers every declared number to JSON Schema 2020-12 and to generated code (grammar 3.8), neither of which can write infinity or NaN",
        ),
    );
    false
}

/// How a non-finite `f64` is named in a diagnostic.
fn non_finite_name(value: f64) -> &'static str {
    if value.is_nan() {
        "NaN"
    } else if value.is_sign_positive() {
        "infinity"
    } else {
        "-infinity"
    }
}

/// The closest candidate to `actual`, if one is close enough to be worth
/// suggesting.
///
/// The threshold is the usual one third of the longer string (at least one
/// edit), computed over Damerau-Levenshtein distance so that a transposition —
/// `retyr` for `retry` — counts as a single edit.
///
/// A candidate equal to `actual` is never one: "unknown key `default` … did you
/// mean `default`?" tells an author nothing, and every caller reaches here
/// having already refused the name it passes, so an identity match means the
/// candidate list is wider than the surface rather than that the spelling was
/// close (PRD G3).
pub(crate) fn suggest<'k>(actual: &str, candidates: &[&'k str]) -> Option<&'k str> {
    let limit = (actual.chars().count() / 3).max(1);
    candidates
        .iter()
        .filter(|candidate| **candidate != actual)
        .filter_map(|candidate| {
            let allowed = limit.max(candidate.chars().count() / 3);
            let distance = strsim::damerau_levenshtein(actual, candidate);
            (distance <= allowed).then_some((distance, *candidate))
        })
        .min_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, candidate)| candidate)
}

/// `a` or `an`, so a sentence a compiler composes reads like one.
///
/// Read off the word's first letter, which is all a vocabulary this compiler
/// owns needs: every word it reaches for is a keyword of the grammar
/// (`openai_compatible`, `azure_openai`, `bedrock`), never English at large.
pub(crate) fn article(word: &str) -> &'static str {
    if word.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    }
}

/// Format a closed vocabulary for a diagnostic: ``` `a`, `b`, `c` ```.
pub(crate) fn list(values: impl IntoIterator<Item = impl AsRef<str>>) -> String {
    values
        .into_iter()
        .map(|value| format!("`{}`", value.as_ref()))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suggests_near_misses_only() {
        let keys = ["model", "prompt", "output", "input", "tools", "retry"];
        assert_eq!(suggest("retrry", &keys), Some("retry"));
        assert_eq!(suggest("promt", &keys), Some("prompt"));
        assert_eq!(suggest("outputs", &keys), Some("output"));
        assert_eq!(suggest("retyr", &keys), Some("retry"));
        assert_eq!(suggest("entrypoint", &keys), None);
        assert_eq!(suggest("", &keys), None);
    }

    /// A key that *is* one of the candidates is not a near miss. It reaches
    /// `suggest` only when the surface knows the key but refused it for another
    /// reason, and echoing it back is the one suggestion guaranteed to be
    /// useless.
    #[test]
    fn never_suggests_the_word_it_was_given() {
        let keys = ["node", "route_by", "routes", "default"];
        assert_eq!(suggest("default", &keys), None);
        assert_eq!(suggest("defualt", &keys), Some("default"));
    }

    #[test]
    fn formats_vocabularies() {
        assert_eq!(list(["fail", "skip"]), "`fail`, `skip`");
        assert_eq!(list(Vec::<&str>::new()), "");
    }
}
