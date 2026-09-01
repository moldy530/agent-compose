//! The deploy layer's sections (grammar 14).

use std::collections::BTreeMap;

use crate::ast::common::Namespace;
use crate::ast::definition::StoreKind;
use crate::ast::deploy::{
    BackendAlias, BackendConfig, BackendDefault, BackendProvider, ConnectionField, EventSource,
    EventSourceKind, EventSourcesSection, HubSection, Placement, PlacementsSection, PluginEntry,
    PluginValue, SECRET_FIELDS, StorageBackendsSection, TraceSinkFormat, TraceSinkSection,
};
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::yaml::{Mapping, Node, Yaml};

use super::definition::description;
use super::lexical;
use super::reader::{Cx, Fields, expect_finite, expect_mapping, expect_sequence, list};
use super::section;

/// Namespaces a `members:` entry may name (grammar 14.1, Decision D129).
///
/// `flow.*` is deliberately not one of them, and is deliberately not left to
/// this list to refuse: a `flow.` prefix is intercepted before the address is
/// read and answered with the **deferral**, because a reader who wrote one
/// named a real component in a real position, and "expected an `agent.*` or
/// `tool.*` reference" would read as a spelling correction for a decision the
/// PRD took deliberately (PRD resolved q44's out-list). Every other namespace
/// here really is a mistake, and this list is what tells its author what the
/// position takes.
const MEMBERS: &[Namespace] = &[Namespace::Agent, Namespace::Tool];

/// Read the `placements:` section (grammar 14.1).
///
/// A placement is a **named** claim, and its members are the components a
/// worker asserting that name runs (PRD resolved q38). Three rules are decided
/// here because each is decidable from this file alone: the members' lexical
/// form, the `flow.*` deferral, and disjointness across the section. Whether a
/// member *resolves* needs the composition and is
/// [`resolve::target`](crate::resolve)'s; whether a placed tool agrees with the
/// agents that attach it needs both and is `check::placements`'s.
pub(crate) fn placements(node: &Node, cx: &mut Cx) -> Option<PlacementsSection> {
    let mapping = expect_mapping(node, "`placements`", cx)?;
    let mut placements = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = lexical::key_identifier(&entry.key, "a placement name", cx) else {
            continue;
        };
        let subject = format!("placement `{}`", name.value);
        let Some(body) = expect_mapping(&entry.value, &subject, cx) else {
            continue;
        };
        let mut fields = Fields::new(body, entry.value.span.clone(), &subject);
        let members = fields
            .require("members", cx)
            .map(|node| members_of(node, &subject, cx))
            .unwrap_or_default();
        let description = description(&mut fields, cx);
        fields.finish(cx);

        placements.push(Placement {
            name,
            members,
            description,
            span: entry.key.span.joined(&entry.value.span),
        });
    }
    disjoint(&placements, cx);
    Some(PlacementsSection {
        placements,
        span: node.span.clone(),
    })
}

/// Read one placement's `members:` list.
///
/// Duplicates *within* one list are refused here rather than left to
/// [`disjoint`], and the two rules are different rules. Disjointness is about
/// two placements holding two answers; a component written twice in one list
/// holds one answer, written twice, and "a member of both `mac` and `mac`"
/// would name no choice an author could make. The wording and the code are
/// grammar 5.4's, which already refuses a repeated entry of an agent's `tools:`
/// or `stores:` — one repeated-entry rule, spelled one way.
fn members_of(
    node: &Node,
    subject: &str,
    cx: &mut Cx,
) -> Vec<Spanned<crate::ast::common::Address>> {
    let Some(items) = expect_sequence(node, &format!("the `members` of {subject}"), cx) else {
        return Vec::new();
    };
    if items.is_empty() {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                node.span.clone(),
                format!("the `members` of {subject} names no component"),
            )
            .with_help(
                "a placement is the set of components a worker claiming its name runs, so one with no members is a claim nothing is ever dispatched under: name the components it runs, or drop the placement (grammar 14.1, PRD resolved q38)",
            ),
        );
        return Vec::new();
    }
    let mut members = Vec::new();
    for item in items {
        // A reference position, read the way [`lexical::reference`] reads one:
        // the string is taken and the address is the only judgement passed on
        // it. Reading it as literal text instead would add the env-ref rule to
        // that judgement, and `members: [${SOME_AGENT}]` would draw two
        // diagnostics — "not an `agent.*` or `tool.*` reference" *and* "never
        // interpolates environment references" — for the one mistake that
        // `tools: [${SOME_TOOL}]` draws one for. `lexical::reference` is not
        // called only because the `flow.` prefix is intercepted below, before
        // the address is read, and that interception needs the string.
        let Yaml::String(value) = &item.value else {
            cx.wrong_type(
                item,
                "a placement member",
                &lexical::reference_expectation(MEMBERS),
            );
            continue;
        };
        let text = Spanned::new(value.clone(), item.span.clone());
        if text.value.starts_with("flow.") {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::UnsupportedPlacement,
                    text.span.clone(),
                    format!(
                        "`{}` may not be a placement member: placing a flow is deferred",
                        text.value
                    ),
                )
                .with_help(
                    "v1 places `agent.*` and `tool.*` — the leaves that hold a machine's capability — while a flow is a subgraph the hub schedules; place the nodes it reaches instead (grammar 14.1, PRD resolved q44)",
                ),
            );
            continue;
        }
        let Some(address) = lexical::address(&text, "a placement member", MEMBERS, cx) else {
            continue;
        };
        if let Some(first) = members
            .iter()
            .find(|other: &&Spanned<crate::ast::common::Address>| other.value == address.value)
        {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    address.span.clone(),
                    format!(
                        "the `members` of {subject} lists `{}` twice",
                        address.value
                    ),
                )
                .with_label(first.span.clone(), "first listed here")
                .with_help(
                    "a placement's members are the components a worker claiming its name runs, so naming one twice says exactly what naming it once said: delete the repeat (grammar 14.1)",
                ),
            );
            continue;
        }
        members.push(address);
    }
    members
}

