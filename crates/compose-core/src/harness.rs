//! The per-harness **connection table**: which provider connection facts cross
//! into a harness run, and the slot each one lands in (grammar 8.9, Decision
//! D143, PRD resolved q58).
//!
//! # What this table is for
//!
//! A `coder:` node's `model:` is a registry address (Decision D141), and PRD
//! resolved q58 amends what that address carries: the resolved provider's
//! **connection facts** — `base_url:`, the credential, and `headers:` — cross
//! the boundary and are mapped by each driver into the harness's own connection
//! surface. The gap the amendment closes is the deployment resolved q25 exists
//! for: a gateway binds `provider.anthropic` with a `base_url:` and every agent
//! node routes through it, while a coder node on the *same model address* talked
//! to the vendor endpoint unless its author hand-carried the gateway's URL and
//! credential in `env:`, in harness-native spellings, per node.
//!
//! # Curated, on resolved q30's terms — and hardened where q25 hardened it
//!
//! The map is a **table**, not grammar: a slot the vendor ships tomorrow is an
//! edit here rather than a new key. What q58 ruling b changes about q30's
//! two-tier posture is the second tier. A `server_tools:` entry the table cannot
//! speak for is a *warning* and travels to the wire; a connection fact a harness
//! has **no slot for** is an **error**, because a fact that changes where
//! traffic goes or whether it authenticates failing at run time instead of
//! compile time is exactly the class q25 refused (PRD G3).
//!
//! So a row that is `None` is the table's honest content. `codex`'s header row
//! is one: the pinned SDK's connection surface is an API key and a provider base
//! URL and there is nothing on it a request header could be written to — and
//! **never a faked slot**, which for that fact would have meant synthesizing a
//! whole `model_providers.*` entry out of `--config` overrides the composition
//! never wrote.
//!
//! # A slot is a slot on **one wire**
//!
//! A row does not only say *whether* a fact crosses; it says what the thing on
//! the other side is. `ANTHROPIC_BASE_URL` is an endpoint the Anthropic wire is
//! spoken to, and `CodexOptions.baseUrl` is an endpoint OpenAI's is — so a row
//! also names the `provider.*` **kinds** whose connection its slots speak for
//! ([`ConnectionRow::kinds`]). Without that half the map is keyed on the harness
//! alone, and an `openai` provider bound through a `cc` node writes an OpenAI
//! endpoint and an OpenAI key into `ANTHROPIC_BASE_URL` and `ANTHROPIC_API_KEY`
//! with nothing said. That was inert while nothing crossed; it decides where the
//! traffic goes now, which is q58 ruling b's own class, so the pairing is a
//! `validate` error (`unsupported-provider-kind`) rather than a `401` on the
//! first live call.
//!
//! It is also what keeps [`ConnectionFact`] honest. The facts are the connection
//! keys of the kinds the table speaks; the cloud-SDK kinds' credentials
//! (`access_key_id:`, `secret_access_key:`, `session_token:`,
//! `credentials_json:`) are not among them because **no row speaks `bedrock` or
//! `vertex`** — neither SDK's connection surface has a cloud credential chain on
//! it — and the pairing is refused before a fact of one is read.
//! `a_spoken_kinds_connection_keys_are_all_facts` holds the two halves together:
//! a row that grows a kind grows this enum with it, or the suite fails.
//!
//! # A fact has more than one spelling, and the row owns all of them
//!
//! A slot is the name the map **writes**. It is not the only name the thing on
//! the other side **reads**, and ruling c is about the fact rather than about
//! the name: PRD resolved q58 ruling a calls `cc`'s surface "its base-URL,
//! API-key/auth-token, and custom-header variables", plural, because the Agent
//! SDK's bundled runtime carries a whole endpoint table and a whole credential
//! list. `ANTHROPIC_AUTH_TOKEN` is read at client construction beside
//! `ANTHROPIC_API_KEY` and sent as `Authorization: Bearer …` *alongside* the
//! `X-Api-Key` the slot sets; `CLAUDE_CODE_USE_BEDROCK` and its
//! `ANTHROPIC_BEDROCK_BASE_URL` companion select a different endpoint entirely.
//!
//! So a row carries, beside each slot, the **sibling variables** that harness's
//! own runtime reads for the same fact ([`ConnectionRow::siblings`]), and
//! [`variables_read`] is what ruling c compares a node's `env:` against next to
//! [`variables_set`]. Guarding only the three names the table writes would leave
//! a node `env:` free to add a second identity and repoint the endpoint on a
//! `cc` run with no diagnostic — silent shadowing under another spelling, which
//! is the thing ruling c exists to refuse — and would leave the `connection`
//! field of the graph document and of the journal's request identity describing
//! a run that went somewhere else.
//!
//! The siblings are read under the same discipline as the slots, from the same
//! pinned release, and they are **per fact**: a provider that declares no
//! `api_key:` claims no credential name at all, which is q25's keyless posture
//! reading on the family rather than on the one variable.
//!
//! # Where each slot was read
//!
//! Every slot and every sibling below was verified against the pinned SDK's own
//! `.d.ts`, bundled source and documented contract, exactly as resolved q57
//! verified its API shapes — never recalled. [`CONNECTION`] records the release
//! each row was read against, and
//! `the_connection_table_is_audited_against_the_pinned_sdks` fails when the pin
//! moves, so a vendor's new slot arrives with the bump rather than behind it.

use crate::ast::definition::ProviderKind;
use crate::ast::flow::Harness;
use crate::ir::Ir;
use crate::ir::definition::{DefinitionBody, Model, Provider};
use crate::ir::flow::Coder;

