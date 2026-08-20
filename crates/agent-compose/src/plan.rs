//! Turning a [`Plan`] into something to read.
//!
//! `compose-core` produces the plan as data and knows nothing about terminals,
//! the way it knows nothing about them for a [`Diagnostic`](compose_core::Diagnostic);
//! this is the other half, and it is the only place that decides what a person
//! sees. `--format json` needs nothing from here at all — the document
//! serializes itself (`docs/plan.md`).
//!
//! # Not a diagnostic
//!
//! Nothing here goes through `annotate-snippets`. A diagnostic points at one
//! region of one file and its whole job is to draw that region; a plan is a
//! **report** about two compositions, most of whose lines are about something
//! that is in one of them and not the other, and quoting source around each
//! would bury forty changes in four hundred lines of context. So the shape is a
//! list: one line per change, marked `+`, `-`, or `~`, under the section that
//! owns it, each naming where to look.
//!
//! The one thing that *is* a diagnostic keeps its renderer: a spec that does not
//! resolve is reported by [`crate::report::human`], exactly as `validate`
//! reports it, because at that point there is no plan and the composition's own
//! errors are the answer.
//!
//! # Where a location points
//!
//! Each side of a plan has its own project root — the two entrypoints' own
//! directories — and a span names its file relative to the root of the
//! composition it came from. A change carries the **after** spec's location,
//! except a removal, which carries the before spec's; so each line is printed
//! against the root it belongs to, and every path in the report is one that
//! opens from the directory the command ran in. This is `report`'s rule with
//! two roots instead of one.
//!
//! # Long values
//!
//! A field's before and after are written as compact JSON and cut to [`WIDTH`]
//! characters, with a `…` and no closing quote so that a cut is visible rather
//! than plausible. A changed `prompt:` is a paragraph, and a report that printed
//! both copies of it in full would be unreadable for the one line it was run to
//! find.
//!
//! What a cut may not do is hide the change. Two values that agree for the first
//! fifty characters and differ after them would both cut to the same text, and
//! the one line the command was run to produce would read as a non-change — so
//! the window **moves**: when the first difference falls past the end of the
//! cut, both sides are printed from [`LEAD`] characters before it instead, with
//! a leading `…` saying so. The window is the same on both sides, so the two
//! stay aligned.
//!
//! A report that cut anything says so once, above the verdict ([`NOTE`]), and
//! points at `--format json`, which carries every value whole.

use std::path::Path;

use compose_core::plan::{
    ChangeKind, ComponentChange, FieldChange, Finding, InterfaceChange, Plan, Refusal, Refused,
    SpecSide, TopologyChange,
};
use compose_core::{Diagnostic, Span};
use serde_json::Value;

/// How much of one value a report prints before cutting it.
const WIDTH: usize = 48;

/// How much of the text the two sides agree on is kept in front of the first
/// difference, when the cut has to move off the head of the value.
const LEAD: usize = 8;

/// What a report says once when it printed a value it had to cut.
const NOTE: &str = "note: a value longer than one line is cut, with a `…` where the cut is; \
                    `--format json` carries every value whole\n";

/// The human report, as one string ready for stderr.
///
/// `before_root` and `after_root` are the two compositions' project roots, which
/// are their entrypoints' own directories (grammar 1.4).
pub(crate) fn human(before_root: &Path, after_root: &Path, plan: &Plan) -> String {
    // Whether any value on the page was cut, which is what [`NOTE`] is about.
    // A finding is never cut — its message is a whole sentence and is printed as
    // one — so the validation section cannot set it.
    let mut cut = false;
    let components: String = plan
        .components
        .iter()
        .map(|change| component(before_root, after_root, change, &mut cut))
        .collect();
    let topology: String = plan
        .topology
        .iter()
        .map(|change| topology(before_root, after_root, change, &mut cut))
        .collect();
    let interfaces: String = plan
        .interfaces
        .iter()
        .map(|change| interface(after_root, change, &mut cut))
        .collect();
    let mut validation = String::new();
    for finding in &plan.validation.introduced {
        validation.push_str(&reported(after_root, ChangeKind::Added, finding));
    }
    for finding in &plan.validation.resolved {
        validation.push_str(&reported(before_root, ChangeKind::Removed, finding));
    }

    let mut report = String::new();
    for (name, lines) in [
        ("components", components),
        ("topology", topology),
        ("interfaces", interfaces),
        ("validation", validation),
    ] {
        if lines.is_empty() {
            continue;
        }
        report.push_str(name);
        report.push('\n');
        report.push_str(&lines);
        report.push('\n');
    }

    if cut {
        report.push_str(NOTE);
    }
    report.push_str(&verdict(plan));
    report
}