/// Placements are disjoint: one component belongs to at most one of them
/// (grammar 14.1, Decision D129).
///
/// Two placements naming one component are two answers to "which worker runs
/// this", and the hub would have to pick one — so the pick is the author's,
/// made here, rather than a dispatch-time coin toss. The diagnostic names both,
/// with the first labelled, because the repair is a choice between two lines an
/// author wrote deliberately.
///
/// **Two** placements, always: a component repeated inside one list never
/// reaches this pass, because [`members_of`] drops the repeat and reports it as
/// what it is. A rule that names two placements must have two to name, or the
/// message becomes "a member of both `mac` and `mac`" and the repair it offers
/// is a choice between one thing and itself.
fn disjoint(placements: &[Placement], cx: &mut Cx) {
    let mut claimed: BTreeMap<String, (&str, Span)> = BTreeMap::new();
    for placement in placements {
        for member in &placement.members {
            let address = member.value.to_string();
            match claimed.get(&address) {
                Some((first, span)) => cx.push(
                    Diagnostic::error(
                        DiagnosticCode::ConflictingPlacement,
                        member.span.clone(),
                        format!(
                            "`{address}` is a member of both `{first}` and `{}`",
                            placement.name.value
                        ),
                    )
                    .with_label(span.clone(), format!("`{first}` claims it here"))
                    .with_help(
                        "placements are disjoint: a component runs on the workers claiming one placement, and two claims would leave the hub to choose — name it in one of the two (grammar 14.1, PRD resolved q38)",
                    ),
                ),
                None => {
                    claimed.insert(address, (placement.name.value.as_str(), member.span.clone()));
                }
            }
        }
    }
}