/// One fact of a provider connection, as grammar 12.1 spells it.
///
/// Three, and the boundary is drawn twice. These are the keys of a `provider.*`
/// that say **where** the traffic goes and **how** it authenticates, on the
/// kinds [`CONNECTION`]'s rows speak for. Everything else on one of those kinds
/// is either a plugin's own vocabulary (`organization:`, an `api_version:`) or a
/// request-shaping key (`server_tools:`), and neither is a connection fact a
/// harness client has a place for — the harness opens its own connection and
/// composes its own requests.
///
/// **The cloud-SDK kinds' credentials are deliberately absent**, and their
/// absence is not a silent drop. `bedrock`'s `access_key_id:`,
/// `secret_access_key:` and `session_token:` and `vertex`'s `credentials_json:`
/// are credential keys of grammar 12.1 just as much as `api_key:` is — what they
/// are not is reachable, because neither harness row speaks `bedrock` or
/// `vertex` and [`speaks`] refuses that pairing before a fact of one is read. A
/// row that ever grows one of those kinds has to grow this enum with it, and
/// `a_spoken_kinds_connection_keys_are_all_facts` is what makes that a test
/// failure rather than an omission (PRD resolved q58 ruling b).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConnectionFact {
    /// `base_url:` — the endpoint, the one key that can say the traffic is not
    /// going to the vendor (grammar 12.1, PRD resolved q25).
    BaseUrl,
    /// `api_key:` — the credential. **Absent is absent**: q25's keyless-gateway
    /// posture is that no credential variable is injected at all, never an
    /// empty one.
    Credential,
    /// `headers:` — extra request headers, values interpolable.
    Headers,
}

impl ConnectionFact {
    /// Every fact, in the order grammar 12.1's key table lists them.
    pub const ALL: &'static [Self] = &[Self::BaseUrl, Self::Credential, Self::Headers];

    /// The key an author writes it under.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BaseUrl => "base_url",
            Self::Credential => "api_key",
            Self::Headers => "headers",
        }
    }

    /// What the fact **decides**, which is why its slot is an error rather than
    /// a warning: each of the three is a property of the run that a wrong answer
    /// does not surface until a live call.
    #[must_use]
    pub const fn decides(self) -> &'static str {
        match self {
            Self::BaseUrl => "where a connection's traffic goes",
            Self::Credential => "how a connection authenticates",
            Self::Headers => "what a connection sends with every request",
        }
    }
}

/// Where one fact lands in a harness's own connection surface.
///
/// Two shapes, because the two SDKs offer two different surfaces and a table
/// that flattened them would be describing neither:
///
///  * [`Slot::Variable`] — an **environment variable** of the process the
///    harness runs, merged into the run environment the driver already passes.
///    That is the Agent SDK's contract: its `Options.env` replaces the
///    subprocess environment outright, and the Claude Code runtime it embeds
///    reads its endpoint, its credential and its custom headers out of that
///    environment.
///  * [`Slot::Option`] — a **typed option** of the SDK's own client, which is
///    the Codex SDK's surface: `CodexOptions.baseUrl` and `CodexOptions.apiKey`
///    are fields of the constructor. One of them lands in the CLI's environment
///    on the way past — the SDK sets `CODEX_API_KEY` from `apiKey` *on top of*
///    the environment it was handed — and `injects` records that, because it is
///    what makes a node `env:` entry of the same name a collision rather than a
///    coincidence (PRD resolved q58 ruling c).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// An environment variable of the harness process.
    Variable(&'static str),
    /// A typed option of the SDK's client, and the environment variable the SDK
    /// derives from it where it derives one.
    Option {
        /// The option's name, as the SDK's `.d.ts` spells it.
        name: &'static str,
        /// The variable the SDK sets from it, where it sets one.
        injects: Option<&'static str>,
    },
}

impl Slot {
    /// How a table row reads in prose: the slot, named the way its SDK names it.
    #[must_use]
    pub fn describes(self) -> String {
        match self {
            Self::Variable(name) => format!("the `{name}` environment variable"),
            Self::Option { name, injects: _ } => format!("the SDK's `{name}` option"),
        }
    }

    /// The environment variable this slot ends up as, where it ends up as one.
    ///
    /// The half PRD resolved q58 ruling c reads: a node `env:` entry naming this
    /// variable and the connection map both write one name, and one spelling per
    /// fact is the rule.
    #[must_use]
    pub const fn variable(self) -> Option<&'static str> {
        match self {
            Self::Variable(name) => Some(name),
            Self::Option { injects, .. } => injects,
        }
    }
}

