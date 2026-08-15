//! Bindings and implementation blocks (grammar 6.1, 8.0, 8.2, 8.3).

use crate::ast::binding::{
    Binding, Bindings, ExecBlock, FunctionBinding, HttpBlock, HttpMethod, InterpolatedEntry,
    NodeInput, WriteEntry, Writes,
};
use crate::ast::schema::{ScalarKind, Surface, TypeForm};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::{Mapping, Node, Yaml};

use super::lexical;
use super::reader::{Cx, Fields, expect_mapping, expect_sequence, expect_string, in_range};
use super::schema;

/// What a binding map's keys must look like.
#[derive(Clone, Copy)]
pub(crate) enum NameForm {
    /// An identifier (grammar 2.1): schema field names, body fields.
    Identifier,
    /// `[A-Za-z0-9_-]+`: HTTP header and query parameter names.
    HeaderLike,
    /// `[A-Za-z_][A-Za-z0-9_]*`: environment variable names.
    EnvVar,
}

impl NameForm {
    fn accepts(self, text: &str) -> bool {
        match self {
            Self::Identifier => lexical::is_identifier(text),
            Self::HeaderLike => {
                !text.is_empty()
                    && text
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            }
            Self::EnvVar => {
                let mut bytes = text.bytes();
                bytes
                    .next()
                    .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                    && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }
        }
    }

    const fn rule(self) -> &'static str {
        match self {
            Self::Identifier => {
                "identifiers are 1-64 characters of lowercase letters, digits, and `_`, starting with a letter"
            }
            Self::HeaderLike => "names here are letters, digits, `_`, and `-`",
            Self::EnvVar => {
                "environment variable names are letters, digits, and `_`, starting with a letter or `_`"
            }
        }
    }

    const fn noun(self) -> &'static str {
        match self {
            Self::Identifier => "identifier",
            Self::HeaderLike => "name",
            Self::EnvVar => "environment variable name",
        }
    }
}

fn check_name(
    key: &Spanned<String>,
    form: NameForm,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<String>> {
    if form.accepts(&key.value) {
        return Some(key.clone());
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidIdentifier,
            key.span.clone(),
            format!(
                "`{}` is not a valid {} in {subject}",
                key.value,
                form.noun()
            ),
        )
        .with_help(form.rule()),
    );
    None
}

/// Read a node's `input:` — either the scalar form or per-field bindings
/// (grammar 8.0).
pub(crate) fn node_input(node: &Node, subject: &str, cx: &mut Cx) -> Option<NodeInput> {
    match &node.value {
        Yaml::String(_) => lexical::cel(node, subject, cx).map(NodeInput::Scalar),
        Yaml::Mapping(_) => {
            bindings(node, subject, NameForm::Identifier, cx).map(NodeInput::Fields)
        }
        _ => {
            cx.wrong_type(
                node,
                subject,
                "a mapping of bindings, or a single CEL expression",
            );
            None
        }
    }
}

/// Read a node's `input:` where only per-field bindings are legal.
///
/// The scalar form of grammar 8.0 binds a string-in agent's single unnamed
/// input (§5.3, Decision D14). A node whose target declares *named* input
/// fields has nothing for a scalar to bind, so accepting one would drop the
/// author's expression on the floor — the silently-ignored-key class that
/// Decisions D61 and D66 exist to reject. `why` names the declaration the
/// bindings have to match, which differs per node kind.
pub(crate) fn field_input(node: &Node, subject: &str, why: &str, cx: &mut Cx) -> Option<Bindings> {
    let Yaml::Mapping(mapping) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::WrongType,
                node.span.clone(),
                format!(
                    "expected a mapping of bindings for {subject}, found {}",
                    node.description()
                ),
            )
            .with_help(format!(
                "{why}; the scalar form binds a string-in agent's single unnamed input instead (grammar 5.3, 8.0, Decision D14)"
            )),
        );
        return None;
    };
    Some(bindings_from(
        mapping,
        node,
        subject,
        NameForm::Identifier,
        cx,
    ))
}

