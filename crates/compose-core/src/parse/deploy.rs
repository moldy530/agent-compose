//! The deploy layer's sections (grammar 14).

use std::collections::BTreeMap;

use crate::ast::common::Namespace;
use crate::ast::definition::StoreKind;
use crate::ast::deploy::{
    BackendAlias, BackendConfig, BackendDefault, BackendProvider, ConnectionField, EventSource,
    EventSourceKind, EventSourcesSection, HubSection, JournalProvider, JournalSection,
    PackageRegistryScope, PackageRegistrySection, Placement, PlacementsSection, PluginEntry,
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
/// Three keys take an absolute URL written out — `hub.public_url:`
/// (grammar 14.2), `trace_sink.url:` (grammar 14.5) and `package_registry`'s two
/// `url:`s (grammar 14.6) — and they are held to **one** shape rule with one arm
/// inventory, because an author meeting these surfaces should meet one set of
/// URL rules. What differs is only the noun each refusal uses for the thing
/// being written, which is what these fields carry.
///
/// The *key* is passed beside the subject rather than held on it, because one of
/// them is not a constant: a scope's URL is written at
/// `package_registry.scopes."@corp".url`, so the key names the entry the author
/// is looking at.
struct UrlSubject {
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
    /// Completes "carries a credential in its authority, and …", for a key that
    /// has somewhere else for a credential to go.
    ///
    /// `None` where the surface has no such elsewhere, and the arm is then
    /// unreachable for it: `https://user:pass@host/` is a legal ingress base and
    /// a legal collector address, and refusing one there would be this compiler
    /// inventing a rule about somebody else's deployment. A registry is the one
    /// of the three where the idiom is *documented* — Bun's own `bunfig.toml`
    /// page shows `registry = "https://username:password@registry.npmjs.org"` —
    /// and the one where following it writes a literal secret into two emitted
    /// files, the emitted `README.md`, and the artifact hash over all of them.
    userinfo: Option<&'static str>,
    /// Whether an emitted file keys something by an address **derived** from
    /// this URL rather than only writing the URL out as text.
    ///
    /// `false` for an ingress base and a collector address: both are written
    /// out verbatim and nothing is computed from either, so a query, a
    /// fragment or a host spelled a way this compiler cannot reproduce is the
    /// author's business — refusing one there would be this compiler inventing
    /// a rule about somebody else's deployment.
    ///
    /// `true` for a registry, where `.npmrc`'s credential line is keyed by the
    /// address **npm parses out of its own request** rather than by the text of
    /// the `url:` (grammar 14.6 rule 5,
    /// [`codegen::registry::npm_auth_key`](crate::codegen::registry::npm_auth_key)).
    /// A spelling that parse rewrites is a key npm never looks up, and the
    /// failure is silent in the worst direction: `npm install` goes out
    /// unauthenticated while the `bunfig.toml` from the same artifact, which
    /// carries the token inside its registry object rather than keyed by an
    /// address, authenticates. So this pass refuses every URL whose address
    /// `npm_auth_key` could not spell back, which is what lets that function
    /// document three normalizations rather than reimplement a WHATWG `URL` —
    /// and every URL the `.npmrc` **line** carrying that address could not
    /// spell back either ([`misread_npmrc_line_problem`]), which is what lets
    /// the emitter write that file unquoted.
    ///
    /// Unlike every other field here, this one is a flag rather than a clause:
    /// the arms it opens are prose about npm and `.npmrc` throughout, so a
    /// second subject that ever derived an address of its own would need its
    /// own sentences rather than a `true` here.
    derives_an_address: bool,
    /// The help every refusal of this key carries.
    help: &'static str,
}

/// `hub.public_url:` — the ingress base every URL this deployment hands out
/// derives from (grammar 14.2, PRD resolved q44 invariant 4).
const PUBLIC_URL: UrlSubject = UrlSubject {
    own: "a public base",
    absolute: "an ingress base",
    cased: "a URL derived from it is written out as text",
    reached: "a hub",
    userinfo: None,
    derives_an_address: false,
    help: "the base is an absolute URL naming a host, its scheme written lowercase and no wildcard in it — `https://hub.example`; `http` stays legal, which is what makes localhost development work (grammar 14.2, PRD resolved q44)",
};

/// `trace_sink.url:` — where every settled execution's trace is POSTed
/// (grammar 14.5, PRD resolved q50).
const TRACE_SINK_URL: UrlSubject = UrlSubject {
    own: "a sink address",
    absolute: "a sink address",
    cased: "the address is written onto the request as text",
    reached: "a trace sink",
    userinfo: None,
    derives_an_address: false,
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
    absolute_url(node, "hub.public_url", &PUBLIC_URL, cx)
}

/// `package_registry.url:` and a scope's — where the installer of a generated
/// project fetches packages from (grammar 14.6, PRD resolved q59).
///
/// One subject for both, because they are one thing at two scopes: the refusal
/// names the key it was given, and the prose is about registries either way.
const PACKAGE_REGISTRY_URL: UrlSubject = UrlSubject {
    own: "a registry address",
    absolute: "a registry address",
    cased: "the address is written into `bunfig.toml` and `.npmrc` as text",
    reached: "a registry",
    userinfo: Some(
        "a registry credential is `token:`, the `${ENV}` reference each installer expands for itself",
    ),
    derives_an_address: true,
    help: "the registry is an absolute URL naming a host, its scheme written lowercase, no wildcard in it and no credential before an `@` — `https://npm.internal.example/repo/`; `http` stays legal, which is what makes a mirror on the same network work, and the credential goes in `token:` as an `${ENV}` reference, which is what keeps it out of the two emitted files and out of the artifact hash over them. It is an address and nothing more: no query and no fragment, a host of ASCII letters, digits, `-`, `_` and `.` with an optional port, and a path spelled in characters both readers of `.npmrc` hand back — a WHATWG `URL` leaves it alone, its `.` and `..` segments written in dots rather than in `%2e`, and an ini parser does not end a line on it, so no `;` and no `=` in it. npm looks a credential up under the address it parses out of its own request, and reads that request's registry and that credential's key through those two parses, so a spelling either one rewrites is a line npm never reads (grammar 14.6, PRD resolved q59)",
};

/// Read one absolute-URL key, refusing what its [`UrlSubject`] describes.
///
/// A class-3 string (grammar 4.3, Decision D92): the value is part of what the
/// composition *is* and is shape-checked here, so a `${NAME}` token in it would
/// be a value this pass could not read. A deployment whose ingress — or whose
/// collector — differs per environment writes a different deploy file, which is
/// the layer's whole point.
fn absolute_url(
    node: &Node,
    key: &str,
    subject: &UrlSubject,
    cx: &mut Cx,
) -> Option<Spanned<String>> {
    let text = lexical::text(node, &format!("`{key}`"), cx)?;
    if let Some(problem) = url_problem(&text.value, subject) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                text.span.clone(),
                format!("{:?} is not a `{key}`: it {problem}", text.value),
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
/// [`tests::every_refusal_arm_answers_some_url`] — or, for the two groups a
/// subject opts into, by
/// [`tests::a_credential_in_the_authority_is_refused_only_where_a_token_key_exists`]
/// and [`tests::an_address_a_url_parse_respells_is_refused_only_where_one_is_derived`].
/// An arm no value reaches is a message no reader has read, and the negative
/// fixture corpus pins one rule per file rather than one arm. The wording
/// follows `callback_allow:`'s (PRD resolved q33), because an author meeting
/// these surfaces should meet one set of URL rules.
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
    // Userinfo is part of the authority, so it is read here, off the same slice
    // the host came from: an `@` in the *path* — `…/@corp/` — is a scope and not
    // a credential.
    if let Some(elsewhere) = subject.userinfo
        && host.contains('@')
    {
        return Some(format!(
            "carries a credential in its authority, and {elsewhere}"
        ));
    }
    if subject.derives_an_address {
        // Everything after the authority, which is the path plus whichever of
        // `?` and `#` may have ended it.
        let path = &rest[host.len()..];
        // Two readers stand between this text and what npm resolves, and each
        // gets its own pass: the WHATWG `URL` that derives the address, then
        // the ini parser that reads the `.npmrc` line the address is written
        // on. The address comes first because a query or a fragment is a
        // mistake about the URL rather than about the file.
        return respelled_address_problem(host, path, subject.own)
            .or_else(|| misread_npmrc_line_problem(path, subject.own));
    }
    None
}

/// Why a URL is not an address the key derived from it spells back, if it is
/// not (grammar 14.6 rule 5).
///
/// `authority` is the slice between `://` and whichever of `/`, `?` and `#`
/// ends it; `path` is everything after that slice, so it still carries a `?` or
/// a `#` when one is there — which is the first thing these arms look for.
///
/// npm never reads a registry's address out of the `registry=` line it was
/// given: it appends `/<package>` to that text, parses the result through a
/// WHATWG `URL` and then walks *up* what comes out looking for a credential. So
/// the `_authToken` key this compiler writes has to be an address on that walk,
/// spelled the way the parse spells it, and the three normalizations
/// [`codegen::registry::npm_auth_key`](crate::codegen::registry::npm_auth_key)
/// performs — the host's case, the scheme's own default port, dot segments —
/// are the whole of what it reproduces. Every other spelling the parse would
/// rewrite is refused here instead, because refusing beats emitting a line npm
/// never reads: an unmatched key fails *silently*, with `npm install` resolving
/// unauthenticated while the `bunfig.toml` from the same artifact authenticates.
///
/// What the parse rewrites, and therefore what these arms refuse:
///
/// * **a query or a fragment** — `new URL` ends the path at the `?` or the `#`,
///   and npm's `/<package>` goes after the whole text, so
///   `…/repo?group=npm` fetches `…/repo?group=npm/lodash`: the package name
///   lands in the query and the key stops at `//host/repo`;
/// * **an authority it does not spell back** — a non-ASCII host becomes
///   punycode, a percent-escape is decoded before that, a `\` ends the
///   authority the way a `/` does, a bracketed IPv6 literal is re-serialized in
///   its compressed form, and a port is renumbered (`:08443` → `:8443`, an
///   empty `:` dropped). The authority this accepts is the one this compiler
///   can spell: ASCII letters, digits, `-`, `_` and `.`, with an optional port
///   of up to five digits and no leading zero;
/// * **a numeric host** — `new URL` reads a host whose last label is a number
///   as an IPv4 address and writes it back as a dotted quad, so `010.0.0.5` is
///   `8.0.0.5` and `2130706433` is `127.0.0.1`. Only the quad it writes back is
///   accepted;
/// * **a path character it percent-encodes** — every ASCII control, everything
///   above `~`, and the six graphic characters in the WHATWG path percent-encode
///   set (`"`, `<`, `>`, `` ` ``, `{`, `}`), plus the `\` it turns into `/`. A
///   `%` in a *path* is left alone by the parse as a **character**, and so is
///   left alone here — but not as a whole segment, which is the arm below;
/// * **a `.` or `..` path segment spelled with a `%2e`** — that parse's
///   dot-segment rules are written over the escape as well as over the
///   character. A single-dot segment is `.` or an ASCII case-insensitive `%2e`;
///   a double-dot segment is `..`, `.%2e`, `%2e.` or `%2e%2e`, in any case. All
///   six resolve, and `codegen::registry::canonical_directory` resolves the two
///   written in dots alone — so `…/a/%2e%2e/repo/` would key at
///   `//host/a/%2e%2e/repo/` while npm walks up from `//host/repo/lodash`,
///   which is the silent failure again. Three dots or more is an ordinary
///   segment to that parse and so to this arm.
///
/// This is the *first* of two readers between the `url:` and what npm resolves,
/// and it is only the one that derives the address. The `.npmrc` line that
/// address is written on is read by an ini parser, which ends a line at a `;`
/// and splits one at its first `=` — two characters this parse writes back
/// untouched, and therefore two this function accepts and
/// [`misread_npmrc_line_problem`] refuses immediately after it.
fn respelled_address_problem(authority: &str, path: &str, own: &str) -> Option<String> {
    if let Some(delimiter) = path
        .chars()
        .find(|character| matches!(character, '?' | '#'))
    {
        let part = if delimiter == '?' {
            "query"
        } else {
            "fragment"
        };
        return Some(format!(
            "carries a `{delimiter}` {part}, and {own} is the text npm appends `/<package>` to \
             before parsing the result — the package name would land in the {part}"
        ));
    }
    let (name, port) = match authority.split_once(':') {
        Some((name, port)) => (name, Some(port)),
        None => (authority, None),
    };
    // Every arm below completes the same sentence, because they are one rule
    // about one derived address read at five places in the authority and one in
    // the path.
    let derived =
        format!("{own} is keyed in `.npmrc` by the address a WHATWG `URL` derives from it");
    if name.is_empty() {
        return Some(format!(
            "names no host before its `:`, and {derived} — a host is ASCII letters, digits, `-`, \
             `_` and `.`"
        ));
    }
    if let Some(character) = name
        .chars()
        .find(|character| !matches!(character, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.'))
    {
        return Some(format!(
            "carries `{}` in its authority, and {derived} — a host is ASCII letters, digits, \
             `-`, `_` and `.`",
            spelled(character)
        ));
    }
    if let Some(port) = port {
        if port.is_empty() {
            return Some(format!(
                "ends its authority in a `:` with no port, and {derived} — that parse drops an \
                 empty port rather than writing it back"
            ));
        }
        if !is_a_port_written_back_unchanged(port) {
            return Some(format!(
                "names the port `{}`, and {derived} — a port is up to five digits, no leading \
                 zero, and at most 65535",
                port.escape_debug()
            ));
        }
    }
    if is_read_as_an_ipv4_address(name) && !is_the_dotted_quad_written_back(name) {
        return Some(format!(
            "names the host `{name}`, and {derived} — a host whose last label is a number is \
             read as an IPv4 address and written back as a dotted quad"
        ));
    }
    if let Some(character) = path.chars().find(|character| {
        !character.is_ascii_graphic()
            || matches!(character, '"' | '<' | '>' | '`' | '{' | '}' | '\\')
    }) {
        return Some(format!(
            "carries `{}` in its path, and {derived} — that parse rewrites this character rather \
             than writing it back",
            spelled(character)
        ));
    }
    if let Some((segment, dots)) = path.split('/').find_map(|segment| {
        a_dot_segment_spelled_with_an_escape(segment).map(|dots| (segment, dots))
    }) {
        return Some(format!(
            "carries the path segment `{segment}`, and {derived} — that parse reads a whole \
             segment of `%2e` as a `.`, in either case, so this is a `{dots}` segment it resolves \
             rather than writing back; write `{dots}`"
        ));
    }
    None
}

/// Why a URL is not a path the `.npmrc` lines built from it spell back, if it
/// is not (grammar 14.6 rule 1).
///
/// `path` is the same slice [`respelled_address_problem`] reads, and this runs
/// after it — so a `?`, a `#` and every character a WHATWG `URL` respells are
/// already refused, and what is left is the **second** reader the registry
/// passes through. That reader is not a `URL` at all: npm parses `.npmrc` with
/// the `ini` package, and
/// [`codegen::registry`](crate::codegen::registry) writes this text into it
/// **unquoted**, twice — as the value of `registry=` and as the key of the
/// `//host/path/:_authToken=` line. Two characters a WHATWG `URL` is perfectly
/// happy to leave in a path are line syntax to that parser:
///
/// * **a `;`** ends an unquoted key *and* an unquoted value, because `ini`'s
///   `unsafe()` treats `;` and `#` as the start of a comment. A registry at
///   `…/group;maven=false/` writes `registry=https://host/group;maven=false/`,
///   which npm reads as `https://host/group` — a different path on the mirror —
///   and a credential key that stops at `//host/group`, which the walk up from
///   the request never reaches;
/// * **an `=`** ends the key alone, because `ini` splits a line at its **first**
///   `=`. A registry at `…/repo=corp/` keeps its `registry=` value whole (that
///   line's first `=` is the one after `registry`) while its credential is
///   keyed at `//host/repo`, so npm finds no `_authToken`.
///
/// `bunfig.toml` carries the same text inside a TOML basic string and hands it
/// back exactly, so either character is the one-artifact / two-answers
/// divergence grammar 14.6 rule 5 exists to prevent — reached through the file
/// *format* rather than through the address, and in the same silent direction:
/// `npm install` resolves from the wrong path or unauthenticated while
/// `bun install` from the same artifact is right. Refusing at the `url:` is
/// what lets the emitter go on writing plain ini, rather than carrying an
/// escaper whose rules would then have to match that parser's exactly.
///
/// A `#` would be the third such character; it is refused one rule earlier, as
/// the fragment a WHATWG `URL` reads it as.
fn misread_npmrc_line_problem(path: &str, own: &str) -> Option<String> {
    let character = path
        .chars()
        .find(|character| matches!(character, ';' | '='))?;
    let read = format!(
        "{own} is written into `.npmrc` unquoted, both as the `registry=` value and as the key \
         of its `_authToken` line, and npm reads that file with an ini parser"
    );
    Some(if character == ';' {
        format!(
            "carries `;` in its path, and {read} — that parser ends an unquoted key and an \
             unquoted value at a `;`, so npm would resolve from a shorter address than this one \
             and look its credential up under one shorter still"
        )
    } else {
        format!(
            "carries `=` in its path, and {read} — that parser splits a line at its first `=`, \
             so the `_authToken` key would end at this character and npm would find no \
             credential at all"
        )
    })
}

/// The `.` or `..` a WHATWG `URL` reads this path segment as, where the segment
/// spells one with a `%2e` escape rather than in dots alone.
///
/// That parse's dot-segment rules are written over the escape as well as over
/// the character: a single-dot segment is `.` or an ASCII case-insensitive
/// `%2e`, and a double-dot segment is `..` or an ASCII case-insensitive `.%2e`,
/// `%2e.` or `%2e%2e`. All six resolve on the request npm parses, while
/// `codegen::registry::canonical_directory` resolves the two written in dots
/// alone — so the four escaped spellings are refused at the `url:`, and the
/// emitter never has to decode a percent-escape to stay right.
///
/// `None` for `.` and `..` themselves, which the emitter does resolve, and for
/// three dots or more, which that parse reads as an ordinary segment
/// (`/a/.../b` and `/a/%2e%2e%2e/b` both keep their middle segment).
fn a_dot_segment_spelled_with_an_escape(segment: &str) -> Option<&'static str> {
    if matches!(segment, "." | "..") {
        return None;
    }
    let mut rest = segment;
    let mut dots = 0_usize;
    while !rest.is_empty() {
        rest = if let Some(tail) = rest.strip_prefix('.') {
            tail
        } else if rest
            .get(..3)
            .is_some_and(|head| head.eq_ignore_ascii_case("%2e"))
        {
            &rest[3..]
        } else {
            return None;
        };
        dots += 1;
        if dots > 2 {
            return None;
        }
    }
    match dots {
        1 => Some("."),
        2 => Some(".."),
        _ => None,
    }
}

/// A character as a refusal spells it: as written where it is a printable ASCII
/// character, and in Rust's escaped form otherwise, so a control byte an author
/// pasted in is named rather than carried into the message invisibly.
fn spelled(character: char) -> String {
    if character.is_ascii_graphic() {
        character.to_string()
    } else {
        character.escape_debug().to_string()
    }
}

/// Whether a WHATWG `URL` writes this port back exactly as written.
///
/// Its port parser reads the digits as a number and re-serializes it, so a
/// leading zero is dropped and an empty port disappears entirely; above 65535
/// the URL does not parse at all.
fn is_a_port_written_back_unchanged(port: &str) -> bool {
    !port.is_empty()
        && port.len() <= 5
        && port.chars().all(|digit| digit.is_ascii_digit())
        && (port.len() == 1 || !port.starts_with('0'))
        && port.parse::<u32>().is_ok_and(|number| number <= 65535)
}

/// Whether a WHATWG `URL` reads this host as an IPv4 address rather than as a
/// name: its rule is the last non-empty label being a number, written in
/// decimal, octal or hex.
fn is_read_as_an_ipv4_address(name: &str) -> bool {
    let trimmed = name.strip_suffix('.').unwrap_or(name);
    let last = trimmed.rsplit('.').next().unwrap_or(trimmed);
    match last.strip_prefix("0x").or_else(|| last.strip_prefix("0X")) {
        Some(hexadecimal) => hexadecimal.chars().all(|digit| digit.is_ascii_hexdigit()),
        None => !last.is_empty() && last.chars().all(|digit| digit.is_ascii_digit()),
    }
}

/// Whether a host is the dotted quad a WHATWG `URL` writes an IPv4 address back
/// as: four decimal parts, none of them empty or leading-zeroed, each at most
/// 255.
fn is_the_dotted_quad_written_back(name: &str) -> bool {
    let mut parts = 0;
    for label in name.split('.') {
        parts += 1;
        let canonical = !label.is_empty()
            && label.len() <= 3
            && label.chars().all(|digit| digit.is_ascii_digit())
            && (label.len() == 1 || !label.starts_with('0'))
            && label.parse::<u16>().is_ok_and(|number| number <= 255);
        if !canonical {
            return false;
        }
    }
    parts == 4
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
        .and_then(|node| absolute_url(node, "trace_sink.url", &TRACE_SINK_URL, cx));
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

/// Read the `package_registry:` section (grammar 14.6, PRD resolved q59).
///
/// A closed construct like `hub:` and `trace_sink:`: all three keys are this
/// compiler's own, and an unknown one is a mistake rather than a plugin's
/// business (Decision D50).
///
/// Nothing here reaches a running process. What it reaches is the two files
/// `build` writes beside `package.json` — `bunfig.toml` and `.npmrc` — which is
/// why the one rule that is not about a single value lives at the bottom of this
/// function: two entries that would write **one** `.npmrc` authentication line
/// with two different variables are refused, because npm's auth is keyed by
/// address and an emitted file whose last line silently won would make the two
/// installers disagree about a credential.
pub(crate) fn package_registry(node: &Node, cx: &mut Cx) -> Option<PackageRegistrySection> {
    let mapping = expect_mapping(node, "`package_registry`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "`package_registry`");
    let url = fields
        .require("url", cx)
        .and_then(|node| absolute_url(node, "package_registry.url", &PACKAGE_REGISTRY_URL, cx));
    let token = fields
        .take("token")
        .and_then(|node| lexical::env_ref(node, "`package_registry.token`", cx));
    let mut scopes = Vec::new();
    if let Some(node) = fields.take("scopes")
        && let Some(body) = expect_mapping(node, "`package_registry.scopes`", cx)
    {
        for entry in body.entries() {
            let name = Spanned::new(entry.key.value.clone(), entry.key.span.clone());
            if let Some(problem) = scope_problem(&name.value) {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        name.span.clone(),
                        format!(
                            "`{}` is not a `package_registry.scopes` key: it {problem}",
                            name.value
                        ),
                    )
                    .with_help(SCOPE_RULE),
                );
                continue;
            }
            let subject = format!("scope `{}`", name.value);
            let Some(body) = expect_mapping(&entry.value, &subject, cx) else {
                continue;
            };
            let mut fields = Fields::new(body, entry.value.span.clone(), &subject);
            let url = fields.require("url", cx).and_then(|node| {
                absolute_url(
                    node,
                    &format!("package_registry.scopes.{}.url", name.value),
                    &PACKAGE_REGISTRY_URL,
                    cx,
                )
            });
            let token = fields.take("token").and_then(|node| {
                lexical::env_ref(
                    node,
                    &format!("`package_registry.scopes.{}.token`", name.value),
                    cx,
                )
            });
            fields.finish(cx);
            scopes.push(PackageRegistryScope {
                name,
                url,
                token,
                span: entry.key.span.joined(&entry.value.span),
            });
        }
    }
    fields.finish(cx);

    let section = PackageRegistrySection {
        url,
        token,
        scopes,
        span: node.span.clone(),
    };
    one_credential_per_address(&section, cx);
    Some(section)
}

/// One `package_registry` entry, as the credential rules below read it.
struct RegistryEntry<'a> {
    /// What a refusal calls it: `package_registry`, or the scope's own key.
    name: &'a str,
    /// The `url:` it declares, which is what anchors a refusal that is about an
    /// entry rather than about a token it does not have.
    url: &'a Spanned<String>,
    token: Option<&'a Spanned<crate::ast::common::EnvRef>>,
    /// The `.npmrc` key this entry's address produces.
    address: String,
}

/// The two emitted files must authenticate the same way, and both rules that
/// can make them differ are refused here (grammar 14.6, PRD resolved q59).
///
/// npm scopes a credential to an **address** rather than to a package scope
/// (`//host/path/:_authToken=`), and it looks one up by walking **up** the
/// address of the request it is making: `//host/repo/corp/` falls back to
/// `//host/repo/`, then to `//host/`. Bun does neither — a `[install.scopes]`
/// entry carries its own `token =` or none, and nothing is keyed by an address
/// at all. So two shapes make one artifact answer differently under the two
/// installers, which is the divergence this key exists to remove:
///
/// 1. **One address, two variables** (`conflicting-registry-credential`). Two
///    entries whose `url:`s derive one address — including one registry written
///    with a trailing `/` and once without — write one `_authToken` key twice
///    and an ini parser keeps the last, while Bun's per-scope table keeps both.
/// 2. **A credential reaching an entry that declared none**
///    (`missing-registry-token`). An entry with no `token:` whose address sits
///    at or under a tokened entry's picks that token up under npm's walk-up and
///    sends nothing under Bun — the credential leaking to a registry the author
///    scoped it away from, which is the worse direction of the two.
///
/// Each gets a **code of its own** rather than an `invalid-value`, for the
/// reasons the two families they join were minted for: nothing is wrong with
/// either `url:` or either `${VAR}` read alone, so rule 1 is the
/// `conflicting-connection-variable` shape — a pair refused rather than a value
/// — and rule 2 is the `missing-credential` / `missing-callback-allowlist` /
/// `missing-join-token` shape, where a **sibling entry's** contents decide
/// whether `token:` is required and the repair is a choice of two. An
/// `invalid-value` would anchor on a perfectly legal `url:` and hand the reader
/// an `explain` page about inert and self-negating values, which describes
/// neither (PRD G3).
///
/// Both are choices the author has to make, so `validate` makes them make it.
/// Equal variables at one address are left alone: writing one line twice says
/// what writing it once said.
fn one_credential_per_address(section: &PackageRegistrySection, cx: &mut Cx) {
    let entries: Vec<RegistryEntry<'_>> = std::iter::once((
        "package_registry",
        section.url.as_ref(),
        section.token.as_ref(),
    ))
    .chain(section.scopes.iter().map(|scope| {
        (
            scope.name.value.as_str(),
            scope.url.as_ref(),
            scope.token.as_ref(),
        )
    }))
    .filter_map(|(name, url, token)| {
        let url = url?;
        Some(RegistryEntry {
            name,
            url,
            token,
            address: crate::codegen::registry::npm_auth_key(&url.value),
        })
    })
    .collect();

    // Rule 1: one key, two variables.
    let mut held: BTreeMap<&str, &RegistryEntry<'_>> = BTreeMap::new();
    for entry in &entries {
        let Some(token) = entry.token else { continue };
        let Some(first) = held.insert(entry.address.as_str(), entry) else {
            continue;
        };
        // `insert` returned the entry already holding this address, so put it
        // back: the first writer is the one a refusal points at, however many
        // entries pile onto one key.
        held.insert(entry.address.as_str(), first);
        let held_token = first
            .token
            .expect("only entries carrying a token are held here");
        if held_token.value.name == token.value.name {
            continue;
        }
        let (name, first, address) = (entry.name, first.name, &entry.address);
        cx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingRegistryCredential,
                token.span.clone(),
                format!(
                    "`{name}` and `{first}` authenticate to `{address}` with two different variables"
                ),
            )
            .with_label(
                held_token.span.clone(),
                format!("`{first}` spends `{}` there", held_token.value.name),
            )
            .with_help(
                "an `.npmrc` credential is keyed by address rather than by scope — the registry's authority and its whole path, so one written with a trailing `/` and one written without it are the same address — and two entries sharing one key write one `_authToken` line that an installer resolves last-one-wins: give each its own path on the mirror, or give both the same variable (grammar 14.6, PRD resolved q59)",
            ),
        );
    }

    // Rule 2: a credential reaching an entry that declared none. The address npm
    // would find is the **longest** tokened one the request's own address sits
    // under, which is the one a refusal has to name.
    for entry in &entries {
        if entry.token.is_some() {
            continue;
        }
        let Some(source) = entries
            .iter()
            .filter(|other| other.token.is_some() && entry.address.starts_with(&other.address))
            .max_by_key(|other| other.address.len())
        else {
            continue;
        };
        let token = source.token.expect("a source entry carries a token");
        let (name, lender, address) = (entry.name, source.name, &entry.address);
        cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingRegistryToken,
                entry.url.span.clone(),
                format!(
                    "`{name}` declares no `token:`, and npm would spend `{lender}`'s at `{address}` anyway"
                ),
            )
            .with_label(
                token.span.clone(),
                format!(
                    "`{lender}` spends `{}` at `{}`",
                    token.value.name, source.address
                ),
            )
            .with_help(
                "npm finds a credential by walking **up** the address of the request — `//host/repo/corp/` falls back to `//host/repo/` — while Bun's registry object carries its own `token =` or none, so an entry under a tokened address that declares none authenticates under one installer and not the other: give this entry the `token:` it should spend, or move it to an address that is not under the other's (grammar 14.6, PRD resolved q59)",
            ),
        );
    }
}