/// The one line every run of the command ends with.
fn verdict(plan: &Plan) -> String {
    let after = &plan.after.entrypoint;
    let before = &plan.before.entrypoint;
    let target = &plan.after.target;
    if plan.is_empty() {
        return format!(
            "`{after}` and `{before}` describe the same composition (target `{target}`)\n"
        );
    }
    let mut counted = Vec::new();
    for (count, noun) in [
        (plan.components.len(), "component change"),
        (plan.topology.len(), "topology change"),
        (plan.interfaces.len(), "interface change"),
        (
            plan.validation.introduced.len() + plan.validation.resolved.len(),
            "validation change",
        ),
    ] {
        if count > 0 {
            counted.push(crate::report::plural(count, noun));
        }
    }
    format!(
        "`{after}` differs from `{before}` (target `{target}`): {}\n",
        counted.join(", ")
    )
}

fn component(
    before_root: &Path,
    after_root: &Path,
    change: &ComponentChange,
    cut: &mut bool,
) -> String {
    entry(
        root(before_root, after_root, change.change),
        change.change,
        &change.address,
        &change.span,
        &change.fields,
        cut,
    )
}

fn topology(
    before_root: &Path,
    after_root: &Path,
    change: &TopologyChange,
    cut: &mut bool,
) -> String {
    entry(
        root(before_root, after_root, change.change),
        change.change,
        &change.address,
        &change.span,
        &change.fields,
        cut,
    )
}

fn interface(after_root: &Path, change: &InterfaceChange, cut: &mut bool) -> String {
    entry(
        after_root,
        change.change,
        &change.address,
        &change.span,
        &change.fields,
        cut,
    )
}

/// Which composition a change's location is in: the before spec for a removal,
/// the after spec for everything else.
fn root<'a>(before: &'a Path, after: &'a Path, change: ChangeKind) -> &'a Path {
    match change {
        ChangeKind::Removed => before,
        ChangeKind::Added | ChangeKind::Changed => after,
    }
}

/// One subject, and a line per field of it that moved.
fn entry(
    root: &Path,
    change: ChangeKind,
    address: &str,
    span: &Span,
    fields: &[FieldChange],
    cut: &mut bool,
) -> String {
    let mut held = format!("  {} {address}  {}\n", mark(change), at(root, span));
    for field in fields {
        let (before, after, was_cut) = sides(field);
        *cut |= was_cut;
        held.push_str(&format!(
            "      {}: {before} -> {after}\n",
            if field.path.is_empty() {
                "(value)"
            } else {
                field.path.as_str()
            },
        ));
    }
    held
}

/// One diagnostic the two sides disagree about.
fn reported(root: &Path, change: ChangeKind, finding: &Finding) -> String {
    format!(
        "  {} {}[{}]: {}  {}\n",
        mark(change),
        finding.severity,
        finding.code,
        finding.message,
        at(root, &finding.span),
    )
}

const fn mark(change: ChangeKind) -> char {
    match change {
        ChangeKind::Added => '+',
        ChangeKind::Removed => '-',
        ChangeKind::Changed => '~',
    }
}

/// Where a span points, from the directory the command ran in.
fn at(root: &Path, span: &Span) -> String {
    format!(
        "{}:{}:{}",
        crate::report::at(root, span.source.as_str()).display(),
        span.start.line,
        span.start.column
    )
}

/// One field change's two sides as the report prints them, and whether either
/// had to be cut.
///
/// The window is chosen from the **pair** rather than from each side on its own,
/// which is the whole point: two values that agree past the end of the cut would
/// otherwise print as the same text twice, and the line the reader came for
/// would say nothing. When the first difference falls outside the first [`WIDTH`]
/// characters, both sides are printed from [`LEAD`] characters ahead of it —
/// the same offset on both, so the two lines still read against each other.
fn sides(field: &FieldChange) -> (String, String, bool) {
    let before = value(field.before.as_ref());
    let after = value(field.after.as_ref());
    if before.chars().count() <= WIDTH && after.chars().count() <= WIDTH {
        return (before, after, false);
    }
    let shared = before
        .chars()
        .zip(after.chars())
        .take_while(|(one, two)| one == two)
        .count();
    let skip = if shared < WIDTH { 0 } else { shared - LEAD };
    (cut(&before, skip), cut(&after, skip), true)
}

/// One side of a field change, as compact JSON — or `(absent)`, which is what a
/// field the spec does not declare at all reads as. That is a real distinction:
/// an absent `timeout:` inherits the next level of grammar 9.3's chain, and one
/// written `null` does not parse at all.
fn value(held: Option<&Value>) -> String {
    held.map_or_else(|| "(absent)".to_string(), Value::to_string)
}

/// A value from `skip` characters in, at most [`WIDTH`] of them, with a `…` on
/// whichever end was cut — and no closing quote, so that a cut looks like one
/// rather than like a whole value.
fn cut(text: &str, skip: usize) -> String {
    let mut held = String::new();
    if skip > 0 {
        held.push('…');
    }
    let mut rest = text.chars().skip(skip);
    held.extend(rest.by_ref().take(WIDTH));
    if rest.next().is_some() {
        held.push('…');
    }
    held
}

