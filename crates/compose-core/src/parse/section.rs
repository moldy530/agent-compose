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
    for item in items {
        let Some(text) = expect_string(item, "each entry of `imports`", cx) else {
            continue;
        };
        if let Some(problem) = import_problem(&text.value) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidImportPath,
                    text.span.clone(),
                    format!("`{}` {problem}", text.value),
                )
                .with_help(
                    "imports are relative paths to `.yml`/`.yaml` spec files, resolved against the entrypoint's directory: there is no directory scanning",
                ),
            );
            continue;
        }
        // Grammar 1.4 requires entries to be unique after normalization.
        // Normalizing (`./a.yml` against `a.yml`, `..` segments) needs the
        // project root, so the resolver owns that; a path repeated verbatim is
        // decidable here, and it is the copy-paste slip a long import list
        // invites.
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
        span: node.span.clone(),
    })
}

/// Why an `imports:` entry is not a legal path, if it is not.
fn import_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("is empty");
    }
    if path.starts_with('/') || path.starts_with('\\') {
        return Some("is an absolute path");
    }
    if path.contains("://") {
        return Some("is a URL");
    }
    if path.contains(['*', '?', '[', ']', '{', '}']) {
        return Some("is a glob pattern");
    }
    if !(path.ends_with(".yml") || path.ends_with(".yaml")) {
        return Some("does not name a `.yml` or `.yaml` file");
    }
    None
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
    let path = fields.string("path", cx).filter(|path| {
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
    let respond = fields
        .take("respond")
        .and_then(|node| lexical::keyword(node, "`respond`", RESPOND_MODES, cx));
    let timeout = fields
        .take("timeout")
        .and_then(|node| lexical::duration(node, "`timeout`", cx));
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
        .filter(|cron| {
            let fields_count = cron.value.split_whitespace().count();
            if fields_count == 5 {
                return true;
            }
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    cron.span.clone(),
                    format!(
                        "`cron` takes a 5-field POSIX cron expression, found {fields_count} field(s)"
                    ),
                )
                .with_help("the fields are minute, hour, day-of-month, month, and day-of-week"),
            );
            false
        });
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