/// Read a mapping of name to CEL expression.
pub(crate) fn bindings(
    node: &Node,
    subject: &str,
    form: NameForm,
    cx: &mut Cx,
) -> Option<Bindings> {
    let mapping = expect_mapping(node, subject, cx)?;
    Some(bindings_from(mapping, node, subject, form, cx))
}

fn bindings_from(
    mapping: &Mapping,
    node: &Node,
    subject: &str,
    form: NameForm,
    cx: &mut Cx,
) -> Bindings {
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = check_name(&entry.key, form, subject, cx) else {
            continue;
        };
        let Some(value) = lexical::cel(&entry.value, &format!("`{}` in {subject}", name.value), cx)
        else {
            continue;
        };
        entries.push(Binding { name, value });
    }
    Bindings {
        entries,
        span: node.span.clone(),
    }
}

/// Read a `writes:` remap (grammar 8.0).
pub(crate) fn writes(node: &Node, subject: &str, cx: &mut Cx) -> Option<Writes> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let Some(field) = lexical::key_identifier(&entry.key, "output field name", cx) else {
            continue;
        };
        let Some(text) = expect_string(
            &entry.value,
            &format!("the channel `{}` is written to", field.value),
            cx,
        ) else {
            continue;
        };
        let Some(channel) = lexical::channel_name(&text, "state channel name", cx) else {
            continue;
        };
        entries.push(WriteEntry { field, channel });
    }
    Some(Writes {
        entries,
        span: node.span.clone(),
    })
}

/// Read an `exec:` block. `output` is legal on an inline node only: a tool
/// declares its result schema on the definition (grammar 6.1, 8.2).
pub(crate) fn exec_block(
    node: &Node,
    subject: &str,
    allow_output: bool,
    cx: &mut Cx,
) -> Option<ExecBlock> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    // `command` and `args` are class-2 surfaces like `cwd` and `env`: the whole
    // `exec:` block is interpolable, and nothing in it is shell-interpreted, so
    // a substituted value is one argv element rather than a re-parsed command
    // line (grammar 4.3, Decision D92).
    let command = fields
        .require("command", cx)
        .and_then(|node| lexical::interpolated(node, "`command`", cx))
        .filter(|command| {
            if command.value.as_str().is_empty() {
                cx.error(
                    DiagnosticCode::InvalidValue,
                    &command.span,
                    "`command` must not be empty",
                );
                return false;
            }
            true
        });

    let mut args = Vec::new();
    if let Some(node) = fields.take("args")
        && let Some(items) = expect_sequence(node, "`args`", cx)
    {
        for item in items {
            if let Some(arg) = lexical::interpolated(item, "each entry of `args`", cx) {
                args.push(arg);
            }
        }
    }

    let cwd = fields
        .take("cwd")
        .and_then(|node| lexical::interpolated(node, "`cwd`", cx));
    let env = fields
        .take("env")
        .map(|node| interpolated_map(node, "`env`", NameForm::EnvVar, cx))
        .unwrap_or_default();
    let expect_exit = accepted_outcomes(&mut fields, "expect_exit", "exit status", 0..=255, cx);
    let output = output_schema(&mut fields, subject, allow_output, EXEC_ENVELOPE, cx);
    fields.finish(cx);

    Some(ExecBlock {
        command,
        args,
        cwd,
        env,
        expect_exit,
        output,
        span: node.span.clone(),
    })
}

