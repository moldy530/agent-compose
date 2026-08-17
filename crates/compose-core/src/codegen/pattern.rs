//! `pattern:` from RE2 into a JavaScript regular expression.
//!
//! # Why this is a pass and not a `format!`
//!
//! Decision D12 makes `pattern:` RE2 so that "patterns mean the same thing in
//! the Rust validator, in generated JS validation, and in provider-side
//! structured-output engines". RE2 is **not** a syntactic subset of ECMAScript,
//! so copying the source text into a regex literal is not a lowering, it is a
//! bet:
//!
//! ```text
//! pattern: "(?i)^abc$"      →  /(?i)^abc$/       SyntaxError: Invalid group
//! pattern: "(?P<word>x)"    →  /(?P<word>x)/     SyntaxError: Invalid group
//! pattern: ""               →  //                a line comment, not a regex
//! ```
//!
//! Each of those is a spec the validator accepts and a `src/schemas.ts` no
//! runtime can even *load* — the whole module, every schema in it, gone at
//! import. So the emitter does not get to assume: [`javascript`] decides, for
//! one pattern, either the literal to emit or the reason it cannot be emitted,
//! and [`super::diagnostics`] turns the second answer into a `build` that
//! refuses rather than a project that will not parse.
//!
//! The narrower question is the point. Whether a `pattern:` is a regular
//! expression **at all** is a shape rule of the type node and is decided by the
//! parser (`parse::schema`'s `is_re2`), so `validate` refuses `^a{$`, a
//! backreference and a lookaround on its own. What arrives here is a pattern
//! every engine can read, and what is asked of it is whether *this target* can
//! hold it. [`javascript`] still answers for a malformed one, because it is a
//! total function a library caller may reach without the validator.
//!
//! # What is rejected, and why each one
//!
//! The walk is over `regex_syntax`'s AST — the same parser the `regex` crate
//! uses, so "not RE2 at all" falls out of it — and it reports two classes:
//!
//! * **constructs ECMAScript cannot parse**: inline flag groups (`(?i)`,
//!   `(?s:…)`), the `(?P<name>…)` spelling of a named group, POSIX classes
//!   (`[[:alpha:]]`), class set operations (`[\w&&[^a]]`), `\u{…}` and `\U…`
//!   escapes;
//! * **constructs ECMAScript parses as something else**, which is worse, because
//!   nothing fails and the schema quietly means a different thing: `\A` and `\z`
//!   are anchors in RE2 and the literal letters `A` and `z` in a JavaScript
//!   regex without `u`; `\a` is a bell in RE2 and the letter `a` in JavaScript;
//!   `\p{Greek}` is a Unicode class in RE2 and the letter `p` followed by a
//!   braced literal in JavaScript; `[]-]` is the class `{]`, `-`}` in RE2 and
//!   the *empty* class followed by two literals in JavaScript; `[a[b]]` is the
//!   union `{a, b}` in RE2 and the class `{a, [, b}` followed by a literal `]`
//!   in JavaScript.
//!
//! The last two are inside a bracketed class, which is why every literal a class
//! holds — including the two endpoints of a range — goes through the same
//! decision as one written outside it. A class is not a quoted region: `[\a-b]`
//! and `[\x41-\u{1F600}]` are the escapes above, one level in, and reading a
//! range's endpoints as opaque would let each of them through.
//!
//! Everything else transfers: literals and their escapes, `.`, `^`, `$`, `\b`,
//! `\B`, the Perl classes, bracketed classes and ranges, all four repetition
//! operators with their lazy forms, alternation, capturing and non-capturing
//! groups, and `(?<name>…)`. The match on the AST is exhaustive on purpose —
//! a construct a future `regex_syntax` adds is a compile error here rather than
//! a new way to emit an unparseable module.
//!
//! The Perl classes are the case worth stating, because they look like the
//! obvious hazard and are not one: `\d`, `\w` and `\s` are Unicode-aware in a
//! Rust `regex` and ASCII-only in ECMAScript, but a JSON Schema `pattern` is
//! **defined** as ECMAScript, and the validator that reads the emitted schema
//! translates them accordingly — `^\d+$` refuses `١٢٣` in both columns. The
//! conformance corpus pins that (`state.slug`'s
//! `perl-classes-mean-the-ascii-thing-in-both-columns` documents) so the claim
//! is a test rather than a paragraph.
//!
//! Two constructs are where that reasoning stops short of agreement, and both are
//! declared rather than papered over. Without the `u` flag (see below), `.`
//! matches one UTF-16 **code unit**, so `^.$` refuses `"😀"` in the emitted Zod
//! and accepts it in the Rust validator, whose engine matches one **code
//! point**. And `\b`/`\B` are word boundaries in both engines over *different*
//! word characters: ECMAScript's are ASCII, and the validating engine's are
//! Unicode — the one Perl construct the paragraph above does not cover, because
//! the translation it relies on rewrites `\w` and leaves the boundary alone. So
//! `\bcat\b` accepts `"caté"` in the emitted Zod and refuses it in the Rust
//! column. Which column is the deviant one is arguable in both cases —
//! ECMAScript is what JSON Schema names, so the emitted regex is the literal
//! reading — but each difference is real and reachable, so they are
//! `dot-matches-a-code-unit` and `word-boundary-is-unicode-aware` in
//! [`super::schema`]'s divergence ledger, with `state.glyph` and `state.word` in
//! the corpus as the documents that decide them.
//!
//! # The literal
//!
//! A pattern is written as a regex **literal** with any unescaped `/` escaped,
//! and falls back to `new RegExp("…")` for a pattern carrying a line terminator,
//! which a literal cannot hold. The empty pattern is `/(?:)/` — which is what
//! `new RegExp("").toString()` answers, and for the same reason: `//` opens a
//! comment. No flags are added: `u` mode rejects escapes RE2 accepts, and both
//! `RegExp.prototype.test` and JSON Schema's `pattern` are unanchored searches,
//! so the two agree without one — everywhere but `.`, which is the declared
//! divergence above and the price of keeping the escapes.