/// One harness's row: the SDK release it was read against, the provider kinds
/// its slots speak for, and a slot per fact.
pub struct ConnectionRow {
    /// The harness this row is for.
    pub harness: Harness,
    /// The SDK version the row was audited against (PRD 5.12).
    pub audited: &'static str,
    /// The `provider.*` kinds whose connection these slots carry.
    ///
    /// A slot is an endpoint and a credential **on one wire**: the Agent SDK's
    /// runtime speaks Anthropic's, the Codex CLI speaks OpenAI's, and a row that
    /// named only its variables would map either provider into either harness
    /// and say nothing. So the wire is part of the row, and a model whose
    /// provider is a kind this row does not name is refused where every other
    /// mapping failure is refused — at `validate`, not at the first live call.
    ///
    /// Read narrowly on purpose, which is the same discipline the slots are read
    /// under: `azure_openai` is not on the `codex` row because its endpoint
    /// carries a deployment path and an `api_version:` the SDK has no slot for,
    /// and the two cloud-SDK kinds are on neither row because neither SDK's
    /// connection surface has a cloud credential chain on it. A kind a vendor
    /// teaches its SDK tomorrow is an edit here, never a change to the grammar.
    pub kinds: &'static [ProviderKind],
    /// Each fact's slot, `None` where this harness's SDK tier offers none.
    pub slots: &'static [(ConnectionFact, Option<Slot>)],
    /// The **other** variables this harness's own runtime reads for a fact the
    /// row already carries — the same fact under another spelling.
    ///
    /// A slot is the name the map writes; these are the names the thing on the
    /// other side reads beside it. `ANTHROPIC_AUTH_TOKEN` is a credential the
    /// bundled runtime sends *in addition to* the `ANTHROPIC_API_KEY` the slot
    /// sets, and `CLAUDE_CODE_USE_BEDROCK` with its `ANTHROPIC_BEDROCK_BASE_URL`
    /// companion is an endpoint chosen instead of the one `ANTHROPIC_BASE_URL`
    /// names. Both are the node `env:` PRD resolved q58 ruling c refuses,
    /// spelled the way the table did not happen to write it, so both belong to
    /// the fact rather than beside it (see this module's own documentation).
    ///
    /// Each entry's fact must have a [`Slot::Variable`] on this row:
    /// `a_rows_siblings_are_other_spellings_of_a_fact_it_carries` holds that,
    /// because a sibling of a fact the harness maps as a typed option is a claim
    /// about an environment the composition does not fill.
    pub siblings: &'static [(ConnectionFact, &'static str)],
}