/// Read the `hub:` section (grammar 14.2, Decision D130).
///
/// A closed construct, like `storage_backends:` and unlike a backend config:
/// both keys are this compiler's own and an unknown one is a mistake rather
/// than a plugin's business.
pub(crate) fn hub(node: &Node, cx: &mut Cx) -> Option<HubSection> {
    let mapping = expect_mapping(node, "`hub`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "`hub`");
    let declares_join_token = fields.contains("join_token");
    let join_token = fields
        .take("join_token")
        .and_then(|node| lexical::env_ref(node, "`hub.join_token`", cx));
    let public_url = fields
        .take("public_url")
        .and_then(|node| public_url(node, cx));
    fields.finish(cx);
    Some(HubSection {
        join_token,
        public_url,
        declares_join_token,
        span: node.span.clone(),
    })
}

/// One absolute-URL key of the deploy layer, and how its refusals name it.
///
/// Two keys take an absolute URL written out — `hub.public_url:` (grammar 14.2)
/// and `trace_sink.url:` (grammar 14.5) — and they are held to **one** shape
/// rule with one arm inventory, because an author meeting both surfaces should
/// meet one set of URL rules. What differs is only the noun each refusal uses
/// for the thing being written, which is what these fields carry.
struct UrlSubject {
    /// The key, as a diagnostic spells it.
    key: &'static str,
    /// Completes "contains `*`, and … is this deployment's own URL rather than
    /// a pattern".
    own: &'static str,
    /// Completes "names no scheme, and … is absolute".
    absolute: &'static str,
    /// Completes "names the scheme `X`, and … — write `x://`", for a scheme
    /// written in the wrong case.
    cased: &'static str,
    /// Completes "names the scheme `x`, and … is reached over `http` or
    /// `https`".
    reached: &'static str,
    /// The help every refusal of this key carries.
    help: &'static str,
}

/// `hub.public_url:` — the ingress base every URL this deployment hands out
/// derives from (grammar 14.2, PRD resolved q44 invariant 4).
const PUBLIC_URL: UrlSubject = UrlSubject {
    key: "hub.public_url",
    own: "a public base",
    absolute: "an ingress base",
    cased: "a URL derived from it is written out as text",
    reached: "a hub",
    help: "the base is an absolute URL naming a host, its scheme written lowercase and no wildcard in it — `https://hub.example`; `http` stays legal, which is what makes localhost development work (grammar 14.2, PRD resolved q44)",
};

/// `trace_sink.url:` — where every settled execution's trace is POSTed
/// (grammar 14.5, PRD resolved q50).
const TRACE_SINK_URL: UrlSubject = UrlSubject {
    key: "trace_sink.url",
    own: "a sink address",
    absolute: "a sink address",
    cased: "the address is written onto the request as text",
    reached: "a trace sink",
    help: "the sink is an absolute URL naming a host, its scheme written lowercase and no wildcard in it — `https://collector.internal.example/v1/traces`; `http` stays legal, which is what makes a collector on the same host work (grammar 14.5, PRD resolved q50)",
};

/// Read `hub.public_url:` — the ingress base every URL this deployment hands
/// out derives from (grammar 14.2, PRD resolved q44 invariant 4).
///
/// The shape rules are `callback_allow:`'s minus the wildcard: an entry of that
/// list is a **pattern** matched against a URL somebody else supplied, and this
/// is our own base, written out. So a `*` here is a character in a hostname
/// rather than a match against anything, and a base carrying one is a URL that
/// resolves nowhere.
fn public_url(node: &Node, cx: &mut Cx) -> Option<Spanned<String>> {
    absolute_url(node, &PUBLIC_URL, cx)
}

/// Read one absolute-URL key, refusing what its [`UrlSubject`] describes.
///
/// A class-3 string (grammar 4.3, Decision D92): the value is part of what the
/// composition *is* and is shape-checked here, so a `${NAME}` token in it would
/// be a value this pass could not read. A deployment whose ingress — or whose
/// collector — differs per environment writes a different deploy file, which is
/// the layer's whole point.
fn absolute_url(node: &Node, subject: &UrlSubject, cx: &mut Cx) -> Option<Spanned<String>> {
    let text = lexical::text(node, &format!("`{}`", subject.key), cx)?;
    if let Some(problem) = url_problem(&text.value, subject) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                text.span.clone(),
                format!("{:?} is not a `{}`: it {problem}", text.value, subject.key),
            )
            .with_help(subject.help),
        );
        return None;
    }
    Some(text)
}

