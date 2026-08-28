//! The spec-file sections that are not definitions: `imports`, `state`, and
//! `triggers` (grammar 1.4, 10, 13).

use crate::ast::common::Namespace;
use crate::ast::document::{Channel, ImportPath, ImportsSection, Reduce, StateSection};
use crate::ast::schema::{Surface, TypeForm, TypeNode};
use crate::ast::trigger::{
    AuthScheme, BearerAuth, CallbackAllow, CallbackAuth, CallbackHmac, EventTrigger, HmacAlgorithm,
    HmacAuth, HttpTrigger, Respond, ScheduleTrigger, SignatureEncoding, TRIGGER_TYPES, Trigger,
    TriggerKind, TriggerMethod, TriggersSection,
};
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
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

const HMAC_ALGORITHMS: &[(&str, HmacAlgorithm)] = &[
    ("sha1", HmacAlgorithm::Sha1),
    ("sha256", HmacAlgorithm::Sha256),
    ("sha512", HmacAlgorithm::Sha512),
];

const SIGNATURE_ENCODINGS: &[(&str, SignatureEncoding)] = &[
    ("hex", SignatureEncoding::Hex),
    ("base64", SignatureEncoding::Base64),
];

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

    let auth = inbound_auth(fields, subject, cx);
    // Whether the author *wrote* a `callback:`, not whether one survived: a
    // `callback:` refused for its own reason above is still a webhook the author
    // declared, and reporting these two keys as inert on top of that would be a
    // report about a key that is there. Decided from `contains` for the reason
    // the provider credential rule is (grammar 12.1, Decision D120).
    let declares_callback = fields.contains("callback");
    let callback_auth = callback_auth(fields, subject, declares_callback, cx);
    let callback_allow = callback_allow(fields, subject, declares_callback, cx);
    // A `callback_auth:` on a trigger with no webhook has already been refused,
    // and the allowlist it would demand is an allowlist for nothing.
    if declares_callback {
        require_callback_allow(fields, subject, cx);
    }

    HttpTrigger {
        path,
        method,
        input,
        respond,
        timeout,
        callback,
        auth,
        callback_auth,
        callback_allow,
    }
}

/// Read `auth:` — how an inbound call to this trigger is authenticated
/// (grammar 13.3, PRD resolved q32).
///
/// Exactly one scheme, which is the same shape a `tool.*` implementation
/// binding takes and is refused the same way (Decision D25): a block declaring
/// none is a `missing-key` naming both spellings, a block declaring both is a
/// `conflicting-keys` on the second. A request carries one credential, so
/// "verify either" is not a posture this grammar can express — it would leave
/// the deployment as open as its weaker half.
fn inbound_auth(
    fields: &mut Fields<'_>,
    subject: &str,
    cx: &mut Cx,
) -> Option<Spanned<AuthScheme>> {
    let entry = fields.take_entry("auth")?;
    let mapping = expect_mapping(&entry.value, &format!("the `auth` of {subject}"), cx)?;
    let context = format!("the `auth` of {subject}");
    let mut block = Fields::new(mapping, entry.value.span.clone(), &context);
    block.note_known(AuthScheme::KEYS);
    let declared: Vec<&str> = AuthScheme::KEYS
        .iter()
        .copied()
        .filter(|key| block.contains(key))
        .collect();

    let scheme = match declared.as_slice() {
        [] => {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingKey,
                    entry.key.span.clone(),
                    format!(
                        "{context} declares no scheme: an inbound `auth` carries exactly one of {}",
                        list(AuthScheme::KEYS)
                    ),
                )
                .with_help(
                    "`bearer` compares a static secret against a named header, `hmac` verifies a signature over the raw request body — drop `auth:` altogether to leave the route open (grammar 13.3, PRD resolved q32)",
                ),
            );
            None
        }
        [single] => auth_scheme(&mut block, single, &context, cx),
        [first, rest @ ..] => {
            for key in rest {
                let span = block
                    .take_entry(key)
                    .map_or_else(|| block.span.clone(), |entry| entry.key.span.clone());
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::ConflictingKeys,
                        span,
                        format!(
                            "{context} declares both `{first}` and `{key}`; an inbound `auth` carries exactly one scheme"
                        ),
                    )
                    .with_help(
                        "one request carries one credential, so a route verifying either would be exactly as open as its weaker half: keep the scheme the caller actually sends (grammar 13.3, PRD resolved q32)",
                    ),
                );
            }
            auth_scheme(&mut block, first, &context, cx)
        }
    };
    block.finish(cx);
    scheme.map(|scheme| Spanned::new(scheme, entry.value.span.clone()))
}

