//! The lexical forms of grammar 2 and 4: identifiers, typed addresses,
//! durations, environment references, and path expressions.
//!
//! Each of these has a grammar decidable from the string itself, so the parser
//! settles them and hands the decomposition to later passes. CEL is the
//! deliberate exception: its *shape* is "a non-empty string", and everything
//! else about it — which roots are in scope, whether it type-checks against the
//! declared schemas — needs the resolved composition (grammar 4.1).

use crate::ast::common::{
    Address, Cel, ControlTarget, Duration, DurationUnit, EdgeSource, EdgeTarget, EnvRef, Ident,
    Interpolated, Namespace, PSEUDO_NODES, PathExpr, PathStep, RESERVED_ROOT_NAMES,
};
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::yaml::{Node, Yaml};

use super::reader::{Cx, expect_string, list, suggest};

const IDENTIFIER_RULE: &str =
    "identifiers are 1-64 characters of lowercase letters, digits, and `_`, starting with a letter";

/// Whether `text` matches the identifier grammar (grammar 2.1).
#[must_use]
pub fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    if text.chars().count() > 64 {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Read an identifier from an already-extracted string.
///
/// `subject` names the position for the diagnostic ("state channel name").
pub(crate) fn identifier(
    text: &Spanned<String>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<Ident>> {
    if is_identifier(&text.value) {
        return Some(Spanned::new(Ident::new(&text.value), text.span.clone()));
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidIdentifier,
            text.span.clone(),
            format!("{subject} `{}` is not a valid identifier", text.value),
        )
        .with_help(IDENTIFIER_RULE),
    );
    None
}

/// Read an identifier from a mapping key.
pub(crate) fn key_identifier(
    key: &Spanned<String>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<Ident>> {
    identifier(key, subject, cx)
}

/// Read a state channel name: an identifier that is not reserved (grammar 2.5,
/// 10.4).
pub(crate) fn channel_name(
    text: &Spanned<String>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<Ident>> {
    let name = identifier(text, subject, cx)?;
    if RESERVED_ROOT_NAMES.contains(&name.value.as_str()) {
        let reason = if name.value.as_str() == "messages" {
            "`messages` is the implicit conversation-history channel"
        } else {
            "it is a CEL root identifier"
        };
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ReservedName,
                name.span.clone(),
                format!("`{}` may not name a state channel: {reason}", name.value),
            )
            .with_help(format!(
                "reserved channel names are {}",
                list(RESERVED_ROOT_NAMES)
            )),
        );
        return None;
    }
    Some(name)
}

/// Why the reserved-root list refuses a node id (grammar 2.5, Decision D74).
const RESERVED_NODE_ID_RULE: &str = "a node id is the root of `<node>.output` in an edge guard and in `map.over`, so a node named `input` would make `input.output.x` ambiguous with the flow input object; the reserved names are `input`, `state`, `execution`, `item`, `messages`, `output`, `payload`";

/// Refuse a reserved root name in a position that names a flow-local node id
/// (grammar 2.5, Decision D74).
///
/// The four positions §2.5 governs for node ids are the `nodes:` key itself and
/// the three that refer to one: an edge's `from`/`to` and the two
/// control-transfer targets. Refusing the name at every one of them keeps a
/// composition from spelling a node the `nodes:` map could never declare.
fn reject_reserved_node_id(name: &Spanned<Ident>, cx: &mut Cx) -> bool {
    if !RESERVED_ROOT_NAMES.contains(&name.value.as_str()) {
        return false;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::ReservedName,
            name.span.clone(),
            format!(
                "`{}` is a reserved name and may not be used as a node id",
                name.value
            ),
        )
        .with_help(RESERVED_NODE_ID_RULE),
    );
    true
}