/// Read an accepted-outcome list: `expect_exit` or `expect_status`
/// (grammar 6.1, Decisions D84, D100).
///
/// One construct on two surfaces, so one reader: a **non-empty** list of
/// **distinct** members in the key's own range. An empty list accepts no
/// outcome at all, so every run would be an error; membership is a set test, so
/// a repeated member changes nothing about which outcomes are accepted. Both
/// halves are the inert key Decision D61 refuses, in the two spellings this
/// shape admits.
fn accepted_outcomes(
    fields: &mut Fields<'_>,
    key: &'static str,
    noun: &str,
    range: std::ops::RangeInclusive<i64>,
    cx: &mut Cx,
) -> Vec<Spanned<i64>> {
    let mut accepted: Vec<Spanned<i64>> = Vec::new();
    let Some(node) = fields.take(key) else {
        return accepted;
    };
    let Some(items) = expect_sequence(node, &format!("`{key}`"), cx) else {
        return accepted;
    };
    if items.is_empty() {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                node.span.clone(),
                format!("`{key}` must declare at least one {noun}"),
            )
            .with_help(format!(
                "an empty `{key}` accepts no outcome at all, so every call would be a node error"
            )),
        );
    }
    for item in items {
        let Some(value) = super::reader::expect_integer(item, &format!("each `{key}` entry"), cx)
        else {
            continue;
        };
        if !in_range(&value, &format!("an `{key}` entry"), range.clone(), cx) {
            continue;
        }
        if let Some(first) = accepted.iter().find(|other| other.value == value.value) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    value.span.clone(),
                    format!("`{key}` lists {} twice", value.value),
                )
                .with_label(first.span.clone(), "first listed here")
                .with_help(format!(
                    "membership in `{key}` is a set test, so the second entry changes nothing"
                )),
            );
            continue;
        }
        accepted.push(value);
    }
    accepted
}

const HTTP_METHODS: &[(&str, HttpMethod)] = &[
    ("GET", HttpMethod::Get),
    ("POST", HttpMethod::Post),
    ("PUT", HttpMethod::Put),
    ("PATCH", HttpMethod::Patch),
    ("DELETE", HttpMethod::Delete),
    ("HEAD", HttpMethod::Head),
    ("OPTIONS", HttpMethod::Options),
];

/// Read an `http:` block (grammar 6.1, 8.3).
pub(crate) fn http_block(
    node: &Node,
    subject: &str,
    allow_output: bool,
    cx: &mut Cx,
) -> Option<HttpBlock> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    let method = fields
        .require("method", cx)
        .and_then(|node| lexical::keyword(node, "`method`", HTTP_METHODS, cx));
    let url = fields
        .require("url", cx)
        .and_then(|node| lexical::interpolated(node, "`url`", cx));
    let headers = fields
        .take("headers")
        .map(|node| interpolated_map(node, "`headers`", NameForm::HeaderLike, cx))
        .unwrap_or_default();
    let query = fields
        .take("query")
        .and_then(|node| bindings(node, "`query`", NameForm::HeaderLike, cx));
    let body = fields.take_entry("body").and_then(|entry| {
        if let Some(method) = method.as_ref()
            && !method.value.carries_body()
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingKeys,
                    entry.key.span.clone(),
                    format!(
                        "`body` is not legal on a `{}` request",
                        method.value.as_str()
                    ),
                )
                .with_label(method.span.clone(), "the method is declared here")
                .with_help("`GET` and `HEAD` carry no request body; use `query:` instead"),
            );
            return None;
        }
        bindings(&entry.value, "`body`", NameForm::Identifier, cx)
    });

    let expect_status =
        accepted_outcomes(&mut fields, "expect_status", "status code", 100..=599, cx);

    let output = output_schema(&mut fields, subject, allow_output, HTTP_ENVELOPE, cx);
    fields.finish(cx);

    Some(HttpBlock {
        method,
        url,
        headers,
        query,
        body,
        expect_status,
        output,
        span: node.span.clone(),
    })
}