use regex_syntax::ast::{
    Assertion, AssertionKind, Ast, ClassBracketed, ClassPerl, ClassSet, ClassSetBinaryOp,
    ClassSetItem, ClassSetRange, ClassSetUnion, Group, GroupKind, HexLiteralKind, Literal,
    LiteralKind, SpecialLiteralKind, parse::Parser,
};

use super::names;

/// Why one `pattern:` cannot be lowered to a JavaScript regular expression.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported {
    /// The one-line statement, in diagnostic voice: lowercase, no trailing stop.
    pub message: String,
    /// What to write instead.
    pub help: String,
}

impl Unsupported {
    fn new(message: impl Into<String>, help: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            help: help.into(),
        }
    }
}

/// One `pattern:` as the JavaScript expression the emitter writes, or the reason
/// it has none.
///
/// # Errors
///
/// The pattern is not RE2, or it uses a construct ECMAScript either cannot parse
/// or parses as something else — see the module docs for the whole list.
pub fn javascript(pattern: &str) -> Result<String, Unsupported> {
    let ast = Parser::new().parse(pattern).map_err(|error| {
        Unsupported::new(
            format!(
                "`pattern` is not a valid regular expression: {}",
                error.kind()
            ),
            "`pattern:` is RE2 — no backreferences and no lookaround (grammar 3.3, Decision D12)",
        )
    })?;
    representable(&ast)?;
    Ok(verbatim(pattern))
}

/// The pattern's source text as a JavaScript expression, whatever it says.
///
/// [`javascript`] is the one to call: this is the half that does not decide, and
/// is public only so that [`super::schema`] can stay total for a caller who
/// emitted without running [`super::diagnostics`] first.
#[must_use]
pub fn verbatim(pattern: &str) -> String {
    if pattern.is_empty() {
        // `//` is a line comment. `/(?:)/` is the empty regex, and is what
        // `new RegExp("")` prints itself as.
        return "/(?:)/".to_string();
    }
    if pattern.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return format!("new RegExp({})", names::string(pattern));
    }
    let mut text = String::with_capacity(pattern.len() + 2);
    text.push('/');
    let mut escaped = false;
    for character in pattern.chars() {
        if escaped {
            text.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' => {
                text.push('\\');
                escaped = true;
            }
            '/' => text.push_str("\\/"),
            other => text.push(other),
        }
    }
    text.push('/');
    text
}

/// Whether every construct of this AST exists, and means the same thing, in a
/// JavaScript regular expression.
fn representable(ast: &Ast) -> Result<(), Unsupported> {
    match ast {
        Ast::Empty(_) | Ast::Dot(_) | Ast::ClassPerl(_) => Ok(()),
        Ast::Flags(_) => Err(inline_flags()),
        Ast::Literal(literal) => literal_representable(literal),
        Ast::Assertion(assertion) => assertion_representable(assertion),
        Ast::ClassUnicode(_) => Err(unicode_class()),
        Ast::ClassBracketed(class) => bracketed(class),
        Ast::Repetition(repetition) => representable(&repetition.ast),
        Ast::Group(group) => group_representable(group),
        Ast::Alternation(alternation) => alternation.asts.iter().try_for_each(representable),
        Ast::Concat(concat) => concat.asts.iter().try_for_each(representable),
    }
}