/// Why a value is not a legal absolute URL for `subject`, if it is not.
///
/// Completes ``` "<value>" is not a `<key>`: it … ```. The arms are ordered so
/// each is the only answer to some value, and every one of them is reached by
/// [`tests::every_refusal_arm_answers_some_url`] — an arm no value reaches is a
/// message no reader has read, and the negative fixture corpus pins one rule per
/// file rather than one arm. The wording follows `callback_allow:`'s (PRD
/// resolved q33), because an author meeting these surfaces should meet one set
/// of URL rules.
fn url_problem(url: &str, subject: &UrlSubject) -> Option<String> {
    if url.is_empty() {
        return Some("is empty".to_string());
    }
    if url.chars().any(char::is_whitespace) {
        return Some("contains whitespace".to_string());
    }
    if url.contains('*') {
        return Some(format!(
            "contains `*`, and {} is this deployment's own URL rather than a pattern",
            subject.own
        ));
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return Some(format!(
            "names no scheme, and {} is absolute",
            subject.absolute
        ));
    };
    if !matches!(scheme, "http" | "https") {
        if let Some(spelling) = ["http", "https"]
            .into_iter()
            .find(|known| known.eq_ignore_ascii_case(scheme))
        {
            return Some(format!(
                "names the scheme `{scheme}`, and {} — write `{spelling}://`",
                subject.cased
            ));
        }
        return Some(format!(
            "names the scheme `{scheme}`, and {} is reached over `http` or `https`",
            subject.reached
        ));
    }
    if rest.is_empty() {
        return Some("names a scheme and nothing else".to_string());
    }
    // The host runs from the scheme to whichever of `/`, `?` and `#` ends it —
    // the same three delimiters that end an authority in a URL, read the same
    // way `callback_allow:` reads them.
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

/// `join_token:` is required exactly where placements are (grammar 14.2,
/// Decision D130).
///
/// A rule about two sections of one file, so the parser owns it — the layer
/// `require_callback_allow` sits at for the same reason. What makes it its own
/// code rather than a `missing-key` is what `missing-credential` and
/// `missing-callback-allowlist` are: whether the key is required is decided by
/// a **sibling section's** contents, and the repair is a choice of two.
///
/// `written` is whether the file carries a `hub:` key **at all**, which is not
/// the same question as whether `hub` parsed: a `hub:` that is not a mapping is
/// refused by [`hub`] and reaches here as `None`. Telling the two apart is the
/// same distinction [`HubSection::declares_join_token`] draws one level in — the
/// author of `hub: "https://hub.example"` has already been told what is wrong
/// with it, and "declare `hub: { join_token: ${SOME_VAR} }`" is advice about a
/// block they wrote, so stating it would be a second diagnostic for one mistake
/// whose repair line is false about the file in front of the reader.
pub(crate) fn require_join_token(
    placements: Option<&PlacementsSection>,
    hub: Option<&HubSection>,
    written: bool,
    document: &Span,
    cx: &mut Cx,
) {
    let Some(section) = placements else { return };
    if section.placements.is_empty() {
        return;
    }
    if hub.is_some_and(|hub| hub.declares_join_token) {
        return;
    }
    if hub.is_none() && written {
        return;
    }
    // The `hub:` block is where the key belongs, so a file that has one is
    // pointed at it and a file that has none is pointed at itself — the same
    // anchoring the missing `version:` of a deploy file takes.
    let span = hub.map_or(document, |hub| &hub.span);
    cx.push(
        Diagnostic::error(
            DiagnosticCode::MissingJoinToken,
            span.clone(),
            "this target declares placements and no `hub.join_token`",
        )
        .with_label(
            section.span.clone(),
            format!(
                "{} declared here",
                match section.placements.len() {
                    1 => "one placement".to_string(),
                    count => format!("{count} placements"),
                }
            ),
        )
        .with_help(
            "a placement is claimed by a worker at an authenticated join, and the join token is the whole of that authentication — holding it is being trusted with the mesh: declare `hub: { join_token: ${SOME_VAR} }`, or remove the placements if this target runs in one process (grammar 14.2, PRD resolved q38)",
        ),
    );
}

/// The `format:` keywords a `trace_sink:` chooses between (grammar 14.5).
const TRACE_SINK_FORMATS: &[(&str, TraceSinkFormat)] = &[
    ("envelope", TraceSinkFormat::Envelope),
    ("otlp", TraceSinkFormat::Otlp),
];

/// Read the `trace_sink:` section (grammar 14.5, PRD resolved q50, q51).
///
/// A closed construct like `hub:` and unlike a backend config: all three keys
/// are this compiler's own, and an unknown one is a mistake rather than a
/// plugin's business (Decision D50).
///
/// **There is deliberately no allowlist key here, and none is required.**
/// `callback_allow:` exists because a callback URL comes out of a request
/// payload and is attacker-controlled by construction (grammar 13.3, Decision
/// D126); this address is written by the operator in the deploy file, and is
/// trusted exactly as a `storage_backends:` connection string is. Requiring a
/// list an operator would write to admit the address they just wrote on the line
/// above would be ceremony rather than a control (PRD resolved q50).
pub(crate) fn trace_sink(node: &Node, cx: &mut Cx) -> Option<TraceSinkSection> {
    let mapping = expect_mapping(node, "`trace_sink`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "`trace_sink`");
    let url = fields
        .require("url", cx)
        .and_then(|node| absolute_url(node, &TRACE_SINK_URL, cx));
    let format = fields
        .take("format")
        .and_then(|node| lexical::keyword(node, "`trace_sink.format`", TRACE_SINK_FORMATS, cx));
    let auth = fields.take_entry("auth").and_then(|entry| {
        let scheme = section::outbound_auth(
            &entry.key.span,
            &entry.value,
            "the `auth` of `trace_sink`",
            "an `auth`",
            TRACE_SINK_AUTH_WITHOUT_A_SCHEME,
            cx,
        )?;
        Some(Spanned::new(scheme, entry.value.span.clone()))
    });
    fields.finish(cx);
    Some(TraceSinkSection {
        url,
        format,
        auth,
        span: node.span.clone(),
    })
}

/// What a `trace_sink.auth:` declaring neither scheme is told.
///
/// The sibling of `callback_auth:`'s, and it ends differently for the reason the
/// section header gives: leaving the block out is a legitimate posture here —
/// an unauthenticated collector on a private network — and it buys no allowlist
/// obligation, because there is none to buy.
const TRACE_SINK_AUTH_WITHOUT_A_SCHEME: &str = "`hmac` signs the delivered body and `bearer` sends a static token; a delivery that carries neither is what leaving `auth:` out already means, which is the posture a collector on a private network takes (grammar 14.5, PRD resolved q50)";

const STORE_KINDS: &[(&str, StoreKind)] = &[
    ("kv", StoreKind::Kv),
    ("vector", StoreKind::Vector),
    ("blob", StoreKind::Blob),
];

/// Read the `storage_backends:` section (grammar 14.3).
pub(crate) fn storage_backends(node: &Node, cx: &mut Cx) -> Option<StorageBackendsSection> {
    let mapping = expect_mapping(node, "`storage_backends`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "`storage_backends`");

    let mut defaults = Vec::new();
    if let Some(node) = fields.take("defaults")
        && let Some(body) = expect_mapping(node, "`storage_backends.defaults`", cx)
    {
        for entry in body.entries() {
            let Some(kind) = lexical::keyword(
                &Node {
                    value: crate::yaml::Yaml::String(entry.key.value.clone()),
                    span: entry.key.span.clone(),
                },
                "store kind",
                STORE_KINDS,
                cx,
            ) else {
                continue;
            };
            let subject = format!("the `{}` backend default", kind.value.as_str());
            let Some(config) = backend_config(&entry.value, &subject, Some(kind.value), cx) else {
                continue;
            };
            defaults.push(BackendDefault { kind, config });
        }
    }

    let mut aliases = Vec::new();
    if let Some(node) = fields.take("aliases")
        && let Some(body) = expect_mapping(node, "`storage_backends.aliases`", cx)
    {
        for entry in body.entries() {
            let Some(name) = lexical::key_identifier(&entry.key, "backend alias", cx) else {
                continue;
            };
            let subject = format!("backend alias `{}`", name.value);
            let Some(config) = backend_config(&entry.value, &subject, None, cx) else {
                continue;
            };
            aliases.push(BackendAlias { name, config });
        }
    }
    fields.finish(cx);

    Some(StorageBackendsSection {
        defaults,
        aliases,
        span: node.span.clone(),
    })
}

fn backend_config(
    node: &Node,
    subject: &str,
    kind: Option<StoreKind>,
    cx: &mut Cx,
) -> Option<BackendConfig> {
    let mapping = expect_mapping(node, subject, cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), subject);

    let providers: Vec<(&str, BackendProvider)> = BackendProvider::ALL
        .iter()
        .map(|provider| (provider.as_str(), *provider))
        .collect();
    let provider = fields
        .require("provider", cx)
        .and_then(|node| lexical::keyword(node, "storage `provider`", &providers, cx))
        .filter(|provider| match kind {
            Some(kind) if provider.value.kind() != kind => {
                let accepted: Vec<&str> = BackendProvider::ALL
                    .iter()
                    .filter(|candidate| candidate.kind() == kind)
                    .map(|candidate| candidate.as_str())
                    .collect();
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        provider.span.clone(),
                        format!(
                            "`{}` is not a `{}` storage provider",
                            provider.value.as_str(),
                            kind.as_str()
                        ),
                    )
                    .with_help(format!(
                        "the `{}` providers are {}",
                        kind.as_str(),
                        list(&accepted)
                    )),
                );
                false
            }
            _ => true,
        });

    let (connection, extra) = plugin_config(mapping, &["provider"], subject, cx);
    Some(BackendConfig {
        provider,
        connection,
        extra,
        span: node.span.clone(),
    })
}