/// The connection table (PRD resolved q58 ruling b).
///
/// # The wire each row is a row of
///
/// `cc` carries the connection of an `anthropic` provider and no other: the
/// variables below are the Anthropic wire's, and the Agent SDK's bundled runtime
/// talks to a Claude endpoint with a Claude API key. `codex` carries `openai` and
/// `openai_compatible`, which are the two kinds grammar 12.1 spells as *an
/// OpenAI-wire endpoint with a key* — which is exactly and only what
/// `CodexOptions.baseUrl` and `CodexOptions.apiKey` are. `azure_openai` is on
/// neither: its endpoint carries a deployment path and a required `api_version:`,
/// and a row claiming it would be claiming slots the SDK does not have. Nor are
/// `bedrock` and `vertex`, whose credentials are a cloud provider's own chain and
/// whose rows take no `base_url:` at all — grammar 12.1 already sends a
/// deployment that needs a bare endpoint to `openai_compatible`.
///
/// # `cc` — `@anthropic-ai/claude-agent-sdk`
///
/// The SDK's own `Options` carry no endpoint and no credential: what they carry
/// is `env`, documented as *replacing* the subprocess environment entirely, and
/// the Claude Code runtime that subprocess runs reads its connection out of it.
/// So all three facts are variables of that environment, and the driver merges
/// them into the `env` it already builds from the node's own `env:`:
///
///  * `ANTHROPIC_BASE_URL` — the client's `baseURL`, which the bundled runtime
///    reads at construction;
///  * `ANTHROPIC_API_KEY` — the credential, read the same way beside
///    `ANTHROPIC_AUTH_TOKEN`. The **key** rather than the auth token, because
///    what grammar 12.1 spells `api_key:` is a vendor API key: an author whose
///    gateway wants a bearer token of its own writes it into `headers:`, which
///    is q25's own answer and reaches the runtime through the row below;
///  * `ANTHROPIC_CUSTOM_HEADERS` — the runtime's custom-header variable, whose
///    format is one `Name: value` per line. It is the one slot on either harness
///    that is not a one-to-one carry, and [`cc_custom_headers`] is where the
///    encoding lives so the compiler's account of it and the driver's cannot
///    disagree.
///
/// …and the **siblings** are the rest of that environment contract, read from
/// the same bundled runtime. Its endpoint table holds seven rows — one per
/// vendor-hosted wire — each a base-URL variable and, for the six that are not
/// the direct endpoint, a `CLAUDE_CODE_USE_*` selector that chooses it; its
/// **provider-selection** family holds those six selectors and a seventh,
/// `CLAUDE_CODE_USE_GATEWAY`, which has no base-URL variable of its own in this
/// bundle — the gateway path it turns on is resolved in the CLI the SDK spawns,
/// not here — and is claimed all the same, because what a selector decides is
/// *which endpoint the run talks to* and that is the question `base_url:`
/// answers; its credential list holds seven names beside the six that decide
/// whether a selected endpoint authenticates at all and the four that hand a
/// credential over on a file descriptor. Every one of them is `base_url:` or
/// `api_key:` under another spelling, which is why the row claims them for those
/// two facts rather than leaving a node `env:` free to write them.
///
/// The selection family is claimed **whole** on purpose. A row that guarded six
/// of seven selectors would be a completeness claim with a hole in it, and the
/// hole is the one shape ruling c exists to refuse: a node `env:` that repoints
/// the run while `validate` is clean and the graph document and the journal's
/// request identity go on reporting `ANTHROPIC_BASE_URL`. Where this table
/// cannot read a selector's endpoint out of the pinned bundle, the honest answer
/// is to claim the selector and say why — never to leave it out because its
/// companion could not be found.
///
/// # `codex` — `@openai/codex-sdk`
///
/// `CodexOptions` is the client's own connection surface and carries exactly two
/// of the three: `baseUrl`, which the SDK passes to the CLI as
/// `--config openai_base_url=…`, and `apiKey`, which it sets as `CODEX_API_KEY`
/// in the environment it spawns the CLI with.
///
/// **There is no header slot**, and that narrowness is the row's honest content.
/// A thread's options carry none, the client's options carry none, and the only
/// way a header could reach that CLI is a `model_providers.*` table entry
/// composed out of `--config` overrides — a provider definition this composition
/// never wrote, carrying its own name, its own endpoint and its own credential
/// variable. Writing one would be the faked slot PRD resolved q58 ruling b
/// forbids by name, so `headers:` on a provider a `codex` node's model resolves
/// through is a `validate` error instead.
///
/// It has **no siblings** either, and that narrowness is read from the same
/// documented contract: this SDK's connection surface is two typed options, and
/// the only name it puts in an environment is the `CODEX_API_KEY` the credential
/// slot already records as its `injects` (the README: *"the SDK still injects
/// its required variables (such as `CODEX_API_KEY`) on top of the environment
/// you provide"*, and *"if you set `baseUrl`, the SDK passes it as a
/// `--config openai_base_url=…` override"*). What the CLI it spawns reads out of
/// that environment on its own account is the CLI's surface, pinned as its own
/// artifact and never audited here — so this row claims nothing about it rather
/// than claiming it is empty.
pub const CONNECTION: &[ConnectionRow] = &[
    ConnectionRow {
        harness: Harness::Cc,
        audited: "0.3.272",
        kinds: &[ProviderKind::Anthropic],
        slots: &[
            (
                ConnectionFact::BaseUrl,
                Some(Slot::Variable("ANTHROPIC_BASE_URL")),
            ),
            (
                ConnectionFact::Credential,
                Some(Slot::Variable("ANTHROPIC_API_KEY")),
            ),
            (
                ConnectionFact::Headers,
                Some(Slot::Variable("ANTHROPIC_CUSTOM_HEADERS")),
            ),
        ],
        siblings: &[
            // The endpoint table's other six rows, each a base-URL variable and
            // the `CLAUDE_CODE_USE_*` selector that chooses it over
            // `ANTHROPIC_BASE_URL`, plus the companion that decides how the
            // slot's own value is treated. A node `env:` writing any of them
            // sends the run somewhere the composition's `base_url:` does not
            // name.
            (
                ConnectionFact::BaseUrl,
                "_CLAUDE_CODE_ASSUME_FIRST_PARTY_BASE_URL",
            ),
            (ConnectionFact::BaseUrl, "ANTHROPIC_BEDROCK_BASE_URL"),
            (ConnectionFact::BaseUrl, "CLAUDE_CODE_USE_BEDROCK"),
            (ConnectionFact::BaseUrl, "ANTHROPIC_VERTEX_BASE_URL"),
            (ConnectionFact::BaseUrl, "CLAUDE_CODE_USE_VERTEX"),
            (ConnectionFact::BaseUrl, "ANTHROPIC_FOUNDRY_BASE_URL"),
            (ConnectionFact::BaseUrl, "CLAUDE_CODE_USE_FOUNDRY"),
            (ConnectionFact::BaseUrl, "ANTHROPIC_AWS_BASE_URL"),
            (ConnectionFact::BaseUrl, "CLAUDE_CODE_USE_ANTHROPIC_AWS"),
            (ConnectionFact::BaseUrl, "ANTHROPIC_GOOGLE_CLOUD_BASE_URL"),
            (
                ConnectionFact::BaseUrl,
                "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
            ),
            (ConnectionFact::BaseUrl, "ANTHROPIC_BEDROCK_MANTLE_BASE_URL"),
            (ConnectionFact::BaseUrl, "CLAUDE_CODE_USE_MANTLE"),
            // …and the seventh member of that runtime's own provider-*selection*
            // family, which the six selectors above are the rest of. It is the
            // one selector with no base-URL variable beside it in this bundle:
            // the gateway path it turns on is resolved in the CLI the SDK
            // spawns, so what the bundle shows is the flag and the family it
            // belongs to. The row claims it anyway, because a selector decides
            // which endpoint the run talks to whether or not this table can read
            // that endpoint's name, and six of seven is a completeness claim
            // with exactly the hole ruling c refuses.
            (ConnectionFact::BaseUrl, "CLAUDE_CODE_USE_GATEWAY"),
            // The credential list beside `ANTHROPIC_API_KEY`. The auth token is
            // the one that matters most and reads least like a collision: the
            // bundled client reads it at construction and sends
            // `Authorization: Bearer …` *together with* the `X-Api-Key` the slot
            // set, so a gateway keying off the bearer authenticates as whoever
            // the `env:` entry names while the composition's own key rides along
            // unused.
            (ConnectionFact::Credential, "ANTHROPIC_AUTH_TOKEN"),
            (ConnectionFact::Credential, "CLAUDE_CODE_OAUTH_TOKEN"),
            (ConnectionFact::Credential, "AWS_BEARER_TOKEN_BEDROCK"),
            (ConnectionFact::Credential, "ANTHROPIC_FOUNDRY_API_KEY"),
            (ConnectionFact::Credential, "ANTHROPIC_FOUNDRY_AUTH_TOKEN"),
            (ConnectionFact::Credential, "ANTHROPIC_AWS_API_KEY"),
            // …the six that decide whether a selected endpoint authenticates at
            // all, which is the same question `api_key:` answers…
            (ConnectionFact::Credential, "CLAUDE_CODE_SKIP_BEDROCK_AUTH"),
            (ConnectionFact::Credential, "CLAUDE_CODE_SKIP_VERTEX_AUTH"),
            (ConnectionFact::Credential, "CLAUDE_CODE_SKIP_FOUNDRY_AUTH"),
            (
                ConnectionFact::Credential,
                "CLAUDE_CODE_SKIP_ANTHROPIC_AWS_AUTH",
            ),
            (
                ConnectionFact::Credential,
                "CLAUDE_CODE_SKIP_ANTHROPIC_GOOGLE_CLOUD_AUTH",
            ),
            (ConnectionFact::Credential, "CLAUDE_CODE_SKIP_MANTLE_AUTH"),
            // …and the four that hand one over on a file descriptor instead of
            // in the variable's own value.
            (
                ConnectionFact::Credential,
                "CLAUDE_CODE_OAUTH_TOKEN_FILE_DESCRIPTOR",
            ),
            (
                ConnectionFact::Credential,
                "CLAUDE_CODE_GATEWAY_TOKEN_FILE_DESCRIPTOR",
            ),
            (
                ConnectionFact::Credential,
                "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
            ),
            (
                ConnectionFact::Credential,
                "CLAUDE_CODE_WEBSOCKET_AUTH_FILE_DESCRIPTOR",
            ),
            // `headers:` has no sibling: `ANTHROPIC_CUSTOM_HEADERS` is the one
            // name that runtime reads a request header out of, and it is the
            // slot.
        ],
    },
    ConnectionRow {
        harness: Harness::Codex,
        audited: "0.154.0",
        kinds: &[ProviderKind::OpenAi, ProviderKind::OpenAiCompatible],
        slots: &[
            (
                ConnectionFact::BaseUrl,
                Some(Slot::Option {
                    name: "baseUrl",
                    injects: None,
                }),
            ),
            (
                ConnectionFact::Credential,
                Some(Slot::Option {
                    name: "apiKey",
                    injects: Some("CODEX_API_KEY"),
                }),
            ),
            // No slot. See this constant's own documentation.
            (ConnectionFact::Headers, None),
        ],
        // No siblings either, and for the same reason the header row is `None`:
        // see this constant's own documentation.
        siblings: &[],
    },
];