/// The one scheme an `auth:` block declares, read under its own key.
fn auth_scheme(
    block: &mut Fields<'_>,
    key: &str,
    context: &str,
    cx: &mut Cx,
) -> Option<AuthScheme> {
    match key {
        "bearer" => bearer(block, "bearer", context, cx).map(AuthScheme::Bearer),
        "hmac" => {
            let node = block.take("hmac")?;
            let context = format!("the `hmac` of {context}");
            let mapping = expect_mapping(node, &context, cx)?;
            let mut scheme = Fields::new(mapping, node.span.clone(), &context);
            let secret = scheme
                .require("secret", cx)
                .and_then(|node| lexical::env_ref(node, "`secret`", cx));
            let header = scheme
                .take("header")
                .and_then(|node| lexical::non_empty_text(node, "`header`", cx))
                .filter(|header| header_shape(header, cx));
            let algorithm = scheme
                .take("algorithm")
                .and_then(|node| lexical::keyword(node, "`algorithm`", HMAC_ALGORITHMS, cx));
            let encoding = scheme
                .take("encoding")
                .and_then(|node| lexical::keyword(node, "`encoding`", SIGNATURE_ENCODINGS, cx));
            // A prefix is legitimately empty — that is the default — so it is
            // read as plain text rather than as a non-empty value.
            let prefix = scheme
                .take("prefix")
                .and_then(|node| lexical::text(node, "`prefix`", cx))
                .filter(|prefix| prefix_shape(prefix, cx));
            scheme.finish(cx);
            Some(AuthScheme::Hmac(HmacAuth {
                secret,
                header,
                algorithm,
                encoding,
                prefix,
            }))
        }
        other => unreachable!("`{other}` is not one of `AuthScheme::KEYS`"),
    }
}

/// Read a `bearer:` block, inbound or outbound: one shape, one pair of defaults
/// (grammar 13.3).
fn bearer(
    fields: &mut Fields<'_>,
    key: &'static str,
    context: &str,
    cx: &mut Cx,
) -> Option<BearerAuth> {
    let node = fields.take(key)?;
    let context = format!("the `{key}` of {context}");
    let mapping = expect_mapping(node, &context, cx)?;
    let mut scheme = Fields::new(mapping, node.span.clone(), &context);
    let token = scheme
        .require("token", cx)
        .and_then(|node| lexical::env_ref(node, "`token`", cx));
    let header = scheme
        .take("header")
        .and_then(|node| lexical::non_empty_text(node, "`header`", cx))
        .filter(|header| header_shape(header, cx));
    let prefix = scheme
        .take("prefix")
        .and_then(|node| lexical::text(node, "`prefix`", cx))
        .filter(|prefix| prefix_shape(prefix, cx));
    scheme.finish(cx);
    Some(BearerAuth {
        token,
        header,
        prefix,
    })
}

/// Whether a `header:` names one HTTP header, in the form `headers:` keys take
/// (grammar 12.1, `NameForm::HeaderLike`).
///
/// The resolved name is what an inbound check looks a credential up by — case-
/// insensitively, as HTTP header names are — and what a delivery writes onto its
/// own request, verbatim; codegen reads resolved values and re-derives nothing.
/// So a space, a colon or a newline here does not name a header awkwardly, it
/// forges a second one, and the value is held to a name's form for the reason
/// `path:` above refuses whitespace.
fn header_shape(header: &Spanned<String>, cx: &mut Cx) -> bool {
    if header
        .value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return true;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            header.span.clone(),
            format!(
                "`header` must be an HTTP header name, found {:?}",
                header.value
            ),
        )
        .with_help(
            "a header name is letters, digits, `_`, and `-` — the form a provider's `headers:` keys take: outbound the resolved name is written onto the request as it stands, so a space, a colon or a newline in it would forge a second header rather than name this one, and inbound it is the name a caller's header is looked up by, which no such spelling ever is",
        ),
    );
    false
}

