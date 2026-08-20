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
//! about to parse. Both promises are about the whole run, a reader that stops
//! reading included: writing either report is what [`crate::write()`] is for,
//! because a panic over a closed pipe would land a backtrace on the stream this
//! one promises to leave empty.
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
//!
//! # Where a location points
//!
//! A span names its file relative to the **project root**, which is the
//! entrypoint's own directory (grammar 1.4) — the one name for a file that is
//! the same wherever the command was run from, which is why the machine format
//! carries exactly that and the IR is written in it.
//!
//! A person, and the coding agent PRD G3 calls the other reader of these
//! diagnostics, is not standing in the project root: they typed a path to the
//! entrypoint and are standing wherever that path was relative to. So the human
//! format prints the location the same way — the root joined onto the span's own
//! name — and `agent-compose validate services/api/main.yml` reports
//! `services/api/flows/f.yml:4:5`, a path that opens. Validating from inside the
//! project is the case where the two coincide and nothing is added.

use std::collections::BTreeMap;
use std::ffi::OsStr;
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

/// `build`'s JSON report: `{"diagnostics": [ … ], "drift": [ … ]}`.
///
/// One shape for every outcome, the way [`json`] is: a clean build writes two
/// empty arrays rather than nothing, and a `--check` that found drift writes the
/// same two keys with the second populated. A consumer parses one document and
/// branches on its contents instead of on which command produced it.
pub(crate) fn build_json(
    diagnostics: &[Diagnostic],
    drift: &[crate::build::Drift],
) -> Result<String, serde_json::Error> {
    let mut report = serde_json::Map::new();
    report.insert(
        "diagnostics".to_string(),
        serde_json::to_value(diagnostics)?,
    );
    report.insert(
        "drift".to_string(),
        serde_json::Value::Array(
            drift
                .iter()
                .map(|entry| {
                    let mut object = serde_json::Map::new();
                    object.insert(
                        "path".to_string(),
                        serde_json::Value::String(entry.path.clone()),
                    );
                    object.insert(
                        "state".to_string(),
                        serde_json::Value::String(entry.state.as_str().to_string()),
                    );
                    serde_json::Value::Object(object)
                })
                .collect(),
        ),
    );
    let mut text = serde_json::to_string_pretty(&serde_json::Value::Object(report))?;
    text.push('\n');
    Ok(text)
}

/// Everything one `build` run has to report, apart from where it ran.
pub(crate) struct Built<'a> {
    /// What the validator and then codegen had to say, in source order.
    pub(crate) diagnostics: &'a [Diagnostic],
    /// How the output directory disagrees, under `--check`.
    pub(crate) drift: &'a [crate::build::Drift],
    /// What a rebuild would refuse over, under `--check`
    /// (`crate::build::not_ours`): the files in the output directory this
    /// compiler did not write and would have replaced or removed. Empty when a
    /// rebuild would go through, which is what makes it the remedy.
    pub(crate) not_ours: &'a [String],
    /// What was written, when anything was.
    pub(crate) wrote: Option<&'a crate::build::Written>,
    /// Whether the diagnostics are codegen's rather than the validator's.
    ///
    /// The two refuse for different reasons and a reader who has just seen
    /// `validate` say the file is fine should not now be told it is not: a
    /// pattern this target cannot express is a composition that is valid and
    /// cannot be compiled *here* (see `compose_core::codegen::diagnostics`).
    pub(crate) target_only: bool,
}

