//! The spec-file sections that are not definitions: `imports`, `state`, and
//! `triggers` (grammar 1.4, 10, 13).

use crate::ast::common::Namespace;
use crate::ast::document::{Channel, ImportPath, ImportsSection, Reduce, StateSection};
use crate::ast::schema::{Surface, TypeForm, TypeNode};
use crate::ast::trigger::{
    EventTrigger, HttpTrigger, Respond, ScheduleTrigger, TRIGGER_TYPES, Trigger, TriggerKind,
    TriggerMethod, TriggersSection,
};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::Node;

use super::binding::{self, NameForm};
use super::lexical;
use super::reader::{Cx, Fields, expect_mapping, expect_sequence, expect_string, list, suggest};
use super::schema;

/// Read the `imports:` section (grammar 1.4).
pub(crate) fn imports(node: &Node, cx: &mut Cx) -> Option<ImportsSection> {
    let items = expect_sequence(node, "`imports`", cx)?;
    let mut paths: Vec<Spanned<ImportPath>> = Vec::new();
    // An entry refused here names a file that will not join the composition, and
    // a refused entry leaves no trace in `paths` for the resolver to notice. The
    // count is that trace: it is what tells the resolver the name table may be
    // short a file's worth of definitions, so it can withhold the reference pass
    // rather than print one `undefined-reference` per reference into it.
    let mut dropped = 0;
    for item in items {
        let Some(text) = expect_string(item, "each entry of `imports`", cx) else {
            dropped += 1;
            continue;
        };
        if let Some(problem) = import_problem(&text.value) {
            dropped += 1;
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    text.span.clone(),
                    format!("`{}` {}", text.value, problem.reason),
                )
                .with_help(problem.help),
            );
            continue;
        }
        // Grammar 1.4 requires entries to be unique after normalization.
        // Normalizing (`./a.yml` against `a.yml`, `..` segments) needs the
        // project root, so the resolver owns that; a path repeated verbatim is
        // decidable here, and it is the copy-paste slip a long import list
        // invites. It is not counted as dropped: the file is in the composition
        // under the first spelling, so no definition goes missing with it.
        if let Some(first) = paths
            .iter()
            .find(|other| other.value.as_str() == text.value)
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    text.span.clone(),
                    format!("`{}` is imported twice", text.value),
                )
                .with_label(first.span.clone(), "first imported here")
                .with_help("import order does not affect semantics, so the second entry adds nothing: drop it"),
            );
            continue;
        }
        paths.push(Spanned::new(ImportPath::new(&text.value), text.span));
    }
    Some(ImportsSection {
        paths,
        dropped,
        span: node.span.clone(),
    })
}

/// Why an `imports:` entry is not a legal path, and the help that explains it.
struct ImportProblem {
    /// Completes ``` `<path>` … ```.
    reason: &'static str,
    help: &'static str,
}

impl ImportProblem {
    const fn new(reason: &'static str) -> Option<Self> {
        Some(Self {
            reason,
            help: "imports are relative paths to `.yml`/`.yaml` spec files, resolved against the entrypoint's directory: there is no directory scanning",
        })
    }

    const fn with_help(reason: &'static str, help: &'static str) -> Option<Self> {
        Some(Self { reason, help })
    }
}