const EVENT_SOURCE_KINDS: &[(&str, EventSourceKind)] = &[
    ("redis_streams", EventSourceKind::RedisStreams),
    ("sqs", EventSourceKind::Sqs),
    ("nats", EventSourceKind::Nats),
];

/// Read the `event_sources:` section (grammar 14.4).
pub(crate) fn event_sources(node: &Node, cx: &mut Cx) -> Option<EventSourcesSection> {
    let mapping = expect_mapping(node, "`event_sources`", cx)?;
    let mut sources = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = lexical::key_identifier(&entry.key, "event source name", cx) else {
            continue;
        };
        let subject = format!("event source `{}`", name.value);
        let Some(body) = expect_mapping(&entry.value, &subject, cx) else {
            continue;
        };
        let mut fields = Fields::new(body, entry.value.span.clone(), &subject);
        let kind = fields
            .require("kind", cx)
            .and_then(|node| lexical::keyword(node, "event source `kind`", EVENT_SOURCE_KINDS, cx));
        let (connection, extra) = plugin_config(body, &["kind"], &subject, cx);
        sources.push(EventSource {
            name,
            kind,
            connection,
            extra,
            span: entry.key.span.joined(&entry.value.span),
        });
    }
    Some(EventSourcesSection {
        sources,
        span: node.span.clone(),
    })
}