/// Whether a `prefix:` is one a header value can carry, ahead of the credential
/// it introduces (grammar 13.3).
///
/// A prefix is legitimately empty and legitimately punctuated — `"Bearer "`,
/// `"sha256="` — so the only shape it is held to is the one a header value
/// cannot survive: a carriage return or a newline in it ends that field and
/// begins another, which is `header:`'s injection again by the other half.
/// Both directions, for one reason read twice: written ahead of the credential
/// on a delivery, it forges a field; expected ahead of the credential a caller
/// sent, it is a byte no caller could have put there, so it matches nothing.
fn prefix_shape(prefix: &Spanned<String>, cx: &mut Cx) -> bool {
    if !prefix.value.chars().any(char::is_control) {
        return true;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            prefix.span.clone(),
            format!(
                "`prefix` must not contain control characters, found {:?}",
                prefix.value
            ),
        )
        .with_help(
            "the prefix stands between the header and the credential — written ahead of it outbound, expected ahead of it inbound — and a carriage return or newline is neither: outbound it ends that header field and begins another, inbound it is a byte no header value a caller sent can carry; keep it to visible characters and spaces, as in `Bearer ` or `sha256=`",
        ),
    );
    false
}

/// Whether a `callback_auth.bearer.header:` names a header the delivery does not
/// already write itself (grammar 13.3, Decision D127).
///
/// The `X-AgentCompose-` namespace belongs to the wire contract: every delivery
/// carries `X-AgentCompose-Event`, `-Delivery`, `-Ordinal` and `-Timestamp`, and
/// a signed one carries `-Signature`. Those names are *normative* — a receiver
/// is written against them rather than against an observed release — so a static
/// token asked for under one of them arrives joined to the value the delivery
/// wrote, or in place of it, and the receiver's check then fails on every
/// legitimate delivery or passes on one whose signature was never read.
///
/// Only outbound, and deliberately: a trigger that *receives* agent-compose
/// deliveries verifies them by naming `X-AgentCompose-Signature` in its inbound
/// `auth:`, exactly as it would name any other vendor's header.
fn delivery_header_is_free(header: &Spanned<String>, cx: &mut Cx) -> bool {
    let prefix = CallbackAuth::DELIVERY_HEADER_PREFIX;
    if !header
        .value
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
    {
        return true;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            header.span.clone(),
            format!(
                "`header` must not name an `{prefix}` delivery header, found {:?}",
                header.value
            ),
        )
        .with_help(
            "every delivery already writes `X-AgentCompose-Event`, `-Delivery`, `-Ordinal` and `-Timestamp`, and a signed one writes `-Signature`; those names are the receiver's contract, so a token carried under one of them replaces or joins the value the receiver reads — name the header the receiver expects the token on (grammar 13.3, Decision D127)",
        ),
    );
    false
}

/// Read `callback_auth:` — how a delivery identifies itself to its receiver
/// (grammar 13.3, PRD resolved q33).
///
/// **At least** one scheme, unlike inbound `auth:`: a delivery is this
/// deployment's own request, so signing it and carrying a token are two things
/// one receiver may both want, and the resolved question says "and/or".
fn callback_auth(
    fields: &mut Fields<'_>,
    subject: &str,
    declares_callback: bool,
    cx: &mut Cx,
) -> Option<Spanned<CallbackAuth>> {
    let entry = fields.take_entry("callback_auth")?;
    if !declares_callback {
        cx.push(inert_callback_key(
            &entry.key.span,
            "callback_auth",
            subject,
        ));
        return None;
    }
    let context = format!("the `callback_auth` of {subject}");
    let mapping = expect_mapping(&entry.value, &context, cx)?;
    let mut block = Fields::new(mapping, entry.value.span.clone(), &context);
    block.note_known(CallbackAuth::KEYS);
    if !CallbackAuth::KEYS.iter().any(|key| block.contains(key)) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                entry.key.span.clone(),
                format!(
                    "{context} declares no scheme: a `callback_auth` carries {}, or both",
                    list(CallbackAuth::KEYS)
                ),
            )
            .with_help(
                "`hmac` signs the delivered body and `bearer` sends a static token; a delivery that carries neither is what leaving `callback_auth:` out already means, and that posture needs no allowlist (grammar 13.3, PRD resolved q33)",
            ),
        );
        block.finish(cx);
        return None;
    }
    // The one difference between an outbound `bearer:` and an inbound one, and
    // it is about the *other* headers on the same request rather than about
    // this key's shape — so it is applied here, where the direction is known,
    // rather than inside the shared reader.
    let bearer = bearer(&mut block, "bearer", &context, cx).map(|mut scheme| {
        if scheme
            .header
            .as_ref()
            .is_some_and(|header| !delivery_header_is_free(header, cx))
        {
            scheme.header = None;
        }
        scheme
    });
    let hmac = block.take("hmac").and_then(|node| {
        let context = format!("the `hmac` of {context}");
        let mapping = expect_mapping(node, &context, cx)?;
        let mut scheme = Fields::new(mapping, node.span.clone(), &context);
        // One key, and the omission is the decision: outbound signing is fixed
        // HMAC-SHA256 in hex under `X-AgentCompose-Signature`, so a receiver
        // verifying one agent-compose deployment verifies them all.
        let secret = scheme
            .require("secret", cx)
            .and_then(|node| lexical::env_ref(node, "`secret`", cx));
        scheme.finish(cx);
        Some(CallbackHmac { secret })
    });
    block.finish(cx);
    Some(Spanned::new(
        CallbackAuth { bearer, hmac },
        entry.value.span.clone(),
    ))
}

