//! The emitted page performs **zero** external fetches.
//!
//! PRD resolved q56 makes this the artifact's load-bearing property, on PRD
//! 5.12's terms: the single-binary posture extends to what the binary emits, so
//! the page must open on a machine with no network — an air-gapped laptop, a CI
//! runner with egress blocked, a browser reading it off a USB stick. One CDN
//! script, one webfont, one `@import` and the promise is gone, and it is gone
//! *silently*: the page still renders on the machine of whoever tested it.
//!
//! # The one thing that makes this check hard
//!
//! A graph document is **full of URLs**. `examples/triage-fanout` binds
//! `https://${QUEUE_HOST}/v1/tickets` and half a dozen others, and they belong
//! there — they are the composition's own data, which is exactly what a picture
//! of a composition is for. A scan for `https://` over the whole page would
//! either fail on them or be loosened until it caught nothing.
//!
//! So the page is **split** first: the embedded document is located exactly, by
//! reconstructing the bytes the renderer embedded, and every check below runs
//! over what is left — the template's own markup, styles and code, which is the
//! only half that can fetch anything. The data half is asserted to *contain* a
//! URL, so a change that stopped carrying the composition's requests would fail
//! here rather than make this file vacuous.

use compose_core::graph::{self, GraphDocument};
use compose_core::{Ir, resolve};
use std::path::{Path, PathBuf};

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