/// Read a flow-local node id: an identifier that is neither a pseudo-node
/// (grammar 2.4) nor a reserved root name (grammar 2.5, Decision D74).
pub(crate) fn node_id(
    text: &Spanned<String>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<Ident>> {
    let name = identifier(text, subject, cx)?;
    if PSEUDO_NODES.contains(&name.value.as_str()) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ReservedName,
                name.span.clone(),
                format!(
                    "`{}` is a pseudo-node and may not be used as a node id",
                    name.value
                ),
            )
            .with_help("`start` is a flow's entry and `end` its exit; they are written on edges"),
        );
        return None;
    }
    if reject_reserved_node_id(&name, cx) {
        return None;
    }
    Some(name)
}

/// Read a `map`'s `as:` binding: an identifier that shadows no root in scope
/// for the map's per-item expressions (grammar 2.5, 8.6, Decision D74).
///
/// `item` is the one reserved name that is legal here: it is this binding's own
/// default name, so binding it explicitly renames nothing.
pub(crate) fn item_binding_name(
    text: &Spanned<String>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<Ident>> {
    let name = identifier(text, subject, cx)?;
    if name.value.as_str() != "item" && RESERVED_ROOT_NAMES.contains(&name.value.as_str()) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ReservedName,
                name.span.clone(),
                format!(
                    "`{}` is a reserved name and may not be a `map`'s `as` binding",
                    name.value
                ),
            )
            .with_help(
                "the per-item binding is itself a root in the map's per-item expressions, so it may not shadow one that is already there; `item` is the one legal reserved name, being this binding's own default",
            ),
        );
        return None;
    }
    Some(name)
}

/// Read a control-transfer target: a flow-local node id, or `end`
/// (grammar 2.4, 9.2).
pub(crate) fn control_target(
    node: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<ControlTarget>> {
    let text = expect_string(node, subject, cx)?;
    if text.value == "end" {
        return Some(Spanned::new(ControlTarget::End, text.span));
    }
    if text.value == "start" {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ReservedName,
                text.span,
                format!("{subject} may not be `start`"),
            )
            .with_help("control transfers go to a node of this flow or to `end`: re-entering a flow from an error has no defined input"),
        );
        return None;
    }
    let name = identifier(&text, subject, cx)?;
    if reject_reserved_node_id(&name, cx) {
        return None;
    }
    Some(name.map(ControlTarget::Node))
}

/// Read an edge's `from:`: a node id, or `start`.
pub(crate) fn edge_source(node: &Node, cx: &mut Cx) -> Option<Spanned<EdgeSource>> {
    let text = expect_string(node, "`from`", cx)?;
    match text.value.as_str() {
        "start" => Some(Spanned::new(EdgeSource::Start, text.span)),
        "end" => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ReservedName,
                    text.span,
                    "no edge may leave `end`",
                )
                .with_help("`end` terminates the flow instance; nothing runs after it"),
            );
            None
        }
        _ => {
            let name = identifier(&text, "`from`", cx)?;
            (!reject_reserved_node_id(&name, cx)).then(|| name.map(EdgeSource::Node))
        }
    }
}

/// Read an edge's `to:`: a node id, or `end`.
pub(crate) fn edge_target(node: &Node, cx: &mut Cx) -> Option<Spanned<EdgeTarget>> {
    let text = expect_string(node, "`to`", cx)?;
    match text.value.as_str() {
        "end" => Some(Spanned::new(EdgeTarget::End, text.span)),
        "start" => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::ReservedName,
                    text.span,
                    "no edge may target `start`",
                )
                .with_help(
                    "`start` is the flow's entry; one or more edges leave it and none reach it",
                ),
            );
            None
        }
        _ => {
            let name = identifier(&text, "`to`", cx)?;
            (!reject_reserved_node_id(&name, cx)).then(|| name.map(EdgeTarget::Node))
        }
    }
}