/// Read `callback_allow:` — the URL patterns a callback may point at
/// (grammar 13.3, PRD resolved q33).
///
/// Entry *shape* only: whether a URL the payload supplied matches one of these
/// is decided when it is read, at parking or settle, since the URL does not
/// exist until then.
fn callback_allow(
    fields: &mut Fields<'_>,
    subject: &str,
    declares_callback: bool,
    cx: &mut Cx,
) -> Option<CallbackAllow> {
    let entry = fields.take_entry("callback_allow")?;
    if !declares_callback {
        cx.push(inert_callback_key(
            &entry.key.span,
            "callback_allow",
            subject,
        ));
        return None;
    }
    let items = expect_sequence(
        &entry.value,
        &format!("the `callback_allow` of {subject}"),
        cx,
    )?;
    if items.is_empty() {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                entry.value.span.clone(),
                format!("the `callback_allow` of {subject} admits no URL"),
            )
            .with_help(
                "an allowlist matched against every callback URL and satisfied by none refuses every delivery, so an empty list is a webhook that can never fire: name the patterns this trigger may POST to, or drop `callback_allow:` and `callback_auth:` together (grammar 13.3, PRD resolved q33)",
            ),
        );
        return None;
    }
    let mut patterns = Vec::new();
    for item in items {
        // The patterns are part of what the composition *is* and are shape-
        // checked here, so they are class-3 strings: a `${NAME}` token is an
        // error rather than a value that only exists at process start
        // (grammar 4.3, Decision D92).
        let Some(pattern) = lexical::text(item, "each entry of `callback_allow`", cx) else {
            continue;
        };
        if let Some(problem) = allow_problem(&pattern.value) {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    pattern.span.clone(),
                    format!(
                        "{:?} is not a `callback_allow` pattern of {subject}: it {problem}",
                        pattern.value
                    ),
                )
                .with_help(ALLOW_HELP),
            );
            continue;
        }
        patterns.push(pattern);
    }
    Some(CallbackAllow {
        patterns,
        span: entry.value.span.clone(),
    })
}