/// `build`'s one-line verdict, plus a line per drifted file.
///
/// The drift lines come first and each names one file and how it disagrees, so a
/// CI log says what to regenerate rather than only that something did — and the
/// help under them says which command does the regenerating, which is not
/// unconditionally `build` (see [`drift_help`]).
pub(crate) fn build_verdict(
    entrypoint: &Path,
    target: &str,
    out: &Path,
    built: &Built<'_>,
    color: bool,
) -> String {
    let Built {
        diagnostics,
        drift,
        not_ours,
        wrote,
        target_only,
    } = *built;
    let renderer = if color {
        Renderer::styled()
    } else {
        Renderer::plain()
    }
    .decor_style(DecorStyle::Ascii);

    // An invalid composition is reported as `validate` reports it: the emission
    // never happened, and the reason is the diagnostics above this line.
    if !diagnostics.is_empty() {
        if !target_only {
            return verdict(entrypoint, target, diagnostics, color);
        }
        return format!(
            "{}\n",
            renderer.render(&[Group::with_title(
                Level::ERROR.primary_title(
                    format!(
                        "`{}` is valid and cannot be compiled for `{target}`: {}",
                        entrypoint.display(),
                        plural(diagnostics.len(), "error")
                    )
                    .as_str()
                )
            )])
        );
    }

    let out = out.display();
    let (level, title) = match (drift.is_empty(), wrote) {
        (true, Some(written)) => (
            Level::NOTE.no_name(),
            format!(
                "wrote {} to `{out}` (target `{target}`){}",
                plural(written.files, "file"),
                // A removal is the one thing a build does that the caller did not
                // ask for by name, so it is said out loud rather than left for a
                // later `git status`.
                if written.removed.is_empty() {
                    String::new()
                } else {
                    format!(
                        ", and removed {} it no longer emits: {}",
                        plural(written.removed.len(), "generated file"),
                        written
                            .removed
                            .iter()
                            .map(|path| format!("`{path}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            ),
        ),
        (true, None) => (
            Level::NOTE.no_name(),
            format!("`{out}` is up to date (target `{target}`)"),
        ),
        (false, _) => (
            Level::ERROR,
            format!(
                "`{out}` does not match `{}` (target `{target}`): {}",
                entrypoint.display(),
                plural(drift.len(), "file")
            ),
        ),
    };

    let mut report = String::new();
    for entry in drift {
        report.push_str(&format!("{entry}\n"));
    }
    report.push_str(&format!(
        "{}\n",
        renderer.render(&[Group::with_title(level.primary_title(title.as_str()))])
    ));
    if !drift.is_empty() {
        report.push_str(&drift_help(not_ours));
    }
    report
}

/// What to do about the drift just reported.
///
/// Ordinarily that is `agent-compose build`, which is the whole point of
/// `--check`: CI says the committed project no longer matches its spec, and one
/// command settles it (PRD §8).
///
/// It is not the answer for every directory that drifts, though, and a help line
/// that said so anyway would send a reader from an exit `1` they can act on to an
/// exit `2` they cannot. `build` replaces and removes only files carrying its own
/// generated-file header (see [`crate::build::write`]), so a `src/` holding
/// somebody's own TypeScript, or a `package.json` in a directory this compiler
/// has never built into, is drift a rebuild **refuses** rather than fixes — and
/// a [`Drift`](crate::build::Drift) line cannot tell that case from the stale
/// module beside it, because both are the same state. `crate::build::not_ours`
/// answers it off the same scan the write makes, so the remedy printed here is
/// the one the next command actually performs.
fn drift_help(not_ours: &[String]) -> String {
    if not_ours.is_empty() {
        return "help: run `agent-compose build` to regenerate\n".to_string();
    }
    let (noun, pronoun) = if not_ours.len() == 1 {
        ("a file", "it")
    } else {
        ("files", "them")
    };
    format!(
        "help: `agent-compose build` will not regenerate this directory: it holds {noun} this \
         compiler did not write ({}), and the build would have replaced or removed {pronoun}. \
         Point `--out` at a directory of its own, or move {pronoun} aside\n",
        not_ours
            .iter()
            .map(|path| format!("`{path}`"))
            .collect::<Vec<_>>()
            .join(", "),
    )
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
        report.push_str(&one(&renderer, root, diagnostic, &sources));
        report.push_str("\n\n");
    }
    report
}

/// A one-line verdict, rendered through the same styling as the diagnostics.
///
/// Both outcomes go through the renderer, and the clean one takes a level with
/// no name (`Level::no_name`): it prints as bare text, exactly as a `format!`
/// would, while still picking up the emphasis the failing verdict gets on a
/// terminal. A verdict that was styled on one outcome and not the other would be
/// an inconsistency in the one line every run prints.
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
    let (level, title) = if diagnostics.is_empty() {
        (
            Level::NOTE.no_name(),
            format!("`{entrypoint}` is valid (target `{target}`)"),
        )
    } else {
        let mut counted = Vec::new();
        if errors > 0 {
            counted.push(plural(errors, "error"));
        }
        if warnings > 0 {
            counted.push(plural(warnings, "warning"));
        }
        (
            if errors > 0 {
                Level::ERROR
            } else {
                Level::WARNING
            },
            format!(
                "`{entrypoint}` is not valid (target `{target}`): {}",
                counted.join(", ")
            ),
        )
    };
    format!(
        "{}\n",
        renderer.render(&[Group::with_title(level.primary_title(title.as_str()))])
    )
}

pub(crate) fn plural(count: usize, noun: &str) -> String {
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
            if let Ok(text) = std::fs::read_to_string(at(root, name)) {
                sources.insert(name.to_string(), text);
            }
        }
    }
    sources
}

/// Where a file the report quotes is, from the directory the command ran in: the
/// project root joined onto the `/`-separated name the span carries
/// (grammar 1.4). A run from inside the project has an empty root and the two
/// are the same string.
pub(crate) fn at(root: &Path, name: &str) -> PathBuf {
    let mut path = PathBuf::from(root);
    path.extend(name.split('/'));
    path
}

/// One diagnostic: its title, one snippet per file it points into, and its help.
fn one(
    renderer: &Renderer,
    root: &Path,
    diagnostic: &Diagnostic,
    sources: &BTreeMap<String, String>,
) -> String {
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
                Origin::path(at(root, name).display().to_string())
                    .line(span.start.line as usize)
                    .char_column(span.start.column as usize),
            );
            continue;
        };
        let mut snippet = Snippet::source(source.as_str())
            .path(at(root, name).display().to_string())
            .fold(true);
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
    styling(
        std::env::var_os("NO_COLOR").as_deref(),
        std::io::stderr().is_terminal(),
    )
}