fn inline_flags() -> Unsupported {
    Unsupported::new(
        "`pattern` sets flags inline (`(?i)`, `(?s:…)`), which a JavaScript regular expression has \
         no spelling for",
        "write the pattern without inline flags — a case-insensitive class is `[Aa]`, and `(?s)` \
         is `[\\s\\S]` in place of `.` (grammar 3.3, Decision D12)",
    )
}

fn unicode_class() -> Unsupported {
    Unsupported::new(
        "`pattern` uses a Unicode class (`\\p{…}`), which a JavaScript regular expression reads as \
         the letter `p` instead",
        "write the characters out as a bracketed class — `\\p{Nd}` is `[0-9]` for ASCII digits \
         (grammar 3.3, Decision D12)",
    )
}

fn literal_representable(literal: &Literal) -> Result<(), Unsupported> {
    match literal.kind {
        LiteralKind::Verbatim
        | LiteralKind::Meta
        | LiteralKind::Superfluous
        | LiteralKind::Octal
        | LiteralKind::HexFixed(HexLiteralKind::X | HexLiteralKind::UnicodeShort) => Ok(()),
        LiteralKind::HexFixed(HexLiteralKind::UnicodeLong) | LiteralKind::HexBrace(_) => {
            Err(Unsupported::new(
                "`pattern` uses a `\\u{…}` or `\\U…` escape, which a JavaScript regular expression \
                 without the `u` flag does not have",
                "write the character itself, or `\\uXXXX` for one below U+10000 (grammar 3.3, \
                 Decision D12)",
            ))
        }
        LiteralKind::Special(SpecialLiteralKind::Bell) => Err(Unsupported::new(
            "`pattern` uses `\\a`, which a JavaScript regular expression reads as the letter `a` \
             instead",
            "write `\\x07` for the bell character (grammar 3.3, Decision D12)",
        )),
        LiteralKind::Special(
            SpecialLiteralKind::FormFeed
            | SpecialLiteralKind::Tab
            | SpecialLiteralKind::LineFeed
            | SpecialLiteralKind::CarriageReturn
            | SpecialLiteralKind::VerticalTab
            | SpecialLiteralKind::Space,
        ) => Ok(()),
    }
}

fn assertion_representable(assertion: &Assertion) -> Result<(), Unsupported> {
    match assertion.kind {
        AssertionKind::StartLine
        | AssertionKind::EndLine
        | AssertionKind::WordBoundary
        | AssertionKind::NotWordBoundary => Ok(()),
        AssertionKind::StartText => Err(Unsupported::new(
            "`pattern` uses `\\A`, which a JavaScript regular expression reads as the letter `A` \
             instead",
            "`^` is the start of the subject: no flags are added to the emitted expression, so it \
             never means the start of a line (grammar 3.3, Decision D12)",
        )),
        AssertionKind::EndText => Err(Unsupported::new(
            "`pattern` uses `\\z`, which a JavaScript regular expression reads as the letter `z` \
             instead",
            "`$` is the end of the subject: no flags are added to the emitted expression, so it \
             never means the end of a line (grammar 3.3, Decision D12)",
        )),
        AssertionKind::WordBoundaryStart
        | AssertionKind::WordBoundaryEnd
        | AssertionKind::WordBoundaryStartAngle
        | AssertionKind::WordBoundaryEndAngle
        | AssertionKind::WordBoundaryStartHalf
        | AssertionKind::WordBoundaryEndHalf => Err(Unsupported::new(
            "`pattern` uses a half word boundary (`\\b{start}`, `\\<`, `\\>`), which a JavaScript \
             regular expression does not have",
            "`\\b` is the boundary both engines share (grammar 3.3, Decision D12)",
        )),
    }
}

fn group_representable(group: &Group) -> Result<(), Unsupported> {
    match &group.kind {
        GroupKind::CaptureIndex(_) => {}
        GroupKind::CaptureName {
            starts_with_p: true,
            name,
        } => {
            return Err(Unsupported::new(
                format!(
                    "`pattern` names a capture group `(?P<{}>…)`, which is RE2's spelling and not \
                     JavaScript's",
                    name.name
                ),
                format!(
                    "write `(?<{}>…)`, which both engines accept (grammar 3.3, Decision D12)",
                    name.name
                ),
            ));
        }
        GroupKind::CaptureName { .. } => {}
        GroupKind::NonCapturing(flags) => {
            if !flags.items.is_empty() {
                return Err(inline_flags());
            }
        }
    }
    representable(&group.ast)
}