/// Why a `callback_allow:` entry is not a legal URL pattern, if it is not.
///
/// Completes ``` "<entry>" is not a `callback_allow` pattern of …: it … ```.
/// The entry is quoted rather than backticked, as `header:` and `prefix:` are
/// above: an empty pattern and a pattern that is three spaces are both things an
/// author writes, and only quoting shows the difference between them.
///
/// The scheme and a host are the whole of the check. A pattern is matched
/// against the URL a payload supplied, so a match that could not name the
/// scheme would let `https://hooks.example.com/*` admit `javascript:` or
/// `file:` URLs beginning with the same characters — and an entry that names no
/// host, `https:///deliveries`, names no receiver for the list to admit, which
/// is the one shape of this key that reads like a constraint and is not one
/// (Decision D127).
///
/// What a legal entry *means* is not this pass's to narrow. `*` is any run of
/// characters and crosses `/` and `?` like every other, so
/// `https://*.hooks.example.com/*` admits more URLs than the author of a
/// subdomain tree intends, and §13.3 says so where an author writes one.
/// Refusing the shape would settle the open question the other way: a wildcard
/// bounded inside the authority is a second wildcard kind, which the PRD owns,
/// and on a signed trigger the only compiling repair left would be dropping
/// `callback_auth:` — the posture [`require_callback_allow`] exists to prevent
/// (D127).
///
/// The arms are ordered so each one is the *only* answer to some entry — an
/// empty entry names the emptiness, a blank one names the whitespace — because
/// an arm no entry reaches is a message no fixture pins and no reader has read.
fn allow_problem(pattern: &str) -> Option<String> {
    if pattern.is_empty() {
        return Some("is empty".to_string());
    }
    if pattern.chars().any(char::is_whitespace) {
        return Some("contains whitespace".to_string());
    }
    let Some((scheme, rest)) = pattern.split_once("://") else {
        return Some("names no scheme, and a callback URL is absolute".to_string());
    };
    if !matches!(scheme, "http" | "https") {
        return Some(format!(
            "names the scheme `{scheme}`, and a callback is delivered over `http` or `https`"
        ));
    }
    if rest.is_empty() {
        return Some("names a scheme and nothing else".to_string());
    }
    // The host runs from the scheme to whichever of `/`, `?` and `#` ends it —
    // the same three delimiters that end an authority in a URL. Only its
    // emptiness is read: a delimiter sitting where the host should be is an
    // entry no callback URL was ever meant to match.
    let host = rest.find(['/', '?', '#']).map_or(rest, |end| &rest[..end]);
    if host.is_empty() {
        let delimiter = rest
            .chars()
            .next()
            .expect("a non-empty rest has a first character");
        return Some(format!(
            "names no host between `{scheme}://` and the `{delimiter}` that follows it"
        ));
    }
    None
}

/// What an entry is, for every refusal.
const ALLOW_HELP: &str = "an entry is an absolute URL naming a host, with `*` standing for any run of characters, matched against the whole callback URL — `https://hooks.example.com/*`; `http` stays legal, which is what makes localhost development work (grammar 13.3, PRD resolved q33)";

/// A `callback_auth:`/`callback_allow:` on a trigger that delivers no webhook
/// (grammar 13.3).
///
/// The mirror of `timeout:` on an async trigger (Decision D81) and of
/// `callback:` on a synchronous one: the key describes a delivery this trigger
/// never makes, so it changes nothing observable and gets a diagnostic rather
/// than silence.
fn inert_callback_key(span: &Span, key: &str, subject: &str) -> Diagnostic {
    Diagnostic::error(
        DiagnosticCode::ConflictingKeys,
        span.clone(),
        format!("`{key}` is not legal on {subject}, which declares no `callback`"),
    )
    .with_help(
        "both keys describe how a completion webhook is delivered, and a trigger with no `callback:` delivers none: declare the webhook, or drop the key (grammar 13.3, PRD resolved q33)",
    )
}

/// Declaring `callback_auth:` makes `callback_allow:` mandatory
/// (grammar 13.3, PRD resolved q33).
///
/// Read from `contains` rather than from the parsed blocks, so a `callback_auth`
/// refused for its own reason still demands the allowlist it would have needed:
/// what the rule is about is the author having declared outbound authentication
/// at all.
fn require_callback_allow(fields: &Fields<'_>, subject: &str, cx: &mut Cx) {
    if !fields.contains("callback_auth") || fields.contains("callback_allow") {
        return;
    }
    let span = fields
        .span_of("callback_auth")
        .unwrap_or_else(|| fields.span.clone());
    cx.push(
        Diagnostic::error(
            DiagnosticCode::MissingCallbackAllowlist,
            span,
            format!("{subject} declares `callback_auth` and no `callback_allow`"),
        )
        .with_help(
            "the callback URL comes from the request payload and is attacker-controlled by construction, so a deployment careful enough to authenticate its deliveries must not send them wherever a payload said: declare `callback_allow:` with the URL patterns this trigger may POST to, or drop `callback_auth:` and take the documented test posture, which may POST anywhere (grammar 13.3, PRD resolved q33)",
        ),
    );
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