/// Why an `imports:` entry is not a legal path, if it is not.
///
/// The charset half is grammar 1.4's own rule (Decision D80): `/`-separated
/// segments, each `.`, `..`, or `[A-Za-z0-9_][A-Za-z0-9_.-]*`. One portable
/// spelling keeps a path identical in the IR, on a command line, and in a
/// diagnostic on every host, which a path carrying a space or a backslash does
/// not.
///
/// Order matters where the tests overlap. A `${…}` token is looked for before
/// the glob check because its braces are glob metacharacters too: the entry is
/// refused either way, but "is a glob pattern" would send the author looking
/// for the wrong mistake when what they wrote is plainly an environment
/// reference, which grammar 4.3 puts in class 3 along with every other
/// `imports:` entry.
fn import_problem(path: &str) -> Option<ImportProblem> {
    if path.is_empty() {
        return ImportProblem::new("is empty");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return ImportProblem::new("is an absolute path");
    }
    if path.contains("://") {
        return ImportProblem::new("is a URL");
    }
    if lexical::contains_env_token(path) {
        return ImportProblem::with_help(
            "carries an environment reference",
            "an import path is part of what the composition *is*, so it is fixed in the file rather than at process start (grammar 4.3); write `$${` for a literal `${`",
        );
    }
    if path.contains(['*', '?', '[', ']', '{', '}']) {
        return ImportProblem::new("is a glob pattern");
    }
    if !(path.ends_with(".yml") || path.ends_with(".yaml")) {
        return ImportProblem::new("does not name a `.yml` or `.yaml` file");
    }
    if path.contains('\\') {
        return ImportProblem::new("contains a backslash");
    }
    if path.chars().any(char::is_whitespace) {
        return ImportProblem::new("contains whitespace");
    }
    if !path.split('/').all(is_portable_segment) {
        return ImportProblem::new("is outside the portable path charset");
    }
    None
}

/// Whether one `/`-separated segment matches grammar 1.4's charset.
fn is_portable_segment(segment: &str) -> bool {
    if segment == "." || segment == ".." {
        return true;
    }
    let mut bytes = segment.bytes();
    bytes
        .next()
        .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
}

const REDUCERS: &[(&str, Reduce)] = &[
    ("append", Reduce::Append),
    ("merge", Reduce::Merge),
    ("last_wins", Reduce::LastWins),
];

/// Read the `state:` section (grammar 10).
pub(crate) fn state(node: &Node, cx: &mut Cx) -> Option<StateSection> {
    let mapping = expect_mapping(node, "`state`", cx)?;
    let mut channels = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = lexical::channel_name(&entry.key, "state channel name", cx) else {
            continue;
        };
        let subject = format!("state channel `{}`", name.value);
        let Some(body) = expect_mapping(&entry.value, &subject, cx) else {
            continue;
        };
        let mut fields = Fields::new(body, entry.value.span.clone(), &subject);
        let reduce = fields
            .take("reduce")
            .and_then(|node| lexical::keyword(node, "`reduce`", REDUCERS, cx));
        let (form, description) = schema::type_body(&mut fields, &subject, Surface::Channel, 1, cx);
        fields.finish(cx);

        if let Some(reduce) = reduce.as_ref() {
            check_reduce(reduce, &form, &subject, cx);
        }

        channels.push(Channel {
            name,
            ty: TypeNode {
                form,
                description,
                span: entry.value.span.clone(),
            },
            reduce,
            span: entry.key.span.joined(&entry.value.span),
        });
    }
    Some(StateSection {
        channels,
        span: node.span.clone(),
    })
}

/// `append` requires an array channel and `merge` an object one
/// (grammar 10.2, Decision D32).
fn check_reduce(reduce: &Spanned<Reduce>, form: &TypeForm, subject: &str, cx: &mut Cx) {
    let required = match reduce.value {
        Reduce::Append => "array",
        Reduce::Merge => "object",
        Reduce::LastWins => return,
    };
    let matches = matches!(
        (reduce.value, form),
        (Reduce::Append, TypeForm::Array(_)) | (Reduce::Merge, TypeForm::Object(_))
    );
    if matches || matches!(form, TypeForm::Invalid) {
        return;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            reduce.span.clone(),
            format!(
                "`reduce: {}` requires a `type: {required}` channel, but {subject} is {}",
                reduce.value.as_str(),
                describe_form(form)
            ),
        )
        .with_help(match reduce.value {
            Reduce::Append => "each write to an `append` channel contributes one element",
            _ => "`merge` performs a shallow key-wise merge of the written object",
        }),
    );
}

fn describe_form(form: &TypeForm) -> &'static str {
    match form {
        TypeForm::Scalar(_) => "a scalar",
        TypeForm::Enum(_) => "an enum",
        TypeForm::Object(_) => "an object",
        TypeForm::Array(_) => "an array",
        TypeForm::Union(_) => "a discriminated union",
        TypeForm::Invalid => "untyped",
    }
}