/// The `provider:` keywords a `journal:` chooses between (grammar 14.7).
const JOURNAL_PROVIDERS: &[(&str, JournalProvider)] = &[
    ("sqlite", JournalProvider::Sqlite),
    ("postgres", JournalProvider::Postgres),
    ("mysql", JournalProvider::Mysql),
];

/// Read the `journal:` section (grammar 14.7, PRD resolved q62).
///
/// A closed construct like `hub:`, `trace_sink:` and `package_registry:`: both
/// keys are this compiler's own, and an unknown one is a mistake rather than a
/// plugin's business (Decision D50).
///
/// It is a **single backend config** rather than `storage_backends:`' aliases
/// and per-kind defaults, and the asymmetry is the shape of the two things: a
/// store is a slot a composition names with `backend:`, so the deploy layer has
/// to answer per name, while nothing in a composition names the journal at all
/// (PRD resolved q27 — "the composition says nothing, the target binds it").
/// One target, one journal.
///
/// Two rules are decided here because each is decidable from this file alone: a
/// provider that dials out needs the `url:` it dials, and the provider that does
/// not takes no `url:` at all. Whether the block may be written *under this
/// target* is the other half, and is
/// [`resolve::target`](crate::resolve)'s — `deploy/local.yml` carries no
/// `journal:` for the reason it carries no `storage_backends:` (Decision D87).
pub(crate) fn journal(node: &Node, cx: &mut Cx) -> Option<JournalSection> {
    let mapping = expect_mapping(node, "`journal`", cx)?;
    let mut fields = Fields::new(mapping, node.span.clone(), "`journal`");
    let provider = fields
        .require("provider", cx)
        .and_then(|node| lexical::keyword(node, "`journal.provider`", JOURNAL_PROVIDERS, cx));
    let url_key = fields.span_of("url");
    let declares_url = url_key.is_some();
    let url = fields
        .take("url")
        .and_then(|node| lexical::env_ref(node, "`journal.url`", cx));
    fields.finish(cx);

    if let Some(provider) = provider.as_ref() {
        if provider.value.opens_in_process() {
            if let Some(key) = url_key {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnsupportedJournalKey,
                        key,
                        format!(
                            "`journal.url` is not a key of `provider: {}`",
                            provider.value.as_str()
                        ),
                    )
                    .with_label(
                        provider.span.clone(),
                        format!("`{}` is bound here", provider.value.as_str()),
                    )
                    .with_help(SQLITE_TAKES_NO_URL),
                );
            }
        } else if !declares_url {
            // Anchored on the `provider:` that decided it rather than on the
            // block, because that is the line the reader has to look at: the
            // block is right and the value on that line is what made a second
            // key required (the anchoring `missing-credential` takes).
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingJournalUrl,
                    provider.span.clone(),
                    format!(
                        "`journal` binds `provider: {}` and declares no `url:`",
                        provider.value.as_str()
                    ),
                )
                .with_help(remote_journal_needs_a_url(provider.value)),
            );
        }
    }

    Some(JournalSection {
        provider,
        url,
        declares_url,
        span: node.span.clone(),
    })
}