/// Split an open plugin-config object into the connection fields grammar 4.3
/// closes over — which must be `${ENV}` value-form references — and everything
/// else, which the plugin's own published schema checks (Decision D50).
///
/// "Everything else" is not unchecked. Grammar 4.3 puts "non-secret
/// `storage_backends` and `event_sources` config values" in class 2 alongside
/// the `exec:` block and provider `headers`, so each one is read as an
/// interpolable string on the way past: a malformed `${…}` token is reported
/// here, and a well-formed one has its name recorded so the reference survives
/// unresolved into the IR for `build`/`serve`/`run` to check for presence.
/// *Which* keys a plugin admits is still the plugin's schema's business.
fn plugin_config(
    mapping: &Mapping,
    typed: &[&str],
    subject: &str,
    cx: &mut Cx,
) -> (Vec<ConnectionField>, Vec<PluginEntry>) {
    let mut connection = Vec::new();
    let mut extra = Vec::new();
    for entry in mapping.entries() {
        if typed.contains(&entry.key.value.as_str()) {
            continue;
        }
        let context = format!("`{}` in {subject}", entry.key.value);
        if SECRET_FIELDS.contains(&entry.key.value.as_str()) {
            let Some(value) = lexical::env_ref(&entry.value, &context, cx) else {
                continue;
            };
            connection.push(ConnectionField {
                name: entry.key.clone(),
                value,
            });
            continue;
        }
        // A plugin option is *named* by its key, not substituted into it. Class
        // 2 covers the config values, so Decision D92's totality rule leaves the
        // keys in class 3 — the same split `settings:` already draws, where the
        // key is rejected for a token as readily as the value. An unescaped one
        // here would reach the plugin as the characters the author did not
        // intend.
        lexical::reject_env_refs(&entry.key, &format!("a config key of {subject}"), cx);
        extra.push(PluginEntry {
            key: entry.key.clone(),
            value: plugin_value(&entry.value, &context, cx),
        });
    }
    (connection, extra)
}

/// Read one value of an open plugin-config object (grammar 4.3 class 2).
///
/// A plugin object carries arbitrary YAML, so the walk is recursive: a token
/// can sit inside a nested mapping or a sequence as easily as at the top, which
/// is the same reason `settings:` is walked to its leaves for the class-3 rule.
/// The number check rides along for the same reason and the one in
/// [`expect_finite`]: the deploy layer is lowered into the artifact and from
/// there into JSON (grammar 3.8), which cannot write infinity or NaN, so one
/// written here would otherwise reach the artifact as `null` — a value the
/// author never wrote.
pub(crate) fn plugin_value(node: &Node, subject: &str, cx: &mut Cx) -> Spanned<PluginValue> {
    let value = match &node.value {
        Yaml::Null => PluginValue::Null,
        Yaml::Bool(value) => PluginValue::Bool(*value),
        Yaml::Int(value) => PluginValue::Int(*value),
        Yaml::Float(value) => {
            expect_finite(*value, subject, &node.span, cx);
            PluginValue::Float(*value)
        }
        Yaml::String(text) => {
            let text = Spanned::new(text.clone(), node.span.clone());
            PluginValue::Text(lexical::interpolate(text, subject, cx).value)
        }
        Yaml::Sequence(items) => PluginValue::Sequence(
            items
                .iter()
                .map(|item| plugin_value(item, subject, cx))
                .collect(),
        ),
        Yaml::Mapping(mapping) => PluginValue::Mapping(
            mapping
                .entries()
                .iter()
                .map(|entry| {
                    lexical::reject_env_refs(
                        &entry.key,
                        &format!("a nested config key of {subject}"),
                        cx,
                    );
                    PluginEntry {
                        key: entry.key.clone(),
                        value: plugin_value(&entry.value, subject, cx),
                    }
                })
                .collect(),
        ),
    };
    Spanned::new(value, node.span.clone())
}