/// The decision the two signals make, apart from where they are read: a
/// non-empty `NO_COLOR` turns styling off whatever the stream is, and a stream
/// that is not a terminal has nothing to style for.
///
/// Split out because the signals themselves are ambient — the environment and a
/// file descriptor — and a rule about them is worth stating somewhere a test can
/// reach without a terminal to run under.
fn styling(no_color: Option<&OsStr>, terminal: bool) -> bool {
    no_color.is_none_or(|value| value.is_empty()) && terminal
}

#[cfg(test)]
mod tests {
    use compose_core::{DiagnosticCode, Span};

    use super::*;

    fn diagnostic() -> Diagnostic {
        Diagnostic::error(
            DiagnosticCode::DeadEnd,
            Span::file_start(compose_core::diag::SourceName::new("main.yml")),
            "a message",
        )
    }

    /// `tests/cli.rs` pins the rendered report byte for byte, but every run it
    /// makes reads through a pipe with `NO_COLOR` set — so the styled renderer,
    /// which is what a person at a terminal actually sees, is never exercised
    /// there.
    ///
    /// Both verdicts go through it, and both come out styled. The failing one
    /// carries its level, the clean one carries none, and each is the plain
    /// string with escapes around it — the decor is ASCII either way, so colour
    /// is the only difference a terminal makes.
    #[test]
    fn both_verdicts_are_styled_or_neither_is() {
        let entrypoint = Path::new("main.yml");
        for diagnostics in [Vec::new(), vec![diagnostic()]] {
            let plain = verdict(entrypoint, "local", &diagnostics, false);
            let styled = verdict(entrypoint, "local", &diagnostics, true);
            assert!(
                !plain.contains('\u{1b}'),
                "the plain renderer styles nothing: {plain:?}"
            );
            assert!(
                styled.contains('\u{1b}'),
                "the styled renderer styles the verdict: {styled:?}"
            );
            assert_eq!(strip(&styled), plain, "styling changes nothing but colour");
        }
    }

    /// The clean verdict is the bare line and nothing else: going through the
    /// renderer must not have introduced a level name in front of it.
    #[test]
    fn the_clean_verdict_is_the_line_itself() {
        assert_eq!(
            verdict(Path::new("services/api/main.yml"), "staging", &[], false),
            "`services/api/main.yml` is valid (target `staging`)\n"
        );
        assert_eq!(
            verdict(Path::new("main.yml"), "local", &[diagnostic()], false),
            "error: `main.yml` is not valid (target `local`): 1 error\n"
        );
    }

    /// The remedy a drift report prints is one that runs.
    ///
    /// `tests/build_cli.rs` pins both lines through the real binary, on a
    /// directory that produces each. What it cannot reach cheaply is the plural
    /// form — two files that are not the compiler's, in one output directory —
    /// and that is a fact about the sentence rather than about the filesystem.
    #[test]
    fn the_drift_help_names_a_command_that_would_run() {
        assert_eq!(
            drift_help(&[]),
            "help: run `agent-compose build` to regenerate\n"
        );
        assert_eq!(
            drift_help(&["src/mine.ts".to_string()]),
            "help: `agent-compose build` will not regenerate this directory: it holds a file this \
             compiler did not write (`src/mine.ts`), and the build would have replaced or removed \
             it. Point `--out` at a directory of its own, or move it aside\n"
        );
        assert_eq!(
            drift_help(&["package.json".to_string(), "src/mine.ts".to_string()]),
            "help: `agent-compose build` will not regenerate this directory: it holds files this \
             compiler did not write (`package.json`, `src/mine.ts`), and the build would have \
             replaced or removed them. Point `--out` at a directory of its own, or move them \
             aside\n"
        );
    }

    #[test]
    fn no_color_wins_over_a_terminal() {
        assert!(styling(None, true));
        assert!(!styling(None, false));
        assert!(!styling(Some(OsStr::new("1")), true));
        assert!(!styling(Some(OsStr::new("anything")), true));
        // An empty value is not a request to turn colour off (no-color.org).
        assert!(styling(Some(OsStr::new("")), true));
        assert!(!styling(Some(OsStr::new("")), false));
    }

    /// The text of a styled render, with the ANSI escapes taken back out.
    fn strip(text: &str) -> String {
        let mut plain = String::new();
        let mut rest = text;
        while let Some(at) = rest.find('\u{1b}') {
            plain.push_str(&rest[..at]);
            let Some(end) = rest[at..].find('m') else {
                break;
            };
            rest = &rest[at + end + 1..];
        }
        plain.push_str(rest);
        plain
    }
}