/// How a set of accepted namespaces is described in a diagnostic:
/// "a `model.*` reference", "an `agent.*`, `tool.*`, or `flow.*` reference".
pub(crate) fn reference_expectation(allowed: &[Namespace]) -> String {
    let names: Vec<String> = allowed
        .iter()
        .map(|namespace| format!("`{namespace}.*`"))
        .collect();
    let joined = match names.as_slice() {
        [] => String::new(),
        [only] => only.clone(),
        [first, second] => format!("{first} or {second}"),
        [rest @ .., last] => format!("{}, or {last}", rest.join(", ")),
    };
    let article = if allowed.first() == Some(&Namespace::Agent) {
        "an"
    } else {
        "a"
    };
    format!("{article} {joined} reference")
}

/// Read a typed address from a value position (grammar 2.3).
pub(crate) fn reference(
    node: &Node,
    subject: &str,
    allowed: &[Namespace],
    cx: &mut Cx,
) -> Option<Spanned<Address>> {
    let expectation = reference_expectation(allowed);
    let Yaml::String(text) = &node.value else {
        cx.wrong_type(node, subject, &expectation);
        return None;
    };
    let spanned = Spanned::new(text.clone(), node.span.clone());
    address(&spanned, subject, allowed, cx)
}

/// Parse a typed address, checking it names one of the accepted namespaces.
pub(crate) fn address(
    text: &Spanned<String>,
    subject: &str,
    allowed: &[Namespace],
    cx: &mut Cx,
) -> Option<Spanned<Address>> {
    let expectation = reference_expectation(allowed);
    let Some((prefix, name)) = text.value.split_once('.') else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidReference,
                text.span.clone(),
                format!(
                    "expected {expectation} for {subject}, found `{}`",
                    text.value
                ),
            )
            .with_help(format!(
                "references are `<namespace>.<name>`; the namespaces are {}",
                list(Namespace::ALL.iter().map(|n| n.as_str()))
            )),
        );
        return None;
    };

    let Some(namespace) = Namespace::from_prefix(prefix) else {
        let names: Vec<&str> = Namespace::ALL.iter().map(|n| n.as_str()).collect();
        let suggestion =
            suggest(prefix, &names).map(|namespace| format!("did you mean `{namespace}.{name}`?"));
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidReference,
                text.span.clone(),
                format!(
                    "expected {expectation} for {subject}, found `{}`",
                    text.value
                ),
            )
            .with_optional_help(suggestion.or_else(|| {
                Some(format!(
                    "the namespaces are {}",
                    list(Namespace::ALL.iter().map(|n| n.as_str()))
                ))
            })),
        );
        return None;
    };

    if !allowed.contains(&namespace) {
        cx.error(
            DiagnosticCode::InvalidReference,
            &text.span,
            format!(
                "expected {expectation} for {subject}, found `{}`",
                text.value
            ),
        );
        return None;
    }

    if !is_identifier(name) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidIdentifier,
                text.span.clone(),
                format!("`{}` is not a valid name in `{namespace}.*`", name),
            )
            .with_help(IDENTIFIER_RULE),
        );
        return None;
    }

    Some(Spanned::new(
        Address::new(namespace, Ident::new(name)),
        text.span.clone(),
    ))
}

const DURATION_RULE: &str = "durations are a positive integer followed by `ms`, `s`, `m`, or `h`, e.g. `30s`; compound forms like `1m30s` are not supported";

/// Read a duration (grammar 4.4).
pub(crate) fn duration(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<Duration>> {
    let Yaml::String(text) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidDuration,
                node.span.clone(),
                format!(
                    "expected a duration for {subject}, found {}",
                    node.description()
                ),
            )
            .with_help(DURATION_RULE),
        );
        return None;
    };

    match parse_duration(text) {
        Some(duration) => Some(Spanned::new(duration, node.span.clone())),
        None => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidDuration,
                    node.span.clone(),
                    format!("`{text}` is not a valid duration for {subject}"),
                )
                .with_help(DURATION_RULE),
            );
            None
        }
    }
}