/// The `provider.*` kinds one harness's connection surface speaks for.
///
/// Empty for a reserved harness, which has no SDK to speak anything: `validate`
/// refuses the node before its model's provider is read, so a row would be an
/// invention rather than a table entry — the answer [`slot_of`] gives one fact
/// along.
#[must_use]
pub fn kinds_of(harness: Harness) -> &'static [ProviderKind] {
    CONNECTION
        .iter()
        .find(|row| row.harness == harness)
        .map_or(&[], |row| row.kinds)
}

/// Whether one harness's connection surface speaks one provider kind's wire
/// (PRD resolved q58 ruling b).
#[must_use]
pub fn speaks(harness: Harness, kind: ProviderKind) -> bool {
    kinds_of(harness).contains(&kind)
}

/// The slot one fact lands in under one harness, or `None` where the harness's
/// SDK tier offers none.
///
/// A reserved harness answers `None` for everything, which is the answer that
/// claims least: `validate` refuses the node before a connection fact of one is
/// read, so the row would be an invention rather than a table entry.
#[must_use]
pub fn slot_of(harness: Harness, fact: ConnectionFact) -> Option<Slot> {
    CONNECTION
        .iter()
        .find(|row| row.harness == harness)?
        .slots
        .iter()
        .find(|(held, _)| *held == fact)
        .and_then(|(_, slot)| *slot)
}

/// Every environment variable one harness's connection map would set for a
/// provider declaring `facts`, in [`ConnectionFact::ALL`] order.
///
/// The list PRD resolved q58 ruling c compares a node's `env:` against. It is
/// computed from the facts the provider **declares** rather than from the whole
/// table, which is q25's posture doing its work one surface along: a provider
/// with no `api_key:` sets no credential variable, so a node free to declare one
/// of its own is not shadowing anything.
#[must_use]
pub fn variables_set(
    harness: Harness,
    facts: &[ConnectionFact],
) -> Vec<(ConnectionFact, &'static str)> {
    ConnectionFact::ALL
        .iter()
        .copied()
        .filter(|fact| facts.contains(fact))
        .filter_map(|fact| {
            slot_of(harness, fact)
                .and_then(Slot::variable)
                .map(|name| (fact, name))
        })
        .collect()
}

/// Every environment variable one harness's own runtime reads **beside** the
/// slot, for a fact a provider declaring `facts` carries across.
///
/// The other half of the list PRD resolved q58 ruling c compares a node's `env:`
/// against. [`variables_set`] answers what the map writes; this answers what the
/// thing on the other side reads for the same fact under another name, which is
/// where a second endpoint and a second identity get in.
///
/// Computed from the **declared** facts for [`variables_set`]'s own reason: a
/// provider with no `api_key:` claims no credential name at all — not the slot
/// and not a sibling of it — so a node bound to a keyless gateway is free to
/// spell its own credential however that harness reads one.
#[must_use]
pub fn variables_read(
    harness: Harness,
    facts: &[ConnectionFact],
) -> Vec<(ConnectionFact, &'static str)> {
    let Some(row) = CONNECTION.iter().find(|row| row.harness == harness) else {
        return Vec::new();
    };
    ConnectionFact::ALL
        .iter()
        .copied()
        .filter(|fact| facts.contains(fact))
        .flat_map(|fact| {
            row.siblings
                .iter()
                .copied()
                .filter(move |(held, _)| *held == fact)
        })
        .collect()
}

