//! One graph document plus one embedded template, out as one file.
//!
//! # The placeholder
//!
//! The template is ordinary HTML with one token in it, [`PLACEHOLDER`], sitting
//! where a JavaScript object literal goes. Rendering replaces that token with
//! the serialized document and does nothing else — no interpolation of anything
//! else, no templating language — because the file is a **golden** (PRD §9.15,
//! resolved q56): the same composition must answer the same bytes, and a
//! renderer with two substitutions is a renderer with two ways to drift.
//!
//! # Why `<` is escaped
//!
//! The document is embedded inside a `<script>` element, where the HTML
//! tokenizer — not the JavaScript parser — decides where the element ends. A
//! `</script>` inside a string would end it early, and a `<!--` followed by a
//! `<script` would put the tokenizer in a state where the real `</script>`
//! stops closing it. Both live in strings this document carries verbatim: an
//! agent's `prompt:`, a tool's `description:`, a CEL expression.
//!
//! So every `<` is written as `\u003c`, which is a **valid JSON escape** — the
//! embedded text stays a JSON document a reader can lift out and parse, and the
//! JavaScript string it parses to is character-for-character the one the
//! document holds. A partial escape (`</` alone) would close the `</script>`
//! hole and leave the `<!--<script` one; escaping the character closes both,
//! and closes them for every future string this document learns to carry.
//!
//! What `--format json` prints is the document itself, unescaped: that is a
//! machine surface rather than a fragment of a web page.

use super::document::GraphDocument;

/// The token [`render`] replaces with the graph document.
///
/// Spelled so that it is a syntax error on its own — the template is not a
/// runnable page until it has been rendered, which is what keeps a reader from
/// mistaking the embedded source for the artifact.
pub(super) const PLACEHOLDER: &str = "/*{{GRAPH_DOCUMENT}}*/";

/// The page, embedded in the compiler (PRD 5.12, resolved q23).
///
/// Public because the test that proves the artifact fetches nothing reads it,
/// and because a reader asking "what exactly does this binary emit" should be
/// able to reach it from the library rather than from a path inside this crate.
pub const TEMPLATE: &str = include_str!("template.html");

/// One graph document, rendered as one self-contained HTML file.
///
/// # Errors
///
/// Returns the serializer's error, which
/// [`GraphDocument::to_json`](super::GraphDocument::to_json) documents as
/// unreachable and surfaces all the same.
pub fn render(document: &GraphDocument) -> serde_json::Result<String> {
    let json = serde_json::to_string_pretty(document)?;
    Ok(TEMPLATE.replace(PLACEHOLDER, &json.replace('<', "\\u003c")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GRAPH_VERSION;

    fn document() -> GraphDocument {
        GraphDocument {
            graph_version: GRAPH_VERSION,
            entrypoint: "main.yml".to_string(),
            target: "local".to_string(),
            spec_version: "0.1".to_string(),
            flows: Vec::new(),
        }
    }

    /// The template carries the placeholder exactly once, and rendering
    /// consumes it.
    #[test]
    fn the_template_has_one_hole_and_rendering_fills_it() {
        assert_eq!(
            TEMPLATE.matches(PLACEHOLDER).count(),
            1,
            "the template has exactly one substitution"
        );
        let page = render(&document()).expect("the document renders");
        assert!(
            !page.contains(PLACEHOLDER),
            "the placeholder is gone from the artifact"
        );
        assert!(page.contains("\"graph_version\": 1"));
    }

    /// A `</script>` or a `<!--<script` inside a string never reaches the
    /// artifact as markup.
    ///
    /// Both are things an author writes in a prompt without thinking about it,
    /// and both end the embedded document early in a way the JavaScript parser
    /// never sees. The escape is asserted on the artifact rather than on the
    /// serializer, because the artifact is what a browser reads.
    #[test]
    fn markup_inside_the_document_is_escaped_out_of_the_script() {
        let mut held = document();
        held.entrypoint = "</script><!--<script>alert(1)</script>".to_string();
        let page = render(&held).expect("the document renders");
        assert!(
            !page.contains("</script><!--"),
            "the markup survived into the page: {page}"
        );
        assert!(
            page.contains("\\u003c/script>\\u003c!--\\u003cscript>"),
            "and it is there, escaped: {page}"
        );
        // One `</script>` and one `<script>`: the template's own.
        assert_eq!(page.matches("</script>").count(), 1);
    }
}