fn parse_duration(text: &str) -> Option<Duration> {
    let (digits, unit) = if let Some(digits) = text.strip_suffix("ms") {
        (digits, DurationUnit::Milliseconds)
    } else if let Some(digits) = text.strip_suffix('s') {
        (digits, DurationUnit::Seconds)
    } else if let Some(digits) = text.strip_suffix('m') {
        (digits, DurationUnit::Minutes)
    } else if let Some(digits) = text.strip_suffix('h') {
        (digits, DurationUnit::Hours)
    } else {
        return None;
    };
    if digits.is_empty() || digits.starts_with('0') || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    // Grammar 4.4 states a form — a positive integer and a unit — and no
    // ceiling, so a magnitude too large for `u64` is a well-formed duration this
    // implementation cannot count exactly, not a malformed one. Reporting it
    // through the failure below would send the author hunting for a syntax
    // mistake that is not there, and would split two inputs that already mean
    // the same thing: `18446744073709551615h` fits in `u64` and is accepted, and
    // `Duration::as_millis` saturates it to the same value this one saturates
    // to. The text as written survives in the duration's `raw`.
    let amount = digits.parse::<u64>().unwrap_or(u64::MAX);
    Some(Duration::new(text, amount, unit))
}

const ENV_NAME_RULE: &str = "environment names are uppercase letters, digits, and `_`, starting with a letter or `_`; write `$${NAME}` for a literal `${NAME}`";

/// One `${...}` occurrence found while scanning a string.
struct EnvToken {
    /// The variable name, when the token is well-formed.
    name: Option<String>,
    /// The token as written, for the diagnostic.
    text: String,
}

/// Scan a string for unescaped `${...}` tokens, honouring the `$${` escape
/// (grammar 4.3).
fn scan_env_tokens(text: &str) -> Vec<EnvToken> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'$' {
            index += 1;
            continue;
        }
        // `$${` is the escape for a literal `${`.
        if bytes.get(index + 1) == Some(&b'$') {
            index += if bytes.get(index + 2) == Some(&b'{') {
                3
            } else {
                2
            };
            continue;
        }
        if bytes.get(index + 1) != Some(&b'{') {
            index += 1;
            continue;
        }
        match text[index..].find('}') {
            Some(offset) => {
                let token = &text[index..index + offset + 1];
                let name = &token[2..token.len() - 1];
                tokens.push(EnvToken {
                    name: is_env_name(name).then(|| name.to_owned()),
                    text: token.to_owned(),
                });
                index += offset + 1;
            }
            None => {
                tokens.push(EnvToken {
                    name: None,
                    text: text[index..].to_owned(),
                });
                break;
            }
        }
    }
    tokens
}

/// Whether `text` reaches for an environment substitution at all.
///
/// Unlike [`reject_env_refs`], a malformed token counts: this answers "did the
/// author write `${…}` here", which is the question at a surface that has no
/// second pass to report a bad env *name* (an `imports:` entry is a path, not a
/// value read through [`env_ref`]). The `$${` escape is honoured, so an entry
/// that genuinely spells a literal `${` is not caught by it.
pub(crate) fn contains_env_token(text: &str) -> bool {
    !scan_env_tokens(text).is_empty()
}

/// Whether `text` matches the env-name grammar `[A-Z_][A-Z0-9_]*`.
#[must_use]
pub fn is_env_name(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_uppercase() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Whether the whole string is exactly one environment reference.
#[must_use]
pub fn is_env_ref_value(text: &str) -> bool {
    text.strip_prefix("${")
        .and_then(|rest| rest.strip_suffix('}'))
        .is_some_and(is_env_name)
}

/// Read a secret-bearing field: the whole value must be one `${NAME}`
/// (grammar 4.3, Decision D41).
pub(crate) fn env_ref(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<EnvRef>> {
    let Yaml::String(text) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidEnvRef,
                node.span.clone(),
                format!(
                    "{subject} takes an environment reference, found {}",
                    node.description()
                ),
            )
            .with_help(
                "the spec never contains credentials or connection strings: write `${NAME}`",
            ),
        );
        return None;
    };
    if !is_env_ref_value(text) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidEnvRef,
                node.span.clone(),
                format!("{subject} takes an environment reference, found the literal `{text}`"),
            )
            .with_help(
                "the spec never contains credentials or connection strings: write `${NAME}`",
            ),
        );
        return None;
    }
    let name = text[2..text.len() - 1].to_owned();
    Some(Spanned::new(EnvRef::new(text, name), node.span.clone()))
}