const TRIGGER_METHODS: &[(&str, TriggerMethod)] = &[
    ("POST", TriggerMethod::Post),
    ("PUT", TriggerMethod::Put),
    ("GET", TriggerMethod::Get),
];

const RESPOND_MODES: &[(&str, Respond)] = &[("sync", Respond::Sync), ("async", Respond::Async)];

/// Read the `triggers:` section (grammar 13).
pub(crate) fn triggers(node: &Node, cx: &mut Cx) -> Option<TriggersSection> {
    let mapping = expect_mapping(node, "`triggers`", cx)?;
    let mut triggers = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = lexical::key_identifier(&entry.key, "trigger name", cx) else {
            continue;
        };
        if let Some(trigger) = trigger(name, &entry.value, cx) {
            triggers.push(trigger);
        }
    }
    Some(TriggersSection {
        triggers,
        span: node.span.clone(),
    })
}

fn trigger(name: Spanned<crate::ast::common::Ident>, node: &Node, cx: &mut Cx) -> Option<Trigger> {
    let subject = format!("trigger `{}`", name.value);
    let mapping = expect_mapping(node, &subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), &subject);

    let type_name = fields
        .require("type", cx)
        .and_then(|node| expect_string(node, "`type`", cx));
    let flow = fields
        .require("flow", cx)
        .and_then(|node| lexical::reference(node, "`flow`", &[Namespace::Flow], cx));
    let session_key = fields
        .take("session_key")
        .and_then(|node| lexical::cel(node, "`session_key`", cx));
    let description = super::definition::description(&mut fields, cx);

    let kind = match type_name.as_ref().map(|t| t.value.as_str()) {
        Some("manual") => {
            if let Some(entry) = fields.take_entry("input") {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        entry.key.span.clone(),
                        format!("`input` is not legal on the `manual` {subject}"),
                    )
                    .with_help(
                        "CLI arguments are validated directly against the flow's input schema at run start, so a second binding layer would be two ways to supply the same values (grammar 13.2)",
                    ),
                );
            }
            Some(TriggerKind::Manual)
        }
        Some("http") => Some(TriggerKind::Http(http_trigger(&mut fields, &subject, cx))),
        Some("schedule") => Some(TriggerKind::Schedule(schedule_trigger(&mut fields, cx))),
        Some("event") => Some(TriggerKind::Event(event_trigger(&mut fields, cx))),
        Some(other) => {
            let span = type_name
                .as_ref()
                .map_or(node.span.clone(), |t| t.span.clone());
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::UnknownVariant,
                    span,
                    format!("`{other}` is not a trigger type"),
                )
                .with_optional_help(
                    suggest(other, TRIGGER_TYPES)
                        .map(|name| format!("did you mean `{name}`?"))
                        .or_else(|| Some(format!("the trigger types are {}", list(TRIGGER_TYPES)))),
                ),
            );
            None
        }
        None => None,
    };

    // Every key past the common four belongs to one trigger type, so without a
    // legible `type:` there is nothing to check them against — and they are
    // most likely the keys of the type the author meant: `path`, `method`,
    // `respond` and `timeout` under a misspelt `type: htttp` are exactly what
    // grammar 13.3 gives an `http` trigger. Reporting each as "unknown" would
    // assert a violation the source does not contain and bury the one mistake
    // it does, so consume them, as `schema::type_body` does for a type node
    // that declares no form.
    if !type_name
        .as_ref()
        .is_some_and(|type_name| TRIGGER_TYPES.contains(&type_name.value.as_str()))
    {
        fields.consume_rest();
    }
    fields.finish(cx);
    Some(Trigger {
        name,
        flow,
        session_key,
        description,
        kind,
        span: node.span.clone(),
    })
}