/// What an author who wrote `url:` under `provider: sqlite` is told.
const SQLITE_TAKES_NO_URL: &str = "a `sqlite` journal is one file beside the project's stores — `<project>/.agent-compose/journal.sqlite`, moved as a whole by `AGENT_COMPOSE_DATA_DIR` — so there is no address to dial and nothing for a `url:` to say: drop the key, or bind a provider that dials out (`postgres`, `mysql`) if this target's journal lives on a server (grammar 14.7, PRD resolved q62)";

/// …and what an author who bound a remote provider and wrote no `url:` is told.
fn remote_journal_needs_a_url(provider: JournalProvider) -> String {
    format!(
        "a `{}` journal is a connection rather than a file, so the deploy file names the slot and the environment holds the credential: write `url: ${{SOME_VAR}}` — an `${{ENV}}` value-form reference and never a literal, so `validate` never sees a URL — or bind `provider: sqlite`, which needs no address at all (grammar 14.7, 4.3, PRD resolved q15, q32, q62)",
        provider.as_str()
    )
}

/// What a `package_registry.scopes` key must look like, as a refusal's help.
const SCOPE_RULE: &str = "a scope is the `@…` prefix of a package name, written here exactly as a package spells it — `\"@corp\"`, the scope of `@corp/ui` — because that is how it is emitted, into `.npmrc`'s `@corp:registry=` line and Bun's `[install.scopes]` table (grammar 14.6, PRD resolved q59)";