fn artifact(project: &str) -> Ir {
    let resolution = resolve(repository().join(project).join("main.yml"));
    assert!(
        resolution.diagnostics.is_empty(),
        "`{project}` does not resolve: {:#?}",
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    assert!(
        compose_core::check(&ir).is_empty(),
        "`{project}` does not validate"
    );
    ir
}

/// The page, and the two halves it is made of.
struct Page {
    /// The template's own bytes, as rendered — everything but the document.
    template: String,
    /// The embedded graph document, exactly as the page carries it.
    document: String,
}

fn page(project: &str) -> Page {
    let ir = artifact(project);
    let document: GraphDocument = graph::graph(&ir);
    let rendered = graph::render(&document).expect("the document renders");
    // The renderer's own substitution, reconstructed: pretty JSON with every
    // `<` written as its JSON escape. Locating it exactly is what lets the
    // checks below be strict about the half that can fetch.
    let embedded = serde_json::to_string_pretty(&document)
        .expect("the document serializes")
        .replace('<', "\\u003c");
    let at = rendered
        .find(&embedded)
        .expect("the page embeds the document verbatim");
    Page {
        template: format!("{}{}", &rendered[..at], &rendered[at + embedded.len()..]),
        document: embedded,
    }
}

/// The XML namespace SVG elements are created in.
///
/// A namespace is an **identifier**: `createElementNS` compares it as a string
/// and no browser has ever fetched one. It is the one `://` the template is
/// allowed, and naming it here is what keeps the check below exact rather than
/// approximate.
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Nothing in the template names a URL to fetch.
#[test]
fn the_template_names_no_url_but_the_svg_namespace() {
    for project in ["examples/triage-fanout", "examples/review-loop"] {
        let page = page(project);
        let mut rest = page.template.as_str();
        let mut found = 0;
        while let Some(at) = rest.find("://") {
            let from = rest[..at].rfind(char::is_whitespace).map_or(0, |at| at + 1);
            // The quote a JavaScript string literal opens with is not part of
            // the URL, and neither is a CSS `url(`.
            let held = rest[from..].trim_start_matches(['"', '\'', '(']);
            assert!(
                held.starts_with(SVG_NAMESPACE),
                "`{project}`'s page names `{}`, which is something to fetch",
                &held[..held.len().min(60)]
            );
            found += 1;
            rest = &rest[at + 3..];
        }
        assert_eq!(
            found, 1,
            "the namespace is still there, so the scan still reads the template"
        );
    }
}

/// No markup that loads anything: no second `<script>`, no `<link>`, no
/// embedded media.
///
/// The list is the set of elements a browser fetches for, rather than a list of
/// the mistakes anyone has made so far: an `<img>` pointing at a logo would be
/// as fatal to the promise as a CDN script, and nothing else in the suite would
/// notice.
#[test]
fn the_page_carries_no_element_that_loads_anything() {
    for project in ["examples/triage-fanout", "examples/review-loop"] {
        let page = page(project);
        assert_eq!(
            page.template.matches("<script").count(),
            1,
            "`{project}`'s page has exactly one script element"
        );
        assert!(
            page.template.contains("<script>\n"),
            "and it carries no attributes, `src` included"
        );
        for element in [
            "<link", "<img", "<iframe", "<object", "<embed", "<audio", "<video", "<source",
            "<track", "<frame", "<applet",
        ] {
            assert!(
                !page.template.contains(element),
                "`{project}`'s page carries `{element}`, which a browser fetches for"
            );
        }
        for attribute in ["srcset", "src=", "href=", "@import", "@font-face"] {
            assert!(
                !page.template.contains(attribute),
                "`{project}`'s page carries `{attribute}`"
            );
        }
    }
}

/// No CSS `url()` pointing at a scheme.
///
/// `url(#arrow)` is a same-document reference to an SVG marker and is fine;
/// `url(https://…)`, `url(//…)` and a quoted spelling of either are not. The
/// check reads what follows each `url(` rather than forbidding the function,
/// because the template needs the fragment form.
#[test]
fn no_css_url_points_at_a_scheme() {
    for project in ["examples/triage-fanout", "examples/review-loop"] {
        let page = page(project);
        let mut rest = page.template.as_str();
        while let Some(at) = rest.find("url(") {
            let held = rest[at + 4..].trim_start_matches(['"', '\'']);
            for scheme in ["http", "//", "ftp", "ws"] {
                assert!(
                    !held.starts_with(scheme),
                    "`{project}`'s page fetches `{}`",
                    &held[..held.len().min(60)]
                );
            }
            rest = &rest[at + 4..];
        }
    }
}

/// No runtime API that could reach the network.
///
/// The element list above covers what the *markup* fetches; this covers what
/// the code could. A page that drew its graph from a `fetch()` would pass every
/// check above and fail the promise completely.
#[test]
fn the_template_calls_no_networking_api() {
    for project in ["examples/triage-fanout", "examples/review-loop"] {
        let page = page(project);
        for api in [
            "fetch(",
            "XMLHttpRequest",
            "WebSocket",
            "EventSource",
            "sendBeacon",
            "importScripts",
            "new Worker",
            "import(",
        ] {
            assert!(
                !page.template.contains(api),
                "`{project}`'s page calls `{api}`"
            );
        }
    }
}

/// The font stacks are the system's: no webfont, named or loaded.
#[test]
fn every_font_stack_is_the_systems_own() {
    let page = page("examples/review-loop");
    assert!(
        page.template.contains("system-ui") && page.template.contains("ui-monospace"),
        "the page asks for the reader's own fonts"
    );
    for hosted in [
        "fonts.googleapis",
        "fonts.gstatic",
        "@font-face",
        "IBM Plex",
    ] {
        assert!(
            !page.template.contains(hosted),
            "the page names the hosted font `{hosted}`"
        );
    }
}

/// …and the split this file rests on is real: the composition's own URLs are in
/// the document half, and nowhere else.
///
/// Without this every check above would pass over a renderer that had stopped
/// embedding the document at all. It is also the case the checks are written
/// around: `https://${QUEUE_HOST}/v1/tickets` is a request the *composition*
/// makes, and a picture of a composition that hid its requests would be a worse
/// picture.
#[test]
fn the_compositions_own_urls_are_in_the_document_and_not_in_the_template() {
    let page = page("examples/triage-fanout");
    for url in [
        "https://${QUEUE_HOST}/v1/tickets",
        "https://${TRIAGE_HOST}/v1/normalize",
    ] {
        assert!(
            page.document.contains(url),
            "the document carries the composition's `{url}`"
        );
        assert!(
            !page.template.contains(url),
            "and the template does not: the split is what the checks rest on"
        );
    }
    assert!(
        page.document.contains("\"graph_version\": 1"),
        "the document half is the document"
    );
}