fn http_trigger(fields: &mut Fields<'_>, subject: &str, cx: &mut Cx) -> HttpTrigger {
    // The route is part of the API surface the IR describes, so it is a class-3
    // string: nothing is interpolated and a `${NAME}` token is an error
    // (grammar 4.3, Decision D92).
    let path = fields
        .take("path")
        .and_then(|node| lexical::text(node, "`path`", cx))
        .filter(|path| {
            if !path.value.starts_with('/') {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        path.span.clone(),
                        format!("`path` must start with `/`, found `{}`", path.value),
                    )
                    .with_help("the default is `/triggers/<trigger name>`"),
                );
                return false;
            }
            if path.value.chars().any(char::is_whitespace) {
                cx.error(
                    DiagnosticCode::InvalidValue,
                    &path.span,
                    format!("`path` must not contain whitespace, found `{}`", path.value),
                );
                return false;
            }
            true
        });
    let method = fields
        .take("method")
        .and_then(|node| lexical::keyword(node, "trigger `method`", TRIGGER_METHODS, cx));
    let input = trigger_input(fields, cx);
    let respond_declared = fields.contains("respond");
    let respond = fields
        .take("respond")
        .and_then(|node| lexical::keyword(node, "`respond`", RESPOND_MODES, cx));
    // `timeout:` bounds the *response*, and an async trigger — declared or
    // defaulted — has already responded with an execution id, so the key
    // changes nothing observable (grammar 13.3, Decision D81). An unreadable
    // `respond:` has been reported already; a second diagnostic keyed off a
    // value nobody could read would be noise.
    let timeout = fields.take_entry("timeout").and_then(|entry| {
        if respond_declared && respond.is_none() {
            return None;
        }
        if respond.as_ref().is_some_and(|r| r.value == Respond::Sync) {
            return lexical::duration(&entry.value, "`timeout`", cx);
        }
        let mut diagnostic = Diagnostic::error(
            DiagnosticCode::ConflictingKeys,
            entry.key.span.clone(),
            if respond_declared {
                format!("`timeout` is not legal on the `respond: async` {subject}")
            } else {
                format!("`timeout` is not legal on {subject}, which responds asynchronously by default")
            },
        )
        .with_help(
            "an async trigger has already responded with an execution id, so there is no response left for a budget to bound: `timeout` belongs to `respond: sync` (grammar 13.3, Decision D81)",
        );
        if let Some(respond) = respond.as_ref() {
            diagnostic = diagnostic.with_label(respond.span.clone(), "declared asynchronous here");
        }
        cx.push(diagnostic);
        None
    });
    let callback = fields
        .take_entry("callback")
        .and_then(|entry| {
            if let Some(respond) = respond.as_ref()
                && respond.value == Respond::Sync
            {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::ConflictingKeys,
                        entry.key.span.clone(),
                        format!("`callback` is not legal on the `respond: sync` {subject}"),
                    )
                    .with_label(respond.span.clone(), "declared synchronous here")
                    .with_help("a synchronous response already returns the flow's outputs, so a completion webhook has nothing left to deliver"),
                );
                return None;
            }
            lexical::cel(&entry.value, "`callback`", cx)
        });

    HttpTrigger {
        path,
        method,
        input,
        respond,
        timeout,
        callback,
    }
}

fn schedule_trigger(fields: &mut Fields<'_>, cx: &mut Cx) -> ScheduleTrigger {
    let cron = fields
        .require("cron", cx)
        .and_then(|node| lexical::text(node, "`cron`", cx))
        .filter(|cron| cron_shape(cron, cx));
    let timezone = fields
        .take("timezone")
        .and_then(|node| lexical::non_empty_text(node, "`timezone`", cx));
    let input = trigger_input(fields, cx);
    ScheduleTrigger {
        cron,
        timezone,
        input,
    }
}