/// One spec that could not be resolved, as the command holds it before it is
/// written either way.
pub(crate) struct Failed<'a> {
    /// Which side of the comparison it is.
    pub(crate) side: SpecSide,
    /// The entrypoint the command resolved: the file it was given, or the
    /// `main.yml` inside the directory it was given.
    pub(crate) entrypoint: String,
    /// That composition's project root, which is the directory its entrypoint
    /// sits in — what the paths in its report are printed against.
    pub(crate) root: &'a Path,
    /// What the parser and the resolver reported.
    pub(crate) diagnostics: &'a [Diagnostic],
}

/// A spec that does not resolve, reported the way `validate` reports it.
///
/// One block per failed spec — its diagnostics, then a line naming which side of
/// the comparison it is. Both sides are reported when both failed: a person
/// diffing two branches wants to know that neither of them resolves, not to find
/// out one at a time. The two blocks are separated by a blank line, because the
/// line that closes one is a sentence and the line that opens the next is a
/// diagnostic about a different project.
pub(crate) fn refused(failed: &[Failed<'_>], target: &str, color: bool) -> String {
    let mut report = String::new();
    for spec in failed {
        if !report.is_empty() {
            report.push('\n');
        }
        report.push_str(&crate::report::human(spec.root, spec.diagnostics, color));
        let errors = spec
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.is_error())
            .count();
        let warnings = spec.diagnostics.len() - errors;
        let mut counted = Vec::new();
        if errors > 0 {
            counted.push(crate::report::plural(errors, "error"));
        }
        if warnings > 0 {
            counted.push(crate::report::plural(warnings, "warning"));
        }
        report.push_str(&format!(
            "error: the {} spec `{}` does not resolve (target `{target}`): {}\n",
            spec.side.as_str(),
            spec.entrypoint,
            counted.join(", ")
        ));
    }
    report
}

/// The same, as the document `--format json` writes.
pub(crate) fn refusal(failed: &[Failed<'_>]) -> Refusal {
    Refusal {
        plan_version: compose_core::PLAN_VERSION,
        failed: failed
            .iter()
            .map(|spec| Refused {
                spec: spec.side,
                entrypoint: spec.entrypoint.clone(),
                diagnostics: spec.diagnostics.to_vec(),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A field change over two values, as the report renders them.
    fn rendered(before: &str, after: &str) -> (String, String, bool) {
        sides(&FieldChange {
            path: "prompt".to_string(),
            before: Some(Value::String(before.to_string())),
            after: Some(Value::String(after.to_string())),
        })
    }

    #[test]
    fn a_value_is_cut_at_the_reports_width_and_says_so() {
        assert_eq!(cut("\"short\"", 0), "\"short\"");
        let long = format!("\"{}\"", "a".repeat(80));
        let held = cut(&long, 0);
        assert_eq!(held.chars().count(), WIDTH + 1);
        assert!(held.ends_with('…'), "the cut is visible: {held}");
        assert!(
            !held.ends_with("\"…"),
            "…and the value is left unterminated rather than looking whole: {held}"
        );
    }

    /// A cut lands on a character boundary, not on a byte inside one.
    #[test]
    fn a_multibyte_value_is_cut_between_characters() {
        let long = format!("\"{}\"", "é".repeat(80));
        assert_eq!(cut(&long, 0).chars().count(), WIDTH + 1);
    }

    /// Two values that differ inside the first line are printed from the start,
    /// and the pair is reported as cut only when something was actually taken
    /// off it.
    #[test]
    fn a_difference_inside_the_line_leaves_the_window_where_it_is() {
        let (before, after, cut) = rendered("alpha", "beta");
        assert_eq!(
            (before.as_str(), after.as_str(), cut),
            ("\"alpha\"", "\"beta\"", false)
        );

        let (before, after, cut) = rendered("a", &"b".repeat(80));
        assert_eq!(before, "\"a\"");
        assert!(after.starts_with("\"bbb"), "{after}");
        assert!(after.ends_with('…'), "{after}");
        assert!(cut);
    }

    /// …and two values that agree past the end of the cut are printed from the
    /// difference instead, so the line the report was run to find says
    /// something. This is the failure the moving window exists for: both sides
    /// cut at the same prefix render as the same text.
    #[test]
    fn a_difference_past_the_cut_moves_the_window_onto_it() {
        let shared = "Review the draft with great care and much attention, then say ";
        let (before, after, cut) = rendered(&format!("{shared}alpha"), &format!("{shared}beta"));
        assert!(cut);
        assert_ne!(before, after, "the two sides do not print as one text");
        assert!(before.starts_with('…'), "{before}");
        assert!(after.starts_with('…'), "{after}");
        assert!(before.ends_with("say alpha\""), "{before}");
        assert!(after.ends_with("say beta\""), "{after}");
        // The window is the same on both sides, so the two lines read against
        // each other: they still share their first LEAD characters of text.
        let lead: String = before.chars().skip(1).take(LEAD).collect();
        assert_eq!(lead, after.chars().skip(1).take(LEAD).collect::<String>());
    }

    #[test]
    fn an_undeclared_field_reads_as_absent_rather_than_as_null() {
        assert_eq!(value(None), "(absent)");
        assert_eq!(value(Some(&Value::Null)), "null");
    }
}
