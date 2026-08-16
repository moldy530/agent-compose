//! Turning [`Diagnostic`]s into something to look at.
//!
//! `compose-core` is render-free on purpose: a [`Diagnostic`] is inert data with
//! a stable code, a message, spans, and help, and nothing in the library knows
//! about terminals. This module is the other half — the one layer that reads
//! source files back off disk and decides what a person sees.
//!
//! # Two formats, two streams
//!
//! * **human** (the default) writes rendered snippets to **stderr**, and nothing
//!   at all to stdout;
//! * **`--format json`** writes one JSON object to **stdout**, and nothing at
//!   all to stderr.
//!
//! The split is what makes `--format json` pipeable without interleaving, and it
//! means a caller never has to strip a progress line out of a document it is
//! about to parse.
//!
//! # The snippet renderer
//!
//! Snippets come from [`annotate-snippets`](https://docs.rs/annotate-snippets),
//! the crate rustc's own diagnostics are rendered with. It was chosen over
//! `codespan-reporting` for three reasons that matter here: its dependency
//! footprint is one crate (`unicode-width`), which keeps the single static
//! binary of PRD 5.12 small; it renders to a `String` rather than to a
//! `WriteColor` stream, so the exact bytes are testable without a terminal
//! abstraction; and it takes byte ranges over a plain `&str`, which is exactly
//! what a [`Span`](compose_core::Span) already carries — no file database to
//! keep in step. Its decor is ASCII in both the plain and the styled renderer,
//! so colour is the *only* thing that varies between a terminal and a pipe.
//!
//! Colour follows the two signals a CLI is expected to read: `NO_COLOR` (any
//! non-empty value turns styling off, per no-color.org) and whether stderr is a
//! terminal. A redirected run is therefore byte-identical to a piped one.
//!
//! # Reading the sources back
//!
//! A span carries a file name and a byte range but not the text, so the sources
//! are read again here. That is a deliberate trade: the alternative is holding
//! every file of the composition in memory for the whole run so that a report
//! that usually has nothing to say can quote from it. A file that cannot be read
//! back — deleted or rewritten under the command — costs the snippet, never the
//! diagnostic: the location is printed on its own instead.

use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use annotate_snippets::renderer::DecorStyle;
use annotate_snippets::{AnnotationKind, Group, Level, Origin, Renderer, Snippet};
use compose_core::{Diagnostic, Severity, Span};

/// The JSON report: `{"diagnostics": [ … ]}`, pretty-printed with a trailing
/// newline.
///
/// The array holds the diagnostics exactly as `compose-core` declares them, in
/// the order they are reported. A clean run writes `{"diagnostics": []}` rather
/// than nothing at all, so a consumer parses one shape whatever the outcome.
pub(crate) fn json(diagnostics: &[Diagnostic]) -> Result<String, serde_json::Error> {
    let mut report = serde_json::Map::new();
    report.insert(
        "diagnostics".to_string(),
        serde_json::to_value(diagnostics)?,
    );
    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(report))?;
    text.push('\n');
    Ok(text)
}

/// The human report, as one string ready for stderr.
///
/// `root` is the project root every span's file name is relative to — the
/// entrypoint's own directory (grammar 1.4).
pub(crate) fn human(root: &Path, diagnostics: &[Diagnostic], color: bool) -> String {
    let renderer = if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    }
    .decor_style(DecorStyle::Ascii);
    let sources = sources(root, diagnostics);
    let mut report = String::new();
    for diagnostic in diagnostics {
        report.push_str(&one(&renderer, diagnostic, &sources));
        report.push_str("\n\n");
    }
    report
}