fn bracketed(class: &ClassBracketed) -> Result<(), Unsupported> {
    set(&class.kind)
}

fn set(kind: &ClassSet) -> Result<(), Unsupported> {
    match kind {
        ClassSet::Item(item) => set_item(item),
        ClassSet::BinaryOp(ClassSetBinaryOp { .. }) => Err(Unsupported::new(
            "`pattern` uses a character-class set operation (`&&`, `--`, `~~`), which a JavaScript \
             regular expression without the `v` flag does not have",
            "write the resulting set out as one class (grammar 3.3, Decision D12)",
        )),
    }
}

fn set_item(item: &ClassSetItem) -> Result<(), Unsupported> {
    match item {
        ClassSetItem::Empty(_) => Ok(()),
        ClassSetItem::Range(ClassSetRange { start, end, .. }) => {
            class_literal(start).and_then(|()| class_literal(end))
        }
        ClassSetItem::Literal(literal) => class_literal(literal),
        ClassSetItem::Unicode(_) => Err(unicode_class()),
        ClassSetItem::Ascii(_) => Err(Unsupported::new(
            "`pattern` uses a POSIX class (`[[:alpha:]]`), which a JavaScript regular expression \
             does not have",
            "write the class out — `[[:alpha:]]` is `[A-Za-z]` (grammar 3.3, Decision D12)",
        )),
        ClassSetItem::Perl(ClassPerl { .. }) => Ok(()),
        // `[a[b]]` is the union `{a, b}` to RE2 and, to a JavaScript regular
        // expression without the `v` flag, the class `{a, [, b}` followed by a
        // literal `]` — a whole different set, and a different length of match.
        ClassSetItem::Bracketed(_) => Err(Unsupported::new(
            "`pattern` nests a character class (`[a[b]]`), which a JavaScript regular expression \
             without the `v` flag reads as a literal `[` inside the outer class",
            "write the union out as one class — `[a[b]]` is `[ab]` (grammar 3.3, Decision D12)",
        )),
        ClassSetItem::Union(ClassSetUnion { items, .. }) => items.iter().try_for_each(set_item),
    }
}