/// Read a string that may embed environment references (grammar 4.3).
pub(crate) fn interpolated(
    node: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<Interpolated>> {
    let text = expect_string(node, subject, cx)?;
    Some(interpolate(text, subject, cx))
}

/// The interpolated form of a string already read, reporting every malformed
/// `${…}` token it carries (grammar 4.3).
///
/// Split out from [`interpolated`] for the surfaces that have already matched
/// the YAML scalar themselves and so have no "found a mapping" case left to
/// report — an open plugin-config object walks its own tree.
pub(crate) fn interpolate(
    text: Spanned<String>,
    subject: &str,
    cx: &mut Cx,
) -> Spanned<Interpolated> {
    let mut references = Vec::new();
    for token in scan_env_tokens(&text.value) {
        match token.name {
            Some(name) => references.push(name),
            None => {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidEnvRef,
                        text.span.clone(),
                        format!(
                            "`{}` in {subject} is not a valid environment reference",
                            token.text
                        ),
                    )
                    .with_help(ENV_NAME_RULE),
                );
            }
        }
    }
    Spanned::new(Interpolated::new(text.value, references), text.span)
}

/// Check that a string carries no environment reference, on the surfaces where
/// nothing is interpolated (grammar 4.3, Decision D41).
///
/// Only well-formed `${NAME}` tokens count: a prompt that mentions `${1}` is
/// discussing shell syntax, not asking for a substitution, and the published
/// JSON Schema draws the line in the same place.
pub(crate) fn reject_env_refs(text: &Spanned<String>, subject: &str, cx: &mut Cx) {
    for token in scan_env_tokens(&text.value) {
        if token.name.is_some() {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::UnexpectedEnvRef,
                    text.span.clone(),
                    format!(
                        "{subject} never interpolates environment references, but it contains `{}`",
                        token.text
                    ),
                )
                .with_help(format!(
                    "write `$${}` for a literal `{}`",
                    &token.text[1..],
                    token.text
                )),
            );
        }
    }
}

/// Read literal text: a string on a surface where environment references are
/// illegal (prompts, descriptions, ids — grammar 4.3).
pub(crate) fn text(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<String>> {
    let value = expect_string(node, subject, cx)?;
    reject_env_refs(&value, subject, cx);
    Some(value)
}

/// Read a non-empty literal text value.
pub(crate) fn non_empty_text(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<String>> {
    let value = text(node, subject, cx)?;
    if value.value.is_empty() {
        cx.error(
            DiagnosticCode::InvalidValue,
            &value.span,
            format!("{subject} must not be empty"),
        );
        return None;
    }
    Some(value)
}

/// Read a CEL expression (grammar 4.1).
///
/// The parser checks only that it is a non-empty string carrying no
/// environment reference; the expression itself stays raw until the validator
/// type-checks it against the surface's roots.
pub(crate) fn cel(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<Cel>> {
    let Yaml::String(raw) = &node.value else {
        cx.wrong_type(node, subject, "a CEL expression");
        return None;
    };
    let value = Spanned::new(raw.clone(), node.span.clone());
    reject_env_refs(&value, subject, cx);
    if raw.trim().is_empty() {
        cx.error(
            DiagnosticCode::InvalidValue,
            &node.span,
            format!("{subject} must not be an empty expression"),
        );
        return None;
    }
    Some(Spanned::new(Cel::new(raw), node.span.clone()))
}

const PATH_RULE: &str = "a path expression is a root identifier followed by field selections and integer indexes, e.g. `plan.output.tasks`; calls and operators are not allowed";

/// Read a path expression (grammar 4.2, Decision D43).
pub(crate) fn path_expression(
    node: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<PathExpr>> {
    let Yaml::String(raw) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidPathExpression,
                node.span.clone(),
                format!(
                    "expected a path expression for {subject}, found {}",
                    node.description()
                ),
            )
            .with_help(PATH_RULE),
        );
        return None;
    };
    match parse_path(raw) {
        Some(path) => Some(Spanned::new(path, node.span.clone())),
        None => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidPathExpression,
                    node.span.clone(),
                    format!("`{raw}` is not a path expression for {subject}"),
                )
                .with_help(PATH_RULE),
            );
            None
        }
    }
}