/// Whether `cron` has the shape grammar 13.4 fixes: five whitespace-separated
/// fields and nothing around them (Decision D46).
///
/// The published schema spells the same shape as `^\S+(\s+\S+){4}$`, and the
/// anchors are the load-bearing part. Counting the fields alone would accept
/// `"0 3 * * * "`, which the schema rejects — and cron shape is not on PRD §7
/// M0's static-check list, so no later pass would recover it, leaving this pass
/// looser than the schema on a rule it owns (Appendix B).
fn cron_shape(cron: &Spanned<String>, cx: &mut Cx) -> bool {
    if cron.value.trim() != cron.value {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                cron.span.clone(),
                format!(
                    "`cron` must not begin or end with whitespace, found {:?}",
                    cron.value
                ),
            )
            .with_help(
                "the expression is five whitespace-separated fields and nothing else, so a leading or trailing space belongs to no field: drop it",
            ),
        );
        return false;
    }
    let fields_count = cron.value.split_whitespace().count();
    if fields_count == 5 {
        return true;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            cron.span.clone(),
            format!("`cron` takes a 5-field POSIX cron expression, found {fields_count} field(s)"),
        )
        .with_help("the fields are minute, hour, day-of-month, month, and day-of-week"),
    );
    false
}

fn event_trigger(fields: &mut Fields<'_>, cx: &mut Cx) -> EventTrigger {
    let source = fields
        .require("source", cx)
        .and_then(|node| expect_string(node, "`source`", cx))
        .and_then(|text| lexical::identifier(&text, "event source name", cx));
    let input = trigger_input(fields, cx);
    let dedupe_key = fields
        .take("dedupe_key")
        .and_then(|node| lexical::cel(node, "`dedupe_key`", cx));
    EventTrigger {
        source,
        input,
        dedupe_key,
    }
}

fn trigger_input(fields: &mut Fields<'_>, cx: &mut Cx) -> Option<crate::ast::binding::Bindings> {
    fields
        .take("input")
        .and_then(|node| binding::bindings(node, "`input`", NameForm::Identifier, cx))
}

#[cfg(test)]
mod tests {
    use crate::ast::document::Document;
    use crate::parse_str;

    /// The `imports:` entries a file yields, with the drop count the resolver
    /// reads to decide whether the composition is all there.
    fn imports(source: &str) -> (Vec<String>, usize) {
        let document = parse_str(source, "main.yml")
            .document
            .expect("the file parses");
        let Document::Spec(file) = document else {
            panic!("a file declaring `imports:` is a spec file");
        };
        let section = file.imports.expect("the `imports:` section is read");
        (
            section
                .paths
                .iter()
                .map(|path| path.value.as_str().to_string())
                .collect(),
            section.dropped,
        )
    }

    /// An entry the parser refuses names a file that will not join the
    /// composition, and refusing it leaves nothing in `paths` for the resolver
    /// to notice. The count is that trace: without it the resolver resolves
    /// names against a table short a file's worth of definitions and prints one
    /// `undefined-reference` per reference into it — a page of consequences for
    /// one cause.
    #[test]
    fn a_refused_entry_is_counted_as_dropped() {
        assert_eq!(
            imports("imports:\n  - /providers.yml\n  - models.yml\n"),
            (vec!["models.yml".to_string()], 1),
            "an absolute path"
        );
        assert_eq!(
            imports("imports:\n  - \"*.yml\"\n  - models.yml\n"),
            (vec!["models.yml".to_string()], 1),
            "a glob"
        );
        assert_eq!(
            imports("imports:\n  - { path: models.yml }\n"),
            (Vec::new(), 1),
            "a value that is not a string"
        );
        assert_eq!(
            imports("imports:\n  - /a.yml\n  - \"b*.yml\"\n  - c.txt\n"),
            (Vec::new(), 3),
            "every refused entry counts"
        );
    }

    /// A verbatim repeat is refused too, and is deliberately *not* counted: the
    /// file is in the composition under the first spelling, so no definition
    /// goes missing with the second entry and the reference pass has everything
    /// it needs.
    #[test]
    fn a_repeated_entry_loses_no_file_and_is_not_counted() {
        assert_eq!(
            imports("imports:\n  - models.yml\n  - models.yml\n"),
            (vec!["models.yml".to_string()], 0)
        );
    }

    /// The ordinary case: nothing refused, nothing dropped.
    #[test]
    fn a_clean_import_list_drops_nothing() {
        assert_eq!(
            imports("imports:\n  - providers.yml\n  - models.yml\n"),
            (
                vec!["providers.yml".to_string(), "models.yml".to_string()],
                0
            )
        );
    }
}