/// The `ANTHROPIC_CUSTOM_HEADERS` value for a provider's headers.
///
/// The runtime splits the variable on newlines and each line on its first colon,
/// so a header is one `Name: value` line. Written here rather than only in the
/// driver because [`CONNECTION`]'s account of the slot is normative and a second
/// spelling of the encoding is a second thing to keep true.
///
/// **A line-delimited encoding is only as honest as its values**, and this
/// function does not police them because neither of the two places that can is
/// here. A value carrying a carriage return or a newline would declare one
/// header and send two — the second spelled by the value, `x-api-key` as easily
/// as anything else — so it is refused twice: `parse::binding::header_map`
/// refuses the text a composition **wrote** (`invalid-value`, at `validate`,
/// which is where PRD resolved q58 ruling b puts a fact that decides what a
/// connection sends), and the driver's own `forgesAHeaderField` refuses what a
/// `${ENV}` **resolved to**, which is text no build ever sees and exactly what a
/// gateway deployment's operator-set variables are. By the time a run reaches
/// this encoding both questions have been asked.
#[must_use]
pub fn cc_custom_headers<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    headers
        .into_iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The provider one `coder:` node's `model:` resolves to, with its address.
///
/// A **direct** binding only, exactly as [`crate::codegen`] reads it: a route at
/// this position is refused (Decision D141), and an address that resolved to
/// nothing has already been reported by the resolver. `None` therefore means
/// "there is nothing here to map", never "there is nothing to say".
#[must_use]
pub fn provider_of<'ir>(ir: &'ir Ir, coder: &Coder) -> Option<(&'ir str, &'ir Provider)> {
    let DefinitionBody::Model(Model::Direct(direct)) =
        &ir.definitions.get(&coder.model.value.to_string())?.body
    else {
        return None;
    };
    let address = direct.provider.value.to_string();
    let (address, definition) = ir.definitions.get_key_value(&address)?;
    let DefinitionBody::Provider(provider) = &definition.body else {
        return None;
    };
    Some((address.as_str(), provider))
}