/// Read a `function:` implementation binding (grammar 6.1).
pub(crate) fn function_binding(node: &Node, subject: &str, cx: &mut Cx) -> Option<FunctionBinding> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);
    let name = fields
        .require("name", cx)
        .and_then(|node| expect_string(node, "`name`", cx))
        .and_then(|text| lexical::identifier(&text, "host function name", cx));
    fields.finish(cx);
    Some(FunctionBinding {
        name,
        span: node.span.clone(),
    })
}

/// The fields an inline `exec:` node binds from the child process itself, with
/// the type each one is fixed to (grammar 8.2, Decision D56).
const EXEC_ENVELOPE: &[(&str, ScalarKind)] = &[
    ("exit_code", ScalarKind::Integer),
    ("stdout", ScalarKind::String),
    ("stderr", ScalarKind::String),
];

/// The fields an inline `http:` node binds from the response itself
/// (grammar 8.3, Decision D56).
const HTTP_ENVELOPE: &[(&str, ScalarKind)] = &[
    ("status", ScalarKind::Integer),
    ("body", ScalarKind::String),
];

/// An inline node's `output:` field map, rejected where the block is a tool's
/// implementation binding.
///
/// `envelope` names the fields this kind of node synthesizes rather than
/// decodes; declaring one with any other type is an error, because its value
/// comes from the process or the response and cannot be anything else.
fn output_schema(
    fields: &mut Fields<'_>,
    subject: &str,
    allow_output: bool,
    envelope: &[(&str, ScalarKind)],
    cx: &mut Cx,
) -> Option<crate::ast::schema::FieldMap> {
    if !allow_output {
        return None;
    }
    let map = fields.take("output").and_then(|node| {
        schema::field_map(node, &format!("`output` of {subject}"), Surface::Result, cx)
    })?;

    for (name, required) in envelope {
        let Some(field) = map.field(name) else {
            continue;
        };
        let declared = match &field.ty.form {
            TypeForm::Scalar(scalar) => Some(scalar.kind.value),
            TypeForm::Invalid => continue,
            _ => None,
        };
        if declared == Some(*required) {
            continue;
        }
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                field.ty.span.clone(),
                format!(
                    "`{name}` is an envelope field and must be declared as `{{ type: {} }}`",
                    required.as_str()
                ),
            )
            .with_help(
                "envelope fields are bound from the process or the response itself and are never decoded from its payload, so their types are fixed (grammar 8.2, 8.3)",
            ),
        );
    }
    Some(map)
}

pub(crate) fn interpolated_map(
    node: &Node,
    subject: &str,
    form: NameForm,
    cx: &mut Cx,
) -> Vec<InterpolatedEntry> {
    let Some(mapping) = expect_mapping(node, subject, cx) else {
        return Vec::new();
    };
    let mut entries = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = check_name(&entry.key, form, subject, cx) else {
            continue;
        };
        let Some(value) =
            lexical::interpolated(&entry.value, &format!("`{}` in {subject}", name.value), cx)
        else {
            continue;
        };
        entries.push(InterpolatedEntry { name, value });
    }
    entries
}

/// The `env:` keys of an `exec` block that collide with the upper-snake-cased
/// name of an input field (grammar 6.1, 8.2, Decision D66).
///
/// The bound input object arrives as environment variables, so a colliding
/// `env:` key is one whose value would be silently discarded.
pub(crate) fn reject_env_collisions<'a>(
    block: &ExecBlock,
    input_names: impl IntoIterator<Item = &'a Spanned<String>>,
    cx: &mut Cx,
) {
    for name in input_names {
        let upper = name.value.to_ascii_uppercase();
        let Some(entry) = block.env.iter().find(|entry| entry.name.value == upper) else {
            continue;
        };
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingKeys,
                entry.name.span.clone(),
                format!(
                    "`env` declares `{upper}`, which is also where the input field `{}` arrives",
                    name.value
                ),
            )
            .with_label(name.span.clone(), "this input field takes the same slot")
            .with_help("an object input is passed to the child as upper-snake-cased environment variables, so one of the two values would be silently discarded"),
        );
    }
}