#[cfg(test)]
mod tests {
    use super::{PUBLIC_URL, TRACE_SINK_URL, UrlSubject, url_problem};
    use crate::diag::{Diagnostic, DiagnosticCode};
    use crate::parse::parse_str;

    /// Every diagnostic one deploy file draws.
    fn diagnose(source: &str) -> Vec<Diagnostic> {
        parse_str(source, "deploy/mesh.yml").diagnostics
    }

    /// The `missing-join-token` diagnostic, and the assertion that it is alone.
    ///
    /// Alone is half of what each case below is about: the rule's whole job is
    /// to be the one thing a reader is told about one mistake.
    #[track_caller]
    fn missing_join_token(source: &str) -> Diagnostic {
        let mut diagnostics = diagnose(source);
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["missing-join-token"],
            "this source is written to draw the requiredness rule and nothing else"
        );
        diagnostics.remove(0)
    }

    /// Where the rule anchors, and how it counts, in the shape an author who
    /// has never written a `hub:` block produces (grammar 14.2 rule 2).
    ///
    /// The negative fixture corpus pins the message, the code and the position;
    /// what it cannot pin is the **label**, and the label is the half that says
    /// which placements the rule is about. Both arms of the count are here
    /// because a file with one placement and a file with several are the two
    /// files anybody writes, and "1 placements declared here" is the kind of
    /// slip a corpus asserting only the primary message never sees.
    #[test]
    fn the_rule_anchors_on_the_document_when_no_hub_block_is_written() {
        let one = missing_join_token(
            "version: \"0.1\"\nplacements:\n  mac:\n    members: [agent.signer]\n",
        );
        assert_eq!(one.span.start.line, 1, "the whole document is the anchor");
        assert_eq!(one.span.start.column, 1);
        assert_eq!(
            one.labels
                .iter()
                .map(|label| label.message.as_str())
                .collect::<Vec<_>>(),
            ["one placement declared here"]
        );

        let several = missing_join_token(
            "version: \"0.1\"\nplacements:\n  mac:\n    members: [agent.signer]\n  gpu:\n    members: [agent.embedder]\n",
        );
        assert_eq!(several.span.start.line, 1);
        assert_eq!(
            several
                .labels
                .iter()
                .map(|label| label.message.as_str())
                .collect::<Vec<_>>(),
            ["2 placements declared here"]
        );
    }

    /// …and on the block itself when there is one to point at.
    ///
    /// The two anchors are the same choice a missing `version:` makes: the
    /// reader is sent to the line the key belongs on, and a file with no such
    /// line is sent to itself.
    #[test]
    fn the_rule_anchors_on_the_hub_block_when_one_is_written() {
        let reported = missing_join_token(
            "version: \"0.1\"\nhub:\n  public_url: \"https://hub.example\"\nplacements:\n  mac:\n    members: [agent.signer]\n",
        );
        assert_eq!(
            reported.span.start.line, 3,
            "the anchor is the `hub:` block's body, not the document"
        );
    }

    /// A `hub:` that is not a mapping is one mistake and gets one diagnostic.
    ///
    /// `hub` cannot be read, so the requiredness rule would otherwise see the
    /// same `None` a file with no `hub:` at all produces and tell its author to
    /// "declare `hub: { join_token: ${SOME_VAR} }`" — advice about a block that
    /// is already on the screen. This is the cascade
    /// [`HubSection::declares_join_token`](crate::ast::deploy::HubSection)
    /// suppresses one level in, at the level above it.
    #[test]
    fn a_hub_that_is_not_a_mapping_is_not_also_told_to_declare_one() {
        let diagnostics = diagnose(
            "version: \"0.1\"\nhub: \"https://hub.example\"\nplacements:\n  mac:\n    members: [agent.signer]\n",
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect::<Vec<_>>(),
            ["wrong-type"],
            "a malformed `hub:` draws the shape error and nothing else"
        );
    }

    /// …and so is a `join_token:` written as a literal (grammar 4.3).
    ///
    /// The sibling case, kept beside the one above because the two are one
    /// rule about cascades and drift apart the moment they are only tested
    /// apart: whatever is wrong with the token the author wrote, they are not
    /// also told they wrote none.
    #[test]
    fn a_literal_join_token_is_not_also_reported_as_a_missing_one() {
        let diagnostics = diagnose(
            "version: \"0.1\"\nhub:\n  join_token: \"s3cret\"\nplacements:\n  mac:\n    members: [agent.signer]\n",
        );
        assert_eq!(
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code)
                .collect::<Vec<_>>(),
            [DiagnosticCode::InvalidEnvRef],
            "a refused token draws the env-ref rule and nothing else"
        );
    }

    /// Every arm of the refusal, and the base that reaches it.
    ///
    /// A message nothing produces is a message nobody has read, and the reason
    /// this is a unit test rather than eight more fixtures is what the fixture
    /// corpus is for: `tests/parse_invalid.rs` pins one *rule* per file, exact
    /// message and position, and eight files differing only in a URL would be
    /// eight rules in a corpus that asserts they are distinct. The two arms an
    /// author is likeliest to hit — a relative base and a wildcard — carry
    /// fixtures there as well.
    ///
    /// Run over **both** subjects, because one shape rule serving two keys is
    /// exactly the arrangement where a clause added for one of them reads as
    /// nonsense on the other: every arm has to be a sentence about whichever key
    /// reached it.
    #[test]
    fn every_refusal_arm_answers_some_url() {
        for (subject, expected) in [
            (
                &PUBLIC_URL,
                [
                    "contains `*`, and a public base is this deployment's own URL rather than a pattern",
                    "names no scheme, and an ingress base is absolute",
                    "names the scheme `HTTPS`, and a URL derived from it is written out as text — write `https://`",
                    "names the scheme `ftp`, and a hub is reached over `http` or `https`",
                ],
            ),
            (
                &TRACE_SINK_URL,
                [
                    "contains `*`, and a sink address is this deployment's own URL rather than a pattern",
                    "names no scheme, and a sink address is absolute",
                    "names the scheme `HTTPS`, and the address is written onto the request as text — write `https://`",
                    "names the scheme `ftp`, and a trace sink is reached over `http` or `https`",
                ],
            ),
        ] {
            let [wildcard, relative, cased, scheme] = expected;
            for (url, expected) in [
                ("", "is empty"),
                ("https://hub example", "contains whitespace"),
                ("https://*.hub.example", wildcard),
                ("hub.example/ingress", relative),
                ("HTTPS://hub.example", cased),
                ("ftp://hub.example", scheme),
                ("https://", "names a scheme and nothing else"),
                (
                    "https:///ingress",
                    "names no host between `https://` and the `/` that follows it",
                ),
            ] {
                assert_eq!(
                    url_problem(url, subject).as_deref(),
                    Some(expected),
                    "`{url}` no longer reaches the arm written for it under `{}`",
                    subject.key
                );
            }
        }
    }

    /// …and the URLs an author writes are accepted, under either key.
    ///
    /// The other direction, which no negative corpus can catch: a rule written
    /// one character too tight makes a working ingress base — or a working
    /// collector address — unwritable, and nothing else would notice. A port, a
    /// path prefix, and `http` for localhost are all URLs somebody deploys.
    #[test]
    fn a_url_an_author_writes_is_accepted() {
        for subject in [&PUBLIC_URL, &TRACE_SINK_URL] {
            for url in [
                "https://hub.example",
                "https://hub.example/",
                "http://localhost:8080",
                "https://hub.example:8443/agent-compose",
                "https://hub.internal.example/ingress/v1",
                "http://localhost:4318/v1/traces",
            ] {
                assert_eq!(
                    url_problem(url, subject),
                    None,
                    "`{url}` is a URL somebody deploys and this pass refuses it under `{}`",
                    subject.key
                );
            }
        }
    }

    /// The two subjects really are two: no clause is shared between them, so no
    /// refusal reads as a sentence about the other key.
    ///
    /// Cheap, and it is the failure the parameterization invites — a fifth
    /// subject copied from a fourth and half-edited, whose messages then name
    /// the wrong construct in the two arms nobody re-read.
    #[test]
    fn each_url_subject_names_itself() {
        let subjects: [&UrlSubject; 2] = [&PUBLIC_URL, &TRACE_SINK_URL];
        for (index, subject) in subjects.iter().enumerate() {
            for other in &subjects[index + 1..] {
                for (left, right, clause) in [
                    (subject.key, other.key, "key"),
                    (subject.own, other.own, "own"),
                    (subject.absolute, other.absolute, "absolute"),
                    (subject.cased, other.cased, "cased"),
                    (subject.reached, other.reached, "reached"),
                    (subject.help, other.help, "help"),
                ] {
                    assert_ne!(
                        left, right,
                        "`{}` and `{}` share the `{clause}` clause, so one of them names the \
                         other's construct",
                        subject.key, other.key
                    );
                }
            }
        }
    }
}