/// A one-line verdict, rendered through the same styling as the diagnostics.
pub(crate) fn verdict(
    entrypoint: &Path,
    target: &str,
    diagnostics: &[Diagnostic],
    color: bool,
) -> String {
    let renderer = if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    }
    .decor_style(DecorStyle::Ascii);
    let errors = diagnostics.iter().filter(|d| d.is_error()).count();
    let warnings = diagnostics.len() - errors;
    let entrypoint = entrypoint.display();
    if diagnostics.is_empty() {
        return format!("`{entrypoint}` is valid (target `{target}`)\n");
    }
    let mut counted = Vec::new();
    if errors > 0 {
        counted.push(plural(errors, "error"));
    }
    if warnings > 0 {
        counted.push(plural(warnings, "warning"));
    }
    let title = format!(
        "`{entrypoint}` is not valid (target `{target}`): {}",
        counted.join(", ")
    );
    let level = if errors > 0 {
        Level::ERROR
    } else {
        Level::WARNING
    };
    format!(
        "{}\n",
        renderer.render(&[Group::with_title(level.primary_title(title.as_str()))])
    )
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// Every source a report quotes from, by the name its spans use.
fn sources(root: &Path, diagnostics: &[Diagnostic]) -> BTreeMap<String, String> {
    let mut sources = BTreeMap::new();
    for diagnostic in diagnostics {
        for span in std::iter::once(&diagnostic.span)
            .chain(diagnostic.labels.iter().map(|label| &label.span))
        {
            let name = span.source.as_str();
            if sources.contains_key(name) {
                continue;
            }
            // A span's file name is `/`-separated and relative to the project
            // root (grammar 1.4), which is how it is joined back onto a host
            // path here.
            let mut path = PathBuf::from(root);
            path.extend(name.split('/'));
            if let Ok(text) = std::fs::read_to_string(&path) {
                sources.insert(name.to_string(), text);
            }
        }
    }
    sources
}

/// One diagnostic: its title, one snippet per file it points into, and its help.
fn one(renderer: &Renderer, diagnostic: &Diagnostic, sources: &BTreeMap<String, String>) -> String {
    let level = match diagnostic.severity {
        Severity::Error => Level::ERROR,
        Severity::Warning => Level::WARNING,
    };
    let mut group = Group::with_title(
        level
            .primary_title(diagnostic.message.as_str())
            .id(diagnostic.code.as_str()),
    );

    // The primary span's file first, then each label's, so a cross-file
    // diagnostic reads in the order it names its sites.
    let mut files: Vec<&str> = vec![diagnostic.span.source.as_str()];
    for label in &diagnostic.labels {
        let name = label.span.source.as_str();
        if !files.contains(&name) {
            files.push(name);
        }
    }

    for name in files {
        let Some(source) = sources.get(name) else {
            // Unreadable: the location on its own, so the diagnostic still says
            // where it is.
            let span = if diagnostic.span.source.as_str() == name {
                &diagnostic.span
            } else {
                &diagnostic
                    .labels
                    .iter()
                    .find(|label| label.span.source.as_str() == name)
                    .expect("the file came from this diagnostic")
                    .span
            };
            group = group.element(
                Origin::path(name)
                    .line(span.start.line as usize)
                    .char_column(span.start.column as usize),
            );
            continue;
        };
        let mut snippet = Snippet::source(source.as_str()).path(name).fold(true);
        if diagnostic.span.source.as_str() == name {
            snippet =
                snippet.annotation(AnnotationKind::Primary.span(range(&diagnostic.span, source)));
        }
        for label in &diagnostic.labels {
            if label.span.source.as_str() == name {
                snippet = snippet.annotation(
                    AnnotationKind::Context
                        .span(range(&label.span, source))
                        .label(label.message.as_str()),
                );
            }
        }
        group = group.element(snippet);
    }

    if let Some(help) = &diagnostic.help {
        group = group.element(Level::HELP.message(help.as_str()));
    }
    renderer.render(&[group])
}

/// A span's byte range, clamped to the text actually on disk.
///
/// The file is read back after it was parsed, so in principle it can have
/// changed underneath the command. A range that no longer fits would panic the
/// renderer; clamping it to a character boundary inside the text costs an
/// inaccurate underline in a situation that is already lying to the reader, and
/// keeps the diagnostic itself intact.
fn range(span: &Span, source: &str) -> std::ops::Range<usize> {
    let start = boundary(source, span.bytes.start.min(source.len()));
    let end = boundary(source, span.bytes.end.clamp(start, source.len()));
    start..end
}

fn boundary(source: &str, mut at: usize) -> usize {
    while at > 0 && !source.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Whether to style the output: `NO_COLOR` wins, then whether stderr is a
/// terminal (no-color.org).
pub(crate) fn color_enabled() -> bool {
    if std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()) {
        return false;
    }
    std::io::stderr().is_terminal()
}