/// The connection facts one provider **declares**, in [`ConnectionFact::ALL`]
/// order.
///
/// Read only where [`speaks`] already said yes: on a kind no row names, the list
/// would be empty for a `bedrock` provider holding two AWS credentials, and an
/// empty list is a claim ("nothing declared here crosses") this function must not
/// be asked to make. The caller refuses the pairing first, and
/// `a_spoken_kinds_connection_keys_are_all_facts` is what keeps the two in step.
#[must_use]
pub fn declared(provider: &Provider) -> Vec<ConnectionFact> {
    let config = &provider.config;
    ConnectionFact::ALL
        .iter()
        .copied()
        .filter(|fact| match fact {
            ConnectionFact::BaseUrl => config.base_url.is_some(),
            ConnectionFact::Credential => config.api_key.is_some(),
            ConnectionFact::Headers => !config.headers.is_empty(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::harness::version_of;

    /// Every harness this release lowers has a row, and every row names a slot
    /// or says it has none — a fact missing from a row is a fact that would
    /// quietly stop crossing.
    #[test]
    fn every_shipping_harness_has_a_complete_connection_row() {
        for harness in Harness::ALL.iter().copied().filter(|h| h.ships_in_v1()) {
            let row = CONNECTION
                .iter()
                .find(|row| row.harness == harness)
                .unwrap_or_else(|| panic!("`{}` has a connection row", harness.as_str()));
            let held: Vec<ConnectionFact> = row.slots.iter().map(|(fact, _)| *fact).collect();
            assert_eq!(
                held,
                ConnectionFact::ALL.to_vec(),
                "`{}`'s row does not answer every connection fact, in order",
                harness.as_str()
            );
        }
        for harness in [Harness::DeepAgents, Harness::Native] {
            for fact in ConnectionFact::ALL {
                assert_eq!(
                    slot_of(harness, *fact),
                    None,
                    "a reserved harness has no SDK to hold a connection fact"
                );
            }
        }
    }

    /// Every shipping harness names the wire its slots speak, and a reserved one
    /// names none (PRD resolved q58 ruling b).
    #[test]
    fn every_shipping_harness_names_the_kinds_its_slots_speak() {
        for harness in Harness::ALL.iter().copied().filter(|h| h.ships_in_v1()) {
            assert!(
                !kinds_of(harness).is_empty(),
                "`{}` maps connection facts into slots and says nothing about whose wire they \
                 are: a row with no kinds maps every provider into every harness",
                harness.as_str()
            );
        }
        for harness in [Harness::DeepAgents, Harness::Native] {
            assert!(
                kinds_of(harness).is_empty(),
                "a reserved harness has no SDK to speak a wire"
            );
        }
        // The two mispairings the table exists to refuse, named rather than
        // derived: an OpenAI connection under the Anthropic-wire harness, and an
        // Anthropic one under the OpenAI-wire harness.
        assert!(speaks(Harness::Cc, ProviderKind::Anthropic));
        assert!(!speaks(Harness::Cc, ProviderKind::OpenAi));
        assert!(speaks(Harness::Codex, ProviderKind::OpenAi));
        assert!(speaks(Harness::Codex, ProviderKind::OpenAiCompatible));
        assert!(!speaks(Harness::Codex, ProviderKind::Anthropic));
        // …and the kinds whose credentials this enum does not spell are on
        // neither row, which is what makes that narrowness safe.
        for harness in [Harness::Cc, Harness::Codex] {
            for kind in [ProviderKind::Bedrock, ProviderKind::Vertex] {
                assert!(
                    !speaks(harness, kind),
                    "`{}` claims `{}`, whose credential chain no slot here carries",
                    harness.as_str(),
                    kind.as_str()
                );
            }
        }
    }

    /// **Every connection key of a kind a row speaks is a [`ConnectionFact`]**
    /// (PRD resolved q58 ruling b).
    ///
    /// The enum is three variants, and three is only honest while the kinds the
    /// table speaks declare no fourth. `bedrock`'s `access_key_id:`,
    /// `secret_access_key:` and `session_token:` and `vertex`'s
    /// `credentials_json:` are connection keys of grammar 12.1 that this enum
    /// does not spell — safe today only because no row names those kinds, and a
    /// silent drop the moment one does: the key would be declared, cross
    /// nothing, and raise nothing, which is the failure ruling b moved back to
    /// `validate`.
    ///
    /// So the two halves are held together here. A row that grows a kind fails
    /// this test until the enum, [`declared`] and the check's own `fact_span`
    /// grow with it.
    #[test]
    fn a_spoken_kinds_connection_keys_are_all_facts() {
        use crate::ast::deploy::SECRET_FIELDS;

        let spelled: Vec<&str> = ConnectionFact::ALL
            .iter()
            .map(|fact| fact.as_str())
            .collect();
        for row in CONNECTION {
            for kind in row.kinds {
                for key in kind.keys() {
                    // A connection key is a credential key of grammar 4.3 or the
                    // header block; everything else on a kind's row is plugin
                    // vocabulary or request shaping.
                    if !SECRET_FIELDS.contains(key) && *key != "headers" {
                        continue;
                    }
                    assert!(
                        spelled.contains(key),
                        "`{}` speaks `{}`, whose row takes `{key}:` — a connection key with no \
                         `ConnectionFact` is declared, crosses nothing and raises nothing. Give \
                         it a fact (and a slot, or an honest `None`) before this row claims that \
                         kind (grammar 8.9, 12.1, Decision D143, PRD resolved q58 ruling b)",
                        row.harness.as_str(),
                        kind.as_str()
                    );
                }
            }
        }
    }

    /// **A sibling is another spelling of a fact this row already carries as a
    /// variable** (PRD resolved q58 ruling c).
    ///
    /// Three things a sibling list gets wrong on its own, each silent:
    ///
    ///  1. a sibling of a fact with **no variable slot**. `codex` maps
    ///     `base_url:` onto a typed option and fills no environment with it, so
    ///     a name claimed for that fact would refuse a node `env:` entry over a
    ///     variable nothing in the run writes — the over-rejection direction
    ///     `a_connection_that_sets_no_variable_leaves_a_nodes_env_alone` guards
    ///     one step earlier;
    ///  2. a sibling that **is** the slot, which would make one collision two;
    ///  3. one name claimed twice, which would do the same.
    #[test]
    fn a_rows_siblings_are_other_spellings_of_a_fact_it_carries() {
        for row in CONNECTION {
            let mut claimed = std::collections::BTreeSet::new();
            for (fact, variable) in row.siblings {
                let slot = slot_of(row.harness, *fact)
                    .and_then(Slot::variable)
                    .unwrap_or_else(|| {
                        panic!(
                            "`{}` claims `{variable}` for `{}:`, which it does not carry as a \
                             variable at all: a sibling of an option slot refuses a node `env:` \
                             entry over a name no part of this run writes",
                            row.harness.as_str(),
                            fact.as_str()
                        )
                    });
                assert_ne!(
                    *variable,
                    slot,
                    "`{}` lists its own `{}:` slot as a sibling of itself",
                    row.harness.as_str(),
                    fact.as_str()
                );
                assert!(
                    claimed.insert(*variable),
                    "`{}` claims `{variable}` twice, so one `env:` entry earns two diagnostics",
                    row.harness.as_str()
                );
            }
        }
    }

    /// **A declared fact claims every name that harness reads it under** (PRD
    /// resolved q58 rulings a and c).
    ///
    /// The hole this list closes, named rather than derived. Guarding only the
    /// three variables the table *writes* leaves a `cc` node's `env:` free to
    /// add `ANTHROPIC_AUTH_TOKEN` — which the bundled client sends as a bearer
    /// header *beside* the `X-Api-Key` the slot set, so a gateway reading the
    /// bearer authenticates as somebody else — and free to set
    /// `CLAUDE_CODE_USE_BEDROCK` with an `ANTHROPIC_BEDROCK_BASE_URL` of its
    /// own, which sends the run to an endpoint the composition never named while
    /// `visualize` and the journal's request identity both keep drawing the
    /// mapped one. Ruling a calls that whole environment contract `cc`'s
    /// connection surface, so all of it is inside the fact.
    #[test]
    fn a_declared_fact_claims_every_name_its_runtime_reads_it_under() {
        let credential = variables_read(Harness::Cc, &[ConnectionFact::Credential]);
        for name in [
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
            "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
            "CLAUDE_CODE_API_KEY_FILE_DESCRIPTOR",
        ] {
            assert!(
                credential.iter().any(|(_, held)| *held == name),
                "`{name}` is a credential the `cc` runtime reads beside `ANTHROPIC_API_KEY` and \
                 nothing claims it for `api_key:`"
            );
        }
        let endpoint = variables_read(Harness::Cc, &[ConnectionFact::BaseUrl]);
        for name in [
            "CLAUDE_CODE_USE_BEDROCK",
            "ANTHROPIC_BEDROCK_BASE_URL",
            "CLAUDE_CODE_USE_VERTEX",
            "ANTHROPIC_VERTEX_BASE_URL",
        ] {
            assert!(
                endpoint.iter().any(|(_, held)| *held == name),
                "`{name}` sends a `cc` run to an endpoint `ANTHROPIC_BASE_URL` does not name and \
                 nothing claims it for `base_url:`"
            );
        }
        // Each family belongs to its own fact, which is what keeps the keyless
        // posture readable: a gateway with no `api_key:` claims no credential
        // name, and a provider with no `base_url:` claims no endpoint name.
        assert!(
            !endpoint
                .iter()
                .any(|(_, held)| *held == "ANTHROPIC_AUTH_TOKEN")
        );
        assert!(
            !credential
                .iter()
                .any(|(_, held)| *held == "CLAUDE_CODE_USE_BEDROCK")
        );
        assert!(variables_read(Harness::Cc, &[]).is_empty());
        // …and the harness whose slots are typed options claims none, because
        // its row is audited over an SDK surface that fills one variable and
        // says so.
        assert!(variables_read(Harness::Codex, ConnectionFact::ALL).is_empty());
        for harness in [Harness::DeepAgents, Harness::Native] {
            assert!(variables_read(harness, ConnectionFact::ALL).is_empty());
        }
    }

    /// **The table is an audit of the pinned SDKs' connection surfaces, re-opened
    /// by the pin** (PRD resolved q58 ruling b).
    ///
    /// A slot is a claim about somebody else's release, and the release is what
    /// this repository pins. So the version each row was read against is written
    /// down beside it: a bump moves the claim's ground, and a claim whose ground
    /// moved has to be read again rather than carried forward. The same argument
    /// `a_reserved_list_is_audited_against_the_pinned_option_surface` is written
    /// under, on the other table the SDKs own.
    #[test]
    fn the_connection_table_is_audited_against_the_pinned_sdks() {
        for row in CONNECTION {
            assert_eq!(
                version_of(row.harness),
                row.audited,
                "`{}`'s SDK is pinned at {} and its connection row was audited against {}: read \
                 the release's own connection surface — its endpoint, its credential and its \
                 header contract, and every other variable it reads for one of those facts — and \
                 move this version up (grammar 8.9, Decision D143)",
                row.harness.as_str(),
                version_of(row.harness),
                row.audited
            );
        }
    }

    /// …and the half of that audit a version number cannot state: the endpoint
    /// **selection family**, written out by name (grammar 8.9, Decision D143,
    /// PRD resolved q58 ruling c).
    ///
    /// [`the_connection_table_is_audited_against_the_pinned_sdks`] says *when*
    /// to re-read the bundle; this says what a re-reading has to come back with.
    /// A selector is the sharpest thing a node `env:` can write — it repoints
    /// the run while `validate` stays clean and the graph document and the
    /// journal's request identity both go on reporting `ANTHROPIC_BASE_URL` — so
    /// the family is claimed **whole** or the claim is not a claim. It is spelled
    /// out here rather than counted, for
    /// `a_reserved_list_is_audited_against_the_pinned_option_surface`'s reason
    /// on the other table the SDKs own: a count is satisfied by any seven names,
    /// and what is being asserted is these seven.
    ///
    /// `CLAUDE_CODE_USE_GATEWAY` is the one worth naming twice. It is the only
    /// member with no base-URL variable beside it in the pinned bundle — the
    /// path it turns on is resolved in the CLI the SDK spawns — and it was left
    /// out of this row once for exactly that reason, which is the shape of hole
    /// this test exists to keep out.
    #[test]
    fn the_cc_row_claims_the_whole_endpoint_selection_family() {
        let claimed: Vec<&str> = variables_read(Harness::Cc, &[ConnectionFact::BaseUrl])
            .into_iter()
            .map(|(_, name)| name)
            .collect();
        for selector in [
            "CLAUDE_CODE_USE_BEDROCK",
            "CLAUDE_CODE_USE_VERTEX",
            "CLAUDE_CODE_USE_FOUNDRY",
            "CLAUDE_CODE_USE_ANTHROPIC_AWS",
            "CLAUDE_CODE_USE_ANTHROPIC_GOOGLE_CLOUD",
            "CLAUDE_CODE_USE_MANTLE",
            "CLAUDE_CODE_USE_GATEWAY",
        ] {
            assert!(
                claimed.contains(&selector),
                "`{selector}` is a member of the pinned runtime's provider-selection family and \
                 the `cc` row does not claim it for `base_url:`, so a coder node's `env:` may set \
                 it with no diagnostic and send the run to an endpoint the composition never \
                 named (grammar 8.9, Decision D143 rule 4)"
            );
        }
    }

    /// The one slot that is not a one-to-one carry, pinned to the format the
    /// runtime parses: one `Name: value` per line.
    #[test]
    fn custom_headers_are_one_name_value_pair_per_line() {
        assert_eq!(
            cc_custom_headers([("x-team", "platform"), ("authorization", "Bearer t")]),
            "x-team: platform\nauthorization: Bearer t"
        );
        assert_eq!(cc_custom_headers([]), "");
    }

    /// A provider declaring nothing sets nothing, which is q25's posture read as
    /// a list: an absent key is **no variable**, never an empty one.
    #[test]
    fn an_undeclared_fact_sets_no_variable() {
        assert!(variables_set(Harness::Cc, &[]).is_empty());
        assert_eq!(
            variables_set(Harness::Cc, &[ConnectionFact::BaseUrl]),
            vec![(ConnectionFact::BaseUrl, "ANTHROPIC_BASE_URL")]
        );
        // …and a slot that is an option rather than a variable contributes
        // nothing to the collision list unless the SDK derives one.
        assert!(variables_set(Harness::Codex, &[ConnectionFact::BaseUrl]).is_empty());
        assert_eq!(
            variables_set(Harness::Codex, &[ConnectionFact::Credential]),
            vec![(ConnectionFact::Credential, "CODEX_API_KEY")]
        );
    }
}