fn parse_path(raw: &str) -> Option<PathExpr> {
    let mut rest = raw;
    let root_end = rest.find(['.', '[']).unwrap_or(rest.len());
    let root = &rest[..root_end];
    if !is_identifier(root) {
        return None;
    }
    rest = &rest[root_end..];

    let mut steps = Vec::new();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after.find(['.', '[']).unwrap_or(after.len());
            let field = &after[..end];
            if !is_identifier(field) {
                return None;
            }
            steps.push(PathStep::Field(Ident::new(field)));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix('[') {
            let end = after.find(']')?;
            let digits = &after[..end];
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            steps.push(PathStep::Index(digits.parse().ok()?));
            rest = &after[end + 1..];
        } else {
            return None;
        }
    }
    Some(PathExpr::new(raw, Ident::new(root), steps))
}

/// Read one closed-vocabulary keyword, reporting the alternatives when the
/// value is not one of them.
pub(crate) fn keyword<T: Copy>(
    node: &Node,
    subject: &str,
    variants: &[(&'static str, T)],
    cx: &mut Cx,
) -> Option<Spanned<T>> {
    let names: Vec<&str> = variants.iter().map(|(name, _)| *name).collect();
    let Yaml::String(text) = &node.value else {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownVariant,
                node.span.clone(),
                format!(
                    "expected one of {} for {subject}, found {}",
                    list(&names),
                    node.description()
                ),
            )
            .with_help(format!("{subject} takes one of {}", list(&names))),
        );
        return None;
    };
    if let Some((_, value)) = variants.iter().find(|(name, _)| name == text) {
        return Some(Spanned::new(*value, node.span.clone()));
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::UnknownVariant,
            node.span.clone(),
            format!("`{text}` is not a valid {subject}"),
        )
        .with_optional_help(
            suggest(text, &names)
                .map(|name| format!("did you mean `{name}`?"))
                .or_else(|| Some(format!("{subject} takes one of {}", list(&names)))),
        ),
    );
    None
}

/// Whether one `/`-separated segment is a portable file-name segment
/// (grammar 1.4's path form): `[A-Za-z0-9_][A-Za-z0-9_.-]*`.
///
/// `.` and `..` are handled by the callers, which give them meaning rather than
/// treating them as names.
///
/// **The charset lives here for both path surfaces.** A `module:` binding reads
/// it through [`is_relative_path`] and an `imports:` entry through
/// `super::section::import_problem`, which needs the tests separately — it
/// reports *why* a path is not one rather than only that it is not — but not a
/// second copy of the alphabet. Two copies agree on the day they are written.
pub(crate) fn is_path_segment(segment: &str) -> bool {
    let mut bytes = segment.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
}

/// Whether a written path takes grammar 1.4's portable form, with `extensions`
/// as the endings its last segment may take.
///
/// A `module:` binding's whole verdict, and the *charset* half of an `imports:`
/// entry's — which `super::section::import_problem` reads through
/// [`is_path_segment`], because that surface answers with the reason a path is
/// not portable rather than with a bare `false`. "One portable spelling"
/// (grammar 1.4) is a property of paths in this DSL rather than of one key that
/// carries them, so the alphabet is written once whatever asks.
#[must_use]
pub(crate) fn is_relative_path(path: &str, extensions: &[&str]) -> bool {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') {
        return false;
    }
    if path.chars().any(char::is_whitespace) {
        return false;
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|segment| !matches!(*segment, "." | "..") && !is_path_segment(segment))
    {
        return false;
    }
    let Some(last) = segments.last() else {
        return false;
    };
    extensions
        .iter()
        .any(|extension| last.len() > extension.len() && last.ends_with(extension))
}