/// Why a value is not a legal npm scope, if it is not.
///
/// Completes ``` `<key>` is not a `package_registry.scopes` key: it … ```. The
/// arms are ordered so each is the only answer to some key, and every one of
/// them is reached by [`tests::every_scope_refusal_arm_answers_some_key`].
///
/// The rule is npm's own, narrowed to what a *scope* may be: an `@`, then a
/// lowercase letter or digit, then lowercase letters, digits, `-`, `_` and `.`.
/// Uppercase is refused rather than folded, because npm lowercases package names
/// and a scope this compiler quietly rewrote would be one the author could not
/// find in either emitted file.
fn scope_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("is empty".to_string());
    }
    let Some(rest) = name.strip_prefix('@') else {
        return Some(
            "does not start with `@`, and a scope is written the way a package spells it"
                .to_string(),
        );
    };
    let mut chars = rest.chars();
    let Some(first) = chars.next() else {
        return Some("names no scope after its `@`".to_string());
    };
    if !first.is_ascii_lowercase() && !first.is_ascii_digit() {
        return Some(format!(
            "begins with `{first}` after its `@`, and a scope starts with a lowercase letter or a digit"
        ));
    }
    if let Some(bad) = chars
        .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.')))
    {
        return Some(format!(
            "contains `{bad}`, and a scope is lowercase letters, digits, `-`, `_` and `.`"
        ));
    }
    None
}

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
    use super::{
        PACKAGE_REGISTRY_URL, PUBLIC_URL, TRACE_SINK_URL, UrlSubject, scope_problem, url_problem,
    };
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
    /// Every subject this module declares, with the key a refusal of it names.
    ///
    /// The tests below run over all of them, because one shape rule serving
    /// several keys is exactly the arrangement where a clause added for one of
    /// them reads as nonsense on the others.
    const SUBJECTS: [(&str, &UrlSubject); 3] = [
        ("hub.public_url", &PUBLIC_URL),
        ("trace_sink.url", &TRACE_SINK_URL),
        ("package_registry.url", &PACKAGE_REGISTRY_URL),
    ];

    /// Run over **every** subject: every arm has to be a sentence about
    /// whichever key reached it.
    #[test]
    fn every_refusal_arm_answers_some_url() {
        for (key, subject, expected) in [
            (
                "hub.public_url",
                &PUBLIC_URL,
                [
                    "contains `*`, and a public base is this deployment's own URL rather than a pattern",
                    "names no scheme, and an ingress base is absolute",
                    "names the scheme `HTTPS`, and a URL derived from it is written out as text — write `https://`",
                    "names the scheme `ftp`, and a hub is reached over `http` or `https`",
                ],
            ),
            (
                "trace_sink.url",
                &TRACE_SINK_URL,
                [
                    "contains `*`, and a sink address is this deployment's own URL rather than a pattern",
                    "names no scheme, and a sink address is absolute",
                    "names the scheme `HTTPS`, and the address is written onto the request as text — write `https://`",
                    "names the scheme `ftp`, and a trace sink is reached over `http` or `https`",
                ],
            ),
            (
                "package_registry.url",
                &PACKAGE_REGISTRY_URL,
                [
                    "contains `*`, and a registry address is this deployment's own URL rather than a pattern",
                    "names no scheme, and a registry address is absolute",
                    "names the scheme `HTTPS`, and the address is written into `bunfig.toml` and `.npmrc` as text — write `https://`",
                    "names the scheme `ftp`, and a registry is reached over `http` or `https`",
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
                    "`{url}` no longer reaches the arm written for it under `{key}`"
                );
            }
        }
    }

    /// The one arm that is not shared: a credential in the authority, refused
    /// where the key has somewhere else to put one and accepted where it has
    /// not.
    ///
    /// Both directions are asserted from the same URL, because this is the arm
    /// whose two halves are each a real decision. On `package_registry.url` the
    /// refusal keeps a literal secret out of `.npmrc`, `bunfig.toml`, the
    /// emitted `README.md` and the artifact hash over all three — and keeps the
    /// declared `token:` from becoming dead code, since npm sends the URL's own
    /// Basic credentials and never the bearer line. On the other two it would
    /// be this compiler inventing a rule about somebody else's ingress.
    #[test]
    fn a_credential_in_the_authority_is_refused_only_where_a_token_key_exists() {
        const URL: &str = "https://deploy-user:hunter2@npm.internal.example/repository/npm-group/";
        assert_eq!(
            url_problem(URL, &PACKAGE_REGISTRY_URL).as_deref(),
            Some(
                "carries a credential in its authority, and a registry credential is `token:`, the `${ENV}` reference each installer expands for itself"
            )
        );
        for (key, subject) in [
            ("hub.public_url", &PUBLIC_URL),
            ("trace_sink.url", &TRACE_SINK_URL),
        ] {
            assert_eq!(
                url_problem(URL, subject),
                None,
                "`{key}` has no `token:` to point at, so userinfo is the author's business"
            );
        }
        // An `@` in the *path* is a package scope, and every registry URL an
        // author writes for one carries it.
        assert_eq!(
            url_problem(
                "https://npm.internal.example/repository/@corp/",
                &PACKAGE_REGISTRY_URL
            ),
            None
        );
    }

    /// The other group of arms that is not shared: a URL whose address a WHATWG
    /// `URL` spells differently from the text, refused where an emitted file
    /// keys something by that derived address (grammar 14.6 rule 5).
    ///
    /// Each row carries the key **npm actually looks up** for that registry —
    /// read off `new URL`'s own output for the request npm builds — beside the
    /// refusal, and the row asserts both halves: that the value is refused, and
    /// that
    /// [`npm_auth_key`](crate::codegen::registry::npm_auth_key) would have
    /// emitted something else. The second half is what makes this a test of a
    /// bug rather than of a rule: without the refusal, every one of these
    /// deploy files builds an artifact whose `.npmrc` carries an `_authToken`
    /// line at an address npm never visits, so `npm install` and `pnpm install`
    /// resolve unauthenticated — 401 against a private mirror — while
    /// `bun install` from the same artifact authenticates, because `bunfig.toml`
    /// carries the token inside its registry object rather than keyed by an
    /// address at all. That is the one artifact / two answers divergence
    /// grammar 14.6 rule 5 exists to prevent, and it is silent.
    ///
    /// A `None` in that column is the louder failure and the one row that is
    /// not silent: `new URL` refuses the address outright, so npm reaches no
    /// key at all and every install against that registry fails.
    ///
    /// The other two subjects are asserted to accept every row: an ingress base
    /// and a collector address are written out as text and nothing is derived
    /// from either, so a query on a collector URL is the operator's business.
    #[test]
    fn an_address_a_url_parse_respells_is_refused_only_where_one_is_derived() {
        const DERIVED: &str =
            "a registry address is keyed in `.npmrc` by the address a WHATWG `URL` derives from it";
        for (url, npm_looks_up, problem) in [
            // A query and a fragment both end the path `new URL` parses, and
            // npm appends `/<package>` to the *text*: the package name lands
            // after the delimiter and the address stops before it.
            (
                "https://npm.internal.example/repo?group=npm",
                Some("//npm.internal.example/repo"),
                "carries a `?` query, and a registry address is the text npm appends `/<package>` to before parsing the result — the package name would land in the query".to_string(),
            ),
            (
                "https://npm.internal.example/repo#mirror",
                Some("//npm.internal.example/repo"),
                "carries a `#` fragment, and a registry address is the text npm appends `/<package>` to before parsing the result — the package name would land in the fragment".to_string(),
            ),
            // A `\` is a path separator to a WHATWG `URL` parsing a special
            // scheme, so the address npm walks has a `/` where this text has a
            // `\`.
            (
                "https://npm.internal.example/a\\b/",
                Some("//npm.internal.example/a/b/"),
                format!("carries `\\` in its path, and {DERIVED} — that parse rewrites this character rather than writing it back"),
            ),
            // A non-ASCII host is punycoded; `to_ascii_lowercase` is not IDNA.
            (
                "https://npm.\u{ed}nternal.example/repo/",
                Some("//npm.xn--nternal-6ya.example/repo/"),
                format!("carries `\u{ed}` in its authority, and {DERIVED} — a host is ASCII letters, digits, `-`, `_` and `.`"),
            ),
            // A percent-escape in an authority is decoded before the host
            // parser ever sees it.
            (
                "https://ex%41mple.com/repo/",
                Some("//example.com/repo/"),
                format!("carries `%` in its authority, and {DERIVED} — a host is ASCII letters, digits, `-`, `_` and `.`"),
            ),
            // A bracketed IPv6 literal is re-serialized in its compressed form.
            (
                "https://[0:0:0:0:0:0:0:1]:4873/repo/",
                Some("//[::1]:4873/repo/"),
                format!("carries `[` in its authority, and {DERIVED} — a host is ASCII letters, digits, `-`, `_` and `.`"),
            ),
            // …and an authority with no host at all, which that parse refuses
            // outright rather than respelling.
            (
                "https://:4873/repo/",
                None,
                format!("names no host before its `:`, and {DERIVED} — a host is ASCII letters, digits, `-`, `_` and `.`"),
            ),
            // A port is a number to that parse, not a string of digits.
            (
                "https://npm.internal.example:08443/repo/",
                Some("//npm.internal.example:8443/repo/"),
                format!("names the port `08443`, and {DERIVED} — a port is up to five digits, no leading zero, and at most 65535"),
            ),
            (
                "https://npm.internal.example:/repo/",
                Some("//npm.internal.example/repo/"),
                format!("ends its authority in a `:` with no port, and {DERIVED} — that parse drops an empty port rather than writing it back"),
            ),
            // A host whose last label is a number is an IPv4 address, read in
            // whatever base it was written in and written back in decimal.
            (
                "https://010.0.0.5/repo/",
                Some("//8.0.0.5/repo/"),
                format!("names the host `010.0.0.5`, and {DERIVED} — a host whose last label is a number is read as an IPv4 address and written back as a dotted quad"),
            ),
            (
                "https://2130706433/repo/",
                Some("//127.0.0.1/repo/"),
                format!("names the host `2130706433`, and {DERIVED} — a host whose last label is a number is read as an IPv4 address and written back as a dotted quad"),
            ),
            // The WHATWG path percent-encode set, which is not the set of
            // characters a reader would guess: `|`, `^`, `[`, `'` and `~` all
            // survive a path unchanged, and these six do not. (`;` and `=`
            // survive this parse too, and are refused a rule later by the ini
            // reader — [`misread_npmrc_line_problem`], asserted below.)
            (
                "https://npm.internal.example/a{b}/",
                Some("//npm.internal.example/a%7Bb%7D/"),
                format!("carries `{{` in its path, and {DERIVED} — that parse rewrites this character rather than writing it back"),
            ),
            (
                "https://npm.internal.example/a`b/",
                Some("//npm.internal.example/a%60b/"),
                format!("carries `{}` in its path, and {DERIVED} — that parse rewrites this character rather than writing it back", '`'),
            ),
            // Everything above `~` is percent-encoded in a path, as UTF-8.
            (
                "https://npm.internal.example/caf\u{e9}/",
                Some("//npm.internal.example/caf%C3%A9/"),
                format!("carries `\u{e9}` in its path, and {DERIVED} — that parse rewrites this character rather than writing it back"),
            ),
            // …and so is every ASCII control, which a refusal names rather than
            // carrying invisibly.
            (
                "https://npm.internal.example/a\u{1}b/",
                Some("//npm.internal.example/a%01b/"),
                format!("carries `\\u{{1}}` in its path, and {DERIVED} — that parse rewrites this character rather than writing it back"),
            ),
            // A dot segment is a dot segment to that parse however it is
            // spelled: `%2e` is a `.` and `%2e%2e`, `.%2e` and `%2e.` are a
            // `..`, in either case — so the path it walks is not the path this
            // text writes, and the key stops nowhere npm visits.
            (
                "https://npm.internal.example/a/%2e%2e/repo/",
                Some("//npm.internal.example/repo/"),
                format!("carries the path segment `%2e%2e`, and {DERIVED} — that parse reads a whole segment of `%2e` as a `.`, in either case, so this is a `..` segment it resolves rather than writing back; write `..`"),
            ),
            (
                "https://npm.internal.example/a/%2E./repo/",
                Some("//npm.internal.example/repo/"),
                format!("carries the path segment `%2E.`, and {DERIVED} — that parse reads a whole segment of `%2e` as a `.`, in either case, so this is a `..` segment it resolves rather than writing back; write `..`"),
            ),
            (
                "https://npm.internal.example/a/.%2e/repo/",
                Some("//npm.internal.example/repo/"),
                format!("carries the path segment `.%2e`, and {DERIVED} — that parse reads a whole segment of `%2e` as a `.`, in either case, so this is a `..` segment it resolves rather than writing back; write `..`"),
            ),
            (
                "https://npm.internal.example/%2e/repo/",
                Some("//npm.internal.example/repo/"),
                format!("carries the path segment `%2e`, and {DERIVED} — that parse reads a whole segment of `%2e` as a `.`, in either case, so this is a `.` segment it resolves rather than writing back; write `.`"),
            ),
        ] {
            assert_eq!(
                url_problem(url, &PACKAGE_REGISTRY_URL).as_deref(),
                Some(problem.as_str()),
                "`{url}` no longer reaches the arm written for it"
            );
            if let Some(npm_looks_up) = npm_looks_up {
                assert_ne!(
                    crate::codegen::registry::npm_auth_key(url),
                    npm_looks_up,
                    "`{url}` would emit a key npm never looks up, which is why it is refused \
                     here rather than normalized there"
                );
            }
            for (key, subject) in [
                ("hub.public_url", &PUBLIC_URL),
                ("trace_sink.url", &TRACE_SINK_URL),
            ] {
                assert_eq!(
                    url_problem(url, subject),
                    None,
                    "`{key}` derives no address from its URL, so `{url}` is the author's business"
                );
            }
        }
    }

    /// …and the registry addresses an operator really writes still parse.
    ///
    /// The other direction of the rule above, which no negative corpus catches:
    /// a shape rule one character too tight makes a working mirror unwritable.
    /// Every row here is a spelling
    /// [`npm_auth_key`](crate::codegen::registry::npm_auth_key) reproduces
    /// exactly — the host's case, the scheme's own default port and dot
    /// segments are the three things it normalizes, and each is represented —
    /// so the row asserts the accepted URL **and** the key it emits, which is
    /// the address npm walks to.
    #[test]
    fn the_registry_addresses_an_operator_writes_are_accepted_and_keyed_as_npm_walks_them() {
        for (url, key) in [
            (
                "https://npm.internal.example/repository/npm-group/",
                "//npm.internal.example/repository/npm-group/",
            ),
            (
                "https://npm.internal.example/repo",
                "//npm.internal.example/repo/",
            ),
            ("http://localhost:4873/", "//localhost:4873/"),
            ("https://10.0.0.5:8443/repo/", "//10.0.0.5:8443/repo/"),
            (
                "https://npm_mirror.internal.example/repo/",
                "//npm_mirror.internal.example/repo/",
            ),
            (
                "https://npm.internal.example./repo/",
                "//npm.internal.example./repo/",
            ),
            // The three normalizations, each written the way an operator does.
            (
                "https://NPM.Internal.Example/repo/",
                "//npm.internal.example/repo/",
            ),
            (
                "https://npm.internal.example:443/repo/",
                "//npm.internal.example/repo/",
            ),
            (
                "https://npm.internal.example/a/b/../repo/",
                "//npm.internal.example/a/repo/",
            ),
            // Path characters a WHATWG `URL` leaves alone **and** an ini parser
            // hands back, including the `@` of a scope and the `%` of an escape
            // that parse does not re-encode. The `;` and the `=` that would
            // otherwise sit in this row are line syntax to the parser that
            // reads `.npmrc`, and are asserted refused in
            // [`a_path_npms_ini_parser_misreads_is_refused_only_where_a_registry_is_derived`].
            (
                "https://npm.internal.example/repository/@corp/",
                "//npm.internal.example/repository/@corp/",
            ),
            (
                "https://npm.internal.example/a|b^c'd[e]f~g/",
                "//npm.internal.example/a|b^c'd[e]f~g/",
            ),
            (
                "https://npm.internal.example/a%2Fb/",
                "//npm.internal.example/a%2Fb/",
            ),
            // A `%2e` is a dot segment to that parse only as a **whole**
            // segment, and only one or two of them: these three are ordinary
            // segments it writes back untouched, so the refusal above must not
            // reach them.
            (
                "https://npm.internal.example/a%2eb/",
                "//npm.internal.example/a%2eb/",
            ),
            (
                "https://npm.internal.example/%2e%2e%2e/repo/",
                "//npm.internal.example/%2e%2e%2e/repo/",
            ),
            (
                "https://npm.internal.example/.../repo/",
                "//npm.internal.example/.../repo/",
            ),
        ] {
            assert_eq!(
                url_problem(url, &PACKAGE_REGISTRY_URL),
                None,
                "`{url}` is a registry somebody deploys and this pass refuses it"
            );
            assert_eq!(
                crate::codegen::registry::npm_auth_key(url),
                key,
                "the key derived for `{url}`"
            );
        }
    }

    /// The **second** reader of a registry address, and the one a WHATWG `URL`
    /// says nothing about: the ini parser npm reads `.npmrc` with
    /// (grammar 14.6 rule 1).
    ///
    /// `codegen::registry` writes the address into that file unquoted, twice —
    /// as the `registry=` value and as the key of the `//host/path/:_authToken`
    /// line — and npm's `ini` ends an unquoted key and an unquoted value at a
    /// `;` and splits a line at its first `=`. Both characters survive a
    /// WHATWG `URL` path untouched, so
    /// [`an_address_a_url_parse_respells_is_refused_only_where_one_is_derived`]
    /// cannot see them: the *address* this compiler derives is right, and the
    /// *file* it writes that address into is the thing that is misread.
    ///
    /// Each row therefore asserts the refusal beside the second half that makes
    /// it a test of a bug rather than of a rule — that the emitted key really
    /// would carry the character into an `.npmrc` line — and beside what npm's
    /// own bundled `ini` hands back for that line, read off it rather than
    /// recalled:
    ///
    /// ```text
    /// registry=https://npm.internal.example/group;maven=false/
    /// //npm.internal.example/group;maven=false/:_authToken=${NPM_MIRROR_TOKEN}
    ///   → { "registry": "https://npm.internal.example/group",
    ///       "//npm.internal.example/group": "false/:_authToken=${NPM_MIRROR_TOKEN}" }
    ///
    /// registry=https://npm.internal.example/repo=corp/
    /// //npm.internal.example/repo=corp/:_authToken=${NPM_CORP_TOKEN}
    ///   → { "registry": "https://npm.internal.example/repo=corp/",
    ///       "//npm.internal.example/repo": "corp/:_authToken=${NPM_CORP_TOKEN}" }
    /// ```
    ///
    /// No `_authToken` key in either, and under the `;` not even the registry
    /// the author wrote — while `bun install` from the same artifact reads the
    /// address correctly out of `bunfig.toml`, whose TOML basic string hands
    /// every one of these characters back. That is grammar 14.6 rule 5's
    /// one-artifact / two-answers divergence, reached through the file format
    /// instead of through the address.
    ///
    /// The other two subjects accept every row, for the reason they accept a
    /// query: an ingress base and a collector address are written out as text
    /// and no file keys anything by either.
    #[test]
    fn a_path_npms_ini_parser_misreads_is_refused_only_where_a_registry_is_derived() {
        const READ: &str = "a registry address is written into `.npmrc` unquoted, both as the \
                            `registry=` value and as the key of its `_authToken` line, and npm \
                            reads that file with an ini parser";
        for (url, character, problem) in [
            (
                "https://npm.internal.example/group;maven=false/",
                ';',
                format!(
                    "carries `;` in its path, and {READ} — that parser ends an unquoted key and \
                     an unquoted value at a `;`, so npm would resolve from a shorter address \
                     than this one and look its credential up under one shorter still"
                ),
            ),
            (
                "https://npm.internal.example/repo=corp/",
                '=',
                format!(
                    "carries `=` in its path, and {READ} — that parser splits a line at its \
                     first `=`, so the `_authToken` key would end at this character and npm \
                     would find no credential at all"
                ),
            ),
            // A `;` reached before an `=` and an `=` reached before a `;`: the
            // refusal names the character it found first, so neither spelling
            // is answered with the other's sentence.
            (
                "https://npm.internal.example/a;b=c/",
                ';',
                format!(
                    "carries `;` in its path, and {READ} — that parser ends an unquoted key and \
                     an unquoted value at a `;`, so npm would resolve from a shorter address \
                     than this one and look its credential up under one shorter still"
                ),
            ),
            (
                "https://npm.internal.example/a=b;c/",
                '=',
                format!(
                    "carries `=` in its path, and {READ} — that parser splits a line at its \
                     first `=`, so the `_authToken` key would end at this character and npm \
                     would find no credential at all"
                ),
            ),
        ] {
            assert_eq!(
                url_problem(url, &PACKAGE_REGISTRY_URL).as_deref(),
                Some(problem.as_str()),
                "`{url}` no longer reaches the arm written for it"
            );
            assert!(
                crate::codegen::registry::npm_auth_key(url).contains(character),
                "`{url}` would key its credential at a line an ini parser cuts at `{character}`, \
                 which is why it is refused here rather than escaped there"
            );
            for (key, subject) in [
                ("hub.public_url", &PUBLIC_URL),
                ("trace_sink.url", &TRACE_SINK_URL),
            ] {
                assert_eq!(
                    url_problem(url, subject),
                    None,
                    "`{key}` writes no `.npmrc`, so `{url}` is the author's business"
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
        for (key, subject) in SUBJECTS {
            for url in [
                "https://hub.example",
                "https://hub.example/",
                "http://localhost:8080",
                "https://hub.example:8443/agent-compose",
                "https://hub.internal.example/ingress/v1",
                "http://localhost:4318/v1/traces",
                "https://npm.internal.example/repository/npm-group/",
            ] {
                assert_eq!(
                    url_problem(url, subject),
                    None,
                    "`{url}` is a URL somebody deploys and this pass refuses it under `{key}`"
                );
            }
        }
    }

    /// The subjects really are distinct: no clause is shared between any two of
    /// them, so no refusal reads as a sentence about another key.
    ///
    /// Cheap, and it is the failure the parameterization invites — a fourth
    /// subject copied from a third and half-edited, whose messages then name
    /// the wrong construct in the two arms nobody re-read.
    #[test]
    fn each_url_subject_names_itself() {
        for (index, (key, subject)) in SUBJECTS.iter().enumerate() {
            for (other_key, other) in &SUBJECTS[index + 1..] {
                for (left, right, clause) in [
                    (subject.own, other.own, "own"),
                    (subject.absolute, other.absolute, "absolute"),
                    (subject.cased, other.cased, "cased"),
                    (subject.reached, other.reached, "reached"),
                    (subject.help, other.help, "help"),
                ] {
                    assert_ne!(
                        left, right,
                        "`{key}` and `{other_key}` share the `{clause}` clause, so one of them \
                         names the other's construct"
                    );
                }
                // The optional clause: two subjects that both declare one are
                // held to the same rule, and `None` is not a shared clause —
                // it is an arm neither of them reaches.
                if let (Some(left), Some(right)) = (subject.userinfo, other.userinfo) {
                    assert_ne!(
                        left, right,
                        "`{key}` and `{other_key}` share the `userinfo` clause, so one of them \
                         names the other's construct"
                    );
                }
            }
        }
    }

    /// Every arm of the scope-key refusal, and the key that reaches it.
    ///
    /// The same discipline [`every_refusal_arm_answers_some_url`] holds the URL
    /// rule to: an arm no value reaches is a message no reader has read, and the
    /// fixture corpus pins one rule per file rather than one arm.
    #[test]
    fn every_scope_refusal_arm_answers_some_key() {
        for (key, expected) in [
            ("", "is empty"),
            (
                "corp",
                "does not start with `@`, and a scope is written the way a package spells it",
            ),
            ("@", "names no scope after its `@`"),
            (
                "@-corp",
                "begins with `-` after its `@`, and a scope starts with a lowercase letter or a digit",
            ),
            (
                "@Corp",
                "begins with `C` after its `@`, and a scope starts with a lowercase letter or a digit",
            ),
            (
                "@corp/ui",
                "contains `/`, and a scope is lowercase letters, digits, `-`, `_` and `.`",
            ),
        ] {
            assert_eq!(
                scope_problem(key).as_deref(),
                Some(expected),
                "`{key}` no longer reaches the arm written for it"
            );
        }
    }

    /// …and the scopes an author writes are accepted.
    #[test]
    fn a_scope_an_author_writes_is_accepted() {
        for key in ["@corp", "@my-org", "@acme.co", "@a", "@org_2"] {
            assert_eq!(
                scope_problem(key),
                None,
                "`{key}` is a scope somebody publishes under and this pass refuses it"
            );
        }
    }

    /// One `package_registry:` with a mirror, a credential and one scope.
    fn mirror(scope: &str) -> String {
        format!(
            "version: \"0.1\"\n\
             package_registry:\n  \
               url: \"https://npm.internal.example/repo/\"\n  \
               token: ${{NPM_MIRROR_TOKEN}}\n  \
               scopes:\n    \
                 \"@corp\":\n{scope}"
        )
    }

    /// An entry with no `token:` of its own, at an address npm's walk-up reaches
    /// the tokened one from.
    ///
    /// This is the direction that **leaks** rather than the one that 401s: npm
    /// looks a credential up by walking up the address of its request, finds the
    /// parent's key and spends the credential at a registry the author scoped it
    /// away from, while Bun's entry for that scope carries nothing and sends
    /// nothing. Both shapes the walk-up reaches are here — the same address, and
    /// a subpath of it — and so is the label, because the message names the
    /// entry's *own* address and only the label says where the credential it
    /// would pick up actually sits.
    #[test]
    fn an_entry_under_a_tokened_address_may_not_declare_no_credential() {
        for (shape, url, address) in [
            (
                "the same address",
                "https://npm.internal.example/repo/",
                "//npm.internal.example/repo/",
            ),
            (
                "a subpath of it",
                "https://npm.internal.example/repo/corp/",
                "//npm.internal.example/repo/corp/",
            ),
        ] {
            let diagnostics = diagnose(&mirror(&format!("      url: \"{url}\"\n")));
            assert_eq!(
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.clone())
                    .collect::<Vec<_>>(),
                [format!(
                    "`@corp` declares no `token:`, and npm would spend `package_registry`'s at `{address}` anyway"
                )],
                "{shape}"
            );
            // The code is the half a message assertion cannot carry, and it is
            // what routes the reader to an `explain` page about this rule
            // rather than to `invalid-value`'s (PRD G3).
            assert_eq!(
                diagnostics[0].code,
                DiagnosticCode::MissingRegistryToken,
                "{shape}"
            );
            assert_eq!(
                diagnostics[0]
                    .labels
                    .iter()
                    .map(|label| label.message.as_str())
                    .collect::<Vec<_>>(),
                ["`package_registry` spends `NPM_MIRROR_TOKEN` at `//npm.internal.example/repo/`"],
                "{shape}"
            );
        }
    }

    /// …and the arrangements an operator actually writes stay legal.
    ///
    /// The other direction of both credential rules, which no negative corpus
    /// reaches: a rule written one relation too wide makes the ordinary
    /// corporate shape — a group mirror and a scope repository beside it —
    /// unwritable, and nothing else would notice.
    #[test]
    fn a_registry_layout_an_operator_writes_is_accepted() {
        for (shape, deploy) in [
            (
                "a scope beside the default registry rather than under it",
                mirror(
                    "      url: \"https://npm.internal.example/corp/\"\n      token: ${NPM_CORP_TOKEN}\n",
                ),
            ),
            (
                "one variable at one address, written twice",
                mirror(
                    "      url: \"https://npm.internal.example/repo/\"\n      token: ${NPM_MIRROR_TOKEN}\n",
                ),
            ),
            (
                "a scope under the default registry with a credential of its own",
                mirror(
                    "      url: \"https://npm.internal.example/repo/corp/\"\n      token: ${NPM_CORP_TOKEN}\n",
                ),
            ),
            (
                "a mirror nobody authenticates to",
                "version: \"0.1\"\npackage_registry:\n  url: \"https://npm.internal.example/repo/\"\n  scopes:\n    \"@corp\":\n      url: \"https://npm.internal.example/repo/corp/\"\n".to_string(),
            ),
            (
                "a credential below the address the default registry reads from",
                "version: \"0.1\"\npackage_registry:\n  url: \"https://npm.internal.example/\"\n  scopes:\n    \"@corp\":\n      url: \"https://npm.internal.example/corp/\"\n      token: ${NPM_CORP_TOKEN}\n".to_string(),
            ),
        ] {
            let diagnostics = diagnose(&deploy);
            assert_eq!(
                diagnostics
                    .iter()
                    .map(|diagnostic| diagnostic.message.clone())
                    .collect::<Vec<_>>(),
                Vec::<String>::new(),
                "{shape} is a layout somebody deploys and this pass refuses it"
            );
        }
    }
}