/// One literal written **inside** a bracketed class.
///
/// Every escape [`literal_representable`] decides is legal inside a class too, so
/// a class is not a region this walk may skip: `[\a-b]` is the bell escape and
/// `[\x41-\u{1F600}]` is the `\u{…}` one, each one level in.
///
/// One spelling is a hazard only in here. RE2 (like POSIX) reads a `]` written
/// **first** in a class as a member of it — `[]-]` is `{]`, `-`}`, and `[^]a]`
/// is everything but `]` and `a` — while a JavaScript regular expression closes
/// the class at that `]` instead: `[]` matches nothing at all and `[^]` matches
/// anything, with the rest of the intended class left over as literals. Nothing
/// fails; the schema means something else entirely, in both directions.
fn class_literal(literal: &Literal) -> Result<(), Unsupported> {
    if matches!(literal.kind, LiteralKind::Verbatim) && literal.c == ']' {
        return Err(Unsupported::new(
            "`pattern` writes `]` unescaped inside a character class, which is RE2's spelling for \
             a class that holds one — a JavaScript regular expression closes the class there \
             instead",
            "escape it: `[]-]` is `[\\]-]` and `[^]a]` is `[^\\]a]` (grammar 3.3, Decision D12)",
        ));
    }
    literal_representable(literal)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(pattern: &str) -> String {
        javascript(pattern).unwrap_or_else(|error| {
            panic!(
                "`{pattern}` was refused: {} ({})",
                error.message, error.help
            )
        })
    }

    fn refused(pattern: &str) -> Unsupported {
        javascript(pattern).expect_err(&format!("`{pattern}` was accepted"))
    }

    #[test]
    fn the_empty_pattern_is_the_empty_regex_and_not_a_comment() {
        assert_eq!(ok(""), "/(?:)/");
    }

    #[test]
    fn a_slash_is_escaped_and_an_escape_is_left_alone() {
        assert_eq!(ok("a/b"), "/a\\/b/");
        assert_eq!(ok("a\\/b"), "/a\\/b/");
        assert_eq!(ok("\\d+"), "/\\d+/");
    }

    #[test]
    fn a_line_terminator_falls_back_to_the_constructor() {
        assert_eq!(ok("a\nb"), "new RegExp(\"a\\nb\")");
    }

    /// The whole vocabulary that transfers unchanged. Every one of these would
    /// be a regression if the walk grew a case that refused it.
    #[test]
    fn the_shared_vocabulary_survives_the_walk() {
        for pattern in [
            "^[a-z][a-z0-9-]*$",
            "\\d{3}-\\d{4}",
            "a|b|c",
            "(a)(?:b)(?<tail>c)",
            "x*?y+?z??",
            "[^\\w\\s.-]",
            "\\bword\\b",
            "\\.\\+\\*\\?",
            "\\x41\\u00e9",
            "\\t\\n\\r\\f\\v",
            ".{1,8}",
            // The escaped spellings of the two class hazards below, which are
            // the same set in both engines and are what the refusals send a
            // reader to.
            "[\\]-]",
            "[^\\]a]",
            "[ab]",
            "[\\x41-\\x5a]",
        ] {
            let emitted = ok(pattern);
            assert!(
                emitted.starts_with('/') && emitted.ends_with('/'),
                "`{pattern}` did not become a literal: {emitted}"
            );
        }
    }

    #[test]
    fn inline_flags_are_refused_wherever_they_appear() {
        assert!(refused("(?i)^abc$").message.contains("inline"));
        assert!(refused("^(?s:.*)$").message.contains("inline"));
        assert!(refused("a(?i)b").message.contains("inline"));
    }

    #[test]
    fn the_re2_named_group_spelling_is_refused_with_the_javascript_one() {
        let refusal = refused("(?P<word>[a-z]+)");
        assert!(refusal.message.contains("(?P<word>…)"), "{refusal:?}");
        assert!(refusal.help.contains("(?<word>…)"), "{refusal:?}");
    }

    #[test]
    fn constructs_javascript_reads_as_something_else_are_refused() {
        for (pattern, expected) in [
            ("\\Aabc\\z", "\\A"),
            ("\\p{Greek}+", "\\p{…}"),
            ("[\\p{Nd}]", "\\p{…}"),
            ("\\a", "\\a"),
            ("[[:alpha:]]+", "[[:alpha:]]"),
            ("[\\w&&[^a]]", "&&"),
            ("\\u{1F600}", "\\u{…}"),
        ] {
            let refusal = refused(pattern);
            assert!(
                refusal.message.contains(expected),
                "`{pattern}` was refused for the wrong reason: {refusal:?}"
            );
        }
    }

    /// A class is a region of the pattern, not a quoted one: the escapes above
    /// mean the same things one level in, and two spellings mean something
    /// *else* only in here.
    ///
    /// `[]-]` and `[^]a]` are RE2's (and POSIX's) way of putting a `]` in a
    /// class; JavaScript closes the class at that `]`, which makes `[]-]` an
    /// expression that matches nothing and `[^]a]` one that matches almost
    /// everything. `[a[b]]` is a nested class in RE2 and a literal `[` in
    /// JavaScript. Each was accepted and copied verbatim, and each inverts the
    /// verdict on most documents — see `codegen::schema`'s corpus for the
    /// measured columns.
    #[test]
    fn a_class_is_walked_like_the_pattern_around_it() {
        for (pattern, expected) in [
            ("[]-]", "unescaped inside a character class"),
            ("[^]a]", "unescaped inside a character class"),
            ("[]-a]", "unescaped inside a character class"),
            ("^[a[b]]$", "nests a character class"),
            ("[a[b]c]", "nests a character class"),
            // A range's endpoints are literals like any other, and reading them
            // as opaque let two refused escapes through.
            ("[\\a-b]", "\\a"),
            ("[\\x41-\\u{1F600}]", "\\u{…}"),
            ("[\\U00000041-b]", "\\u{…}"),
        ] {
            let refusal = refused(pattern);
            assert!(
                refusal.message.contains(expected),
                "`{pattern}` was refused for the wrong reason: {refusal:?}"
            );
        }
        assert!(
            refused("[]-]").help.contains("[\\]-]"),
            "the help writes the spelling that transfers"
        );
    }

    #[test]
    fn a_pattern_that_is_not_re2_is_refused_as_such() {
        let refusal = refused("(unclosed");
        assert!(
            refusal.message.contains("not a valid regular expression"),
            "{refusal:?}"
        );
        assert!(refused("a{2,1}").message.contains("not a valid"));
        // D12's own two exclusions, which `regex_syntax` reports for us.
        assert!(refused("(?<=a)b").message.contains("look-around"));
        assert!(refused("(a)\\1").message.contains("backreference"));
    }
}