/// Normalize a `/`-separated relative path lexically, or `None` when it climbs
/// out of the project root.
///
/// Lexical rather than filesystem-based on purpose: resolving symlinks would
/// make the composition depend on the machine it is resolved on, which PRD 5.12
/// forbids.
///
/// The root is a fence rather than a floor: a `..` that pops past position 0
/// fails the path even when a later segment would climb back in. Deciding
/// `../proj/f.yml` would mean comparing `proj` against the name of the directory
/// the project was checked out into — so the same composition would resolve on
/// one machine and not on another, which is the thing grammar 1.4's
/// one-portable-spelling rule and PRD 5.12 both exist to prevent. It cannot even
/// be asked uniformly: an entrypoint given as a bare `main.yml` has `""` for a
/// root, and `""` has no name to compare against.
///
/// Shared by the two path surfaces for the reason [`is_relative_path`] is
/// shared: `imports:` and a `module:` binding are both project-relative names,
/// and two normalizers would be two answers to where the root is.
#[must_use]
pub(crate) fn normalize_relative(path: &str) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "." | "" => {}
            ".." => {
                segments.pop()?;
            }
            name => segments.push(name),
        }
    }
    if segments.is_empty() {
        return None;
    }
    Some(segments.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_take_one_portable_form() {
        assert!(is_relative_path("src/tools/sign.ts", &[".ts"]));
        assert!(is_relative_path("./src/tools/sign.ts", &[".ts"]));
        assert!(is_relative_path("a/../b.ts", &[".ts"]));
        assert!(!is_relative_path("/opt/sign.ts", &[".ts"]));
        assert!(!is_relative_path("src\\tools\\sign.ts", &[".ts"]));
        assert!(!is_relative_path("src/my tool.ts", &[".ts"]));
        assert!(!is_relative_path("https://host/sign.ts", &[".ts"]));
        assert!(!is_relative_path("src/tools/sign.js", &[".ts"]));
        assert!(!is_relative_path("src/tools/.ts", &[".ts"]));
        assert!(!is_relative_path("", &[".ts"]));
        assert!(is_relative_path("models.yml", &[".yml", ".yaml"]));
    }

    #[test]
    fn paths_normalize_lexically() {
        assert_eq!(
            normalize_relative("agents/reviewer.yml").as_deref(),
            Some("agents/reviewer.yml")
        );
        assert_eq!(
            normalize_relative("./models.yml").as_deref(),
            Some("models.yml")
        );
        assert_eq!(normalize_relative("a/./b.yml").as_deref(), Some("a/b.yml"));
        assert_eq!(normalize_relative("a/../b.yml").as_deref(), Some("b.yml"));
        assert_eq!(
            normalize_relative("a/b/../../c.yml").as_deref(),
            Some("c.yml")
        );
        assert_eq!(normalize_relative("../outside.yml"), None);
        assert_eq!(normalize_relative("a/../../outside.yml"), None);
        // A path that climbs out and back in is refused too: whether it landed
        // inside would depend on the name of the directory the project was
        // checked out into, and a bare `main.yml` entrypoint has no such name.
        assert_eq!(normalize_relative("../project/models.yml"), None);
        assert_eq!(normalize_relative("a/../../a/models.yml"), None);
    }

    #[test]
    fn identifier_grammar() {
        assert!(is_identifier("a"));
        assert!(is_identifier("review_loop"));
        assert!(is_identifier("agent2"));
        assert!(!is_identifier(""));
        assert!(!is_identifier("Review"));
        assert!(!is_identifier("2fast"));
        assert!(!is_identifier("_leading"));
        assert!(!is_identifier("kebab-case"));
        assert!(!is_identifier(&"a".repeat(65)));
        assert!(is_identifier(&"a".repeat(64)));
    }

    #[test]
    fn duration_grammar() {
        assert_eq!(parse_duration("30s").unwrap().as_millis(), 30_000);
        assert_eq!(parse_duration("250ms").unwrap().as_millis(), 250);
        assert_eq!(parse_duration("5m").unwrap().as_millis(), 300_000);
        assert_eq!(parse_duration("24h").unwrap().as_millis(), 86_400_000);
        assert!(parse_duration("1m30s").is_none());
        assert!(parse_duration("0s").is_none());
        assert!(parse_duration("-5s").is_none());
        assert!(parse_duration("5").is_none());
        assert!(parse_duration("5 s").is_none());
        assert!(parse_duration("5sec").is_none());
        assert!(parse_duration("").is_none());
        assert!(parse_duration("ms").is_none());

        // Grammar 4.4 states a form and no ceiling, so a magnitude past 64 bits
        // is a well-formed duration this implementation counts saturated —
        // never a malformed *form*, which is what reporting it as invalid would
        // tell the author to go looking for. The two absurd magnitudes below
        // already mean the same thing: `as_millis` saturates both.
        let representable = parse_duration("18446744073709551615h").unwrap();
        let past_u64 = parse_duration("99999999999999999999h").unwrap();
        assert_eq!(representable.as_millis(), u64::MAX);
        assert_eq!(past_u64.as_millis(), u64::MAX);
        assert_eq!(past_u64.amount, u64::MAX);
        // The text survives exactly as written.
        assert_eq!(past_u64.as_str(), "99999999999999999999h");
    }

    #[test]
    fn env_reference_forms() {
        assert!(is_env_ref_value("${ANTHROPIC_API_KEY}"));
        assert!(is_env_ref_value("${_A0}"));
        assert!(!is_env_ref_value("${lower}"));
        assert!(!is_env_ref_value("prefix ${A}"));
        assert!(!is_env_ref_value("${A} suffix"));
        assert!(!is_env_ref_value("${0A}"));
    }

    #[test]
    fn env_token_scanning_honours_the_escape() {
        let tokens = scan_env_tokens("https://${HOST}/v1/${PATH}");
        assert_eq!(
            tokens
                .iter()
                .filter_map(|t| t.name.clone())
                .collect::<Vec<_>>(),
            ["HOST", "PATH"]
        );
        assert!(scan_env_tokens("$${FOO}").is_empty());
        assert!(scan_env_tokens("costs $5 and $$").is_empty());
        assert_eq!(scan_env_tokens("${unterminated").len(), 1);
        assert_eq!(scan_env_tokens("${lower}").len(), 1);
        assert!(scan_env_tokens("${lower}")[0].name.is_none());
    }

    #[test]
    fn path_expression_grammar() {
        let path = parse_path("plan.output.tasks").unwrap();
        assert_eq!(path.root.as_str(), "plan");
        assert_eq!(path.steps.len(), 2);
        let indexed = parse_path("state.batches[0].items").unwrap();
        assert_eq!(indexed.steps.len(), 3);
        assert!(parse_path("plan.output.tasks.filter(t, t.ready)").is_none());
        assert!(parse_path("a + b").is_none());
        assert!(parse_path("plan.").is_none());
        assert!(parse_path(".output").is_none());
        assert!(parse_path("plan[x]").is_none());
        assert!(parse_path("plan[]").is_none());
        assert!(parse_path("Plan.output").is_none());
        assert!(parse_path("").is_none());
    }

    #[test]
    fn reference_expectations_read_as_prose() {
        assert_eq!(
            reference_expectation(&[Namespace::Model]),
            "a `model.*` reference"
        );
        assert_eq!(
            reference_expectation(&[Namespace::Tool, Namespace::Flow]),
            "a `tool.*` or `flow.*` reference"
        );
        assert_eq!(
            reference_expectation(&[Namespace::Agent, Namespace::Tool, Namespace::Flow]),
            "an `agent.*`, `tool.*`, or `flow.*` reference"
        );
    }
}
