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
//! `credentials_json:`) and `azure_openai`'s required `api_version:` are not
//! among them because **no row speaks `bedrock`, `vertex` or `azure_openai`** —
//! neither SDK's connection surface has a cloud credential chain on it, and
//! neither takes an API version beside an endpoint — and the pairing is refused
//! before a fact of one is read. `a_spoken_kinds_connection_keys_are_all_facts`
//! holds the two halves together: a row that grows a kind grows this enum with
//! it, or the suite fails. That guard reads a **denylist** of the keys which
//! decide nothing about a connection, because `azure_openai`'s other three keys
//! are facts already — so a check phrased the other way round, as a list of
//! connection-looking names, lets that one kind through with its `api_version:`
//! declared, crossing nothing and raising nothing.
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
//! **`codex` is the same shape and it took a round to see it.** Its slots are
//! typed options rather than variables, which reads like a narrower surface —
//! but one of them *becomes* a variable on the way past (the SDK sets
//! `CODEX_API_KEY` from `apiKey`), and the program that reads it is the CLI the
//! SDK spawns. That CLI is not out of reach: the SDK's own `package.json` pins
//! it (`"dependencies": { "@openai/codex": "0.154.0" }`), so it is exactly as
//! pinned as the `.d.ts`, and it supports **three** auth variables rather than
//! one. `cc`'s own siblings were read out of the runtime its SDK spawns, not out
//! of `Options`, so reading `codex`'s narrowly off `CodexOptions` was one audit
//! done two ways.
//!
//! So a row carries, beside each slot, the **sibling variables** that harness's
//! own runtime reads for the same fact ([`ConnectionRow::siblings`]), and
//! [`variables_read`] is what ruling c compares a node's `env:` against next to
//! [`variables_set`]. Guarding only the names the table writes would leave a
//! node `env:` free to add a second identity and repoint the endpoint on a `cc`
//! run — or hand a `codex` run an `OPENAI_API_KEY` beside the composition's own
//! key — with no diagnostic: silent shadowing under another spelling, which is
//! the thing ruling c exists to refuse, and which would leave the `connection`
//! field of the graph document and of the journal's request identity describing
//! a run that went somewhere else.
//!
//! The siblings are read under the same discipline as the slots, from the same
//! pinned release — including the release a pinned SDK pins in turn — and they
//! are **per fact**: a provider that declares no `api_key:` claims no credential
//! name at all, which is q25's keyless posture reading on the family rather than
//! on the one variable.
//!
//! # Where each slot was read
//!
//! Every slot and every sibling below was verified against the pinned SDK's own
//! `.d.ts`, bundled source and documented contract, exactly as resolved q57
//! verified its API shapes — never recalled. [`CONNECTION`] records the release
//! each row was read against, and
//! `the_connection_table_is_audited_against_the_pinned_sdks` fails when the pin
//! moves, so a vendor's new slot arrives with the bump rather than behind it.
//!
//! # Two more tables, on the same terms (PRD resolved q60)
//!
//! [`CONNECTION`] is the pattern, and two further tables copy it because they are
//! the same kind of claim about the same pinned releases:
//!
//!  * [`PERMISSION`] — which harness has an **approval mode** axis at all, what
//!    its modes are, which of them each `access:` level admits, and the mode a
//!    level derives when `permission_mode:` is absent (grammar 8.9, Decision
//!    D146, PRD resolved q60 ruling a);
//!  * [`RESERVED`] — the options of each harness's own SDK surface that the
//!    generated adapter owns, and the first-class key that states each of them
//!    where one does. A `settings:` key spelling one is a `validate` **error**
//!    since PRD resolved q60 ruling b, and the answer beside each row is what
//!    makes that refusal a repair rather than a wall.
//!
//! Each records the release it was audited against, and each has an audit test
//! that fails when the pin moves.

use crate::ast::definition::ProviderKind;
use crate::ast::flow::{Harness, PermissionMode, WorkspaceAccess};
use crate::ir::Ir;
use crate::ir::definition::{DefinitionBody, Model, Provider};
use crate::ir::flow::Coder;

/// One fact of a provider connection, as grammar 12.1 spells it.
///
/// Three, and the boundary is drawn twice. These are the keys of a `provider.*`
/// that say **where** the traffic goes and **how** it authenticates, on the
/// kinds [`CONNECTION`]'s rows speak for. Everything else on one of those kinds
/// is either a plugin's own vocabulary (`organization:`) or a request-shaping
/// key (`server_tools:`), and neither is a connection fact a harness client has
/// a place for — the harness opens its own connection and composes its own
/// requests.
///
/// **The keys of the kinds no row speaks are deliberately absent**, and their
/// absence is not a silent drop. `bedrock`'s `access_key_id:`,
/// `secret_access_key:` and `session_token:`, `vertex`'s `credentials_json:` and
/// `azure_openai`'s **required** `api_version:` are connection keys of grammar
/// 12.1 just as much as `api_key:` is — what they are not is reachable, because
/// no harness row speaks those three kinds and [`speaks`] refuses that pairing
/// before a fact of one is read. A row that ever grows one of those kinds has to
/// grow this enum with it, and `a_spoken_kinds_connection_keys_are_all_facts` is
/// what makes that a test failure rather than an omission (PRD resolved q58
/// ruling b). `azure_openai` is the one worth naming twice: its other three keys
/// are facts already, so it is the kind a guard written as an allowlist of
/// connection-looking names waves straight through.
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
    /// names; `OPENAI_API_KEY` and `CODEX_ACCESS_TOKEN` are the other two auth
    /// variables the CLI the Codex SDK spawns supports beside the
    /// `CODEX_API_KEY` that SDK injects from `apiKey`. Each is the node `env:`
    /// PRD resolved q58 ruling c refuses, spelled the way the table did not
    /// happen to write it, so each belongs to the fact rather than beside it
    /// (see this module's own documentation).
    ///
    /// Each entry's fact must **reach the run's environment** on this row —
    /// either as a [`Slot::Variable`] or as a [`Slot::Option`] the SDK
    /// `injects` one from, which is how `codex` carries its credential:
    /// `a_rows_siblings_are_other_spellings_of_a_fact_it_carries` holds that,
    /// because a sibling of a fact that fills no variable at all is a claim
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
///    that is not a one-to-one carry, so this row names the *variable* and the
///    encoding has exactly one spelling — the `cc` driver's own `ccConnection`,
///    which `generated_code_gates`'
///    `a_harness_run_is_contained_journaled_and_recorded` executes and pins to
///    `"x-team: platform\nx-run: batch"`. A second copy here would be a second
///    thing to keep true, and nothing would notice the day the two parted
///    company.
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
/// It **does** have siblings, on one fact, and the audit that missed them the
/// first time is worth stating because the mistake was structural rather than
/// clerical. This SDK's connection surface is two typed options — but one of
/// them becomes a variable on the way past (the README: *"the SDK still injects
/// its required variables (such as `CODEX_API_KEY`) on top of the environment
/// you provide"*, and the bundle: `if (args.apiKey) { env.CODEX_API_KEY =
/// args.apiKey; }`), and the program that reads that variable is the CLI this
/// SDK spawns. Calling the CLI "somebody else's artifact" was the error:
/// `@openai/codex-sdk`'s own `package.json` pins it — `"dependencies": {
/// "@openai/codex": "0.154.0" }` — so it is exactly as pinned as the `.d.ts`
/// above it, and every sibling on the `cc` row was itself read out of the
/// runtime *that* SDK spawns rather than out of `Options`.
///
/// Read against that pinned binary, `api_key:` has **three** names and not one.
/// Its own string table carries `auth.json OPENAI_API_KEY CODEX_API_KEY
/// CODEX_ACCESS_TOKEN` beside *"Run codex login or provide an API key through a
/// supported auth env var."*, *"auth is provided by environment"* and *"auth is
/// configured, but multiple auth env vars are present"* — all three are
/// supported auth env vars, and the CLI itself calls the multi-source case an
/// ambiguity. `CODEX_API_KEY` is the slot's own `injects`; `OPENAI_API_KEY` and
/// `CODEX_ACCESS_TOKEN` are `api_key:` under another spelling, so the row claims
/// them rather than leaving a node `env:` free to authenticate the run as
/// somebody else while the graph document and the journal's request identity go
/// on reporting the mapped key.
///
/// `base_url:` claims **nothing** beside its option, and that is what the same
/// reading came back with rather than a gap left in it. The pinned binary's
/// provider table holds `https://api.openai.com/v1` as the built-in endpoint
/// beside `env_key`, `OPENAI_ORGANIZATION` and the `CODEX_OSS_BASE_URL` of the
/// other built-in provider, and no `OPENAI_BASE_URL` with them: the only
/// occurrence of that name in the whole binary sits in its network proxy's
/// secret-redaction list. What the SDK writes from `baseUrl` is the
/// `--config openai_base_url=…` override, which is a config key and not a
/// variable — so there is no environment name a node `env:` could repoint that
/// fact with, which is also why a fact whose slot fills no variable carries no
/// sibling. `headers:` has no slot at all, so it has nothing to be a second
/// spelling of.
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
        siblings: &[
            // The other two auth variables the CLI this SDK pins and spawns
            // supports, beside the `CODEX_API_KEY` the credential slot injects.
            // The binary's own string table names all three together —
            // `auth.json OPENAI_API_KEY CODEX_API_KEY CODEX_ACCESS_TOKEN`,
            // beside "Run codex login or provide an API key through a supported
            // auth env var." and "auth is configured, but multiple auth env vars
            // are present" — so a node `env:` writing either of these hands the
            // run a second identity the composition never named, and the CLI
            // itself treats the multi-source case as an ambiguity.
            (ConnectionFact::Credential, "OPENAI_API_KEY"),
            (ConnectionFact::Credential, "CODEX_ACCESS_TOKEN"),
            // `base_url:` has none: the SDK writes it as a `--config
            // openai_base_url=…` override rather than into the environment, and
            // the pinned binary's provider table carries no endpoint variable
            // beside its built-in `https://api.openai.com/v1`. `headers:` has no
            // slot to be a second spelling of. See this constant's own
            // documentation.
        ],
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

// ---------------------------------------------------------------------------
// The permission table (PRD resolved q60 ruling a)
// ---------------------------------------------------------------------------

/// What one `access:` level admits on one harness's approval axis.
///
/// Two answers per level, and they are different questions. `derived` is what
/// the level means **on its own** — the mapping q57 shipped, which an absent
/// `permission_mode:` still means exactly. `admits` is the **widening bound**:
/// the modes this level may be moved to, which is what makes the key a choice
/// inside a containment statement rather than a way around one.
pub struct PermissionLevel {
    /// The `access:` level this row answers for.
    pub access: WorkspaceAccess,
    /// The mode the level derives when the node states none.
    ///
    /// It is always a member of [`admits`](Self::admits) —
    /// `a_levels_derived_mode_is_one_it_admits` holds that, because a default
    /// the key could not be written out longhand would make the absent form mean
    /// something the present form cannot say.
    pub derived: PermissionMode,
    /// Every mode this level admits, in [`PermissionMode::ALL`] order.
    pub admits: &'static [PermissionMode],
}

/// One harness's approval-mode row: the SDK release it was read against, and a
/// [`PermissionLevel`] per `access:` level.
///
/// **A harness with no row has no such axis**, which is `codex`: its containment
/// primitive is a sandbox preset and its per-call approval tier lives in an app
/// server this project does not adopt (PRD resolved q57 ruling c), so a
/// `permission_mode:` on a node bound to it is refused rather than mapped onto
/// something that would not hold. An empty row would have been a faked slot in
/// the sense Decision D143 forbids by name.
pub struct PermissionRow {
    /// The harness this row is for.
    pub harness: Harness,
    /// The SDK version the modes were read out of (PRD 5.12).
    pub audited: &'static str,
    /// Each `access:` level's derived mode and admitted set, in
    /// [`WorkspaceAccess::ALL`] order.
    pub levels: &'static [PermissionLevel],
}

/// The approval-mode table (PRD resolved q60 ruling a).
///
/// # Where the modes were read
///
/// `@anthropic-ai/claude-agent-sdk@0.3.272`, `sdk.d.ts`:
///
/// ```text
/// export declare type PermissionMode = 'default' | 'acceptEdits'
///   | 'bypassPermissions' | 'plan' | 'dontAsk' | 'auto';
/// ```
///
/// …with the release's own sentence for each, from the doc comment over that
/// declaration: *"'default' - Standard behavior, prompts for dangerous
/// operations. 'acceptEdits' - Auto-accept file edit operations.
/// 'bypassPermissions' - Bypass all permission checks (requires
/// allowDangerouslySkipPermissions). 'plan' - Planning mode, no actual tool
/// execution. 'dontAsk' - Don't prompt for permissions, deny if not
/// pre-approved. 'auto' - Use a model classifier to approve/deny permission
/// prompts."* [`PermissionMode::decides`] is that list, one sentence per member,
/// and `the_permission_table_is_audited_against_the_pinned_sdk` is what re-opens
/// the reading when the pin moves.
///
/// # Why there is a `cc` row and no `codex` row
///
/// The gap PRD resolved q60 comes from: `access:` is codex-shaped — it maps
/// one-to-one onto their three sandbox presets (Decision D138) — and the Agent
/// SDK carries a second axis beside containment that those three flattened, so
/// three of its six modes were unreachable by design rather than by intent.
/// `codex` has nothing on the other side of that map: its own approval axis
/// (`approvalPolicy`) is the app-server tier resolved q57 ruling c states this
/// release does not adopt, and a run that escalated out of its sandbox to an
/// approver nothing answers for would be `access:` saying one thing and the run
/// doing another. So the key is **refused** on that harness, naming the
/// asymmetry, rather than mapped onto a policy this project does not drive.
///
/// # The widening bound, level by level
///
/// The invariant is that a mode may never grant an operation the level's own
/// derived mode would refuse, and the concrete sets are the ruling's:
///
///  * **`read_only` admits `plan` alone.** The level's sentence is "read the
///    workspace, write nothing", and `plan` is the one mode of the six that
///    executes no tool at all. Every other mode would let the loop reach a
///    writing tool under a level that says it may not;
///  * **`workspace_write` admits `acceptEdits`, `auto`, `default` and
///    `dontAsk`.** Each of the four still runs the SDK's permission machinery —
///    they differ in *who answers a prompt*, which is the axis — so the
///    containment `workspace_write` states is the same under all four;
///  * **`full_access` admits all six**, which is the level that asks for no
///    containment: there is nothing left for a mode to widen.
///
/// `bypassPermissions` is the member worth naming twice: it turns the permission
/// machinery **off**, so it is admitted at the one level whose own derived mode
/// already is it, and refused everywhere else.
pub const PERMISSION: &[PermissionRow] = &[PermissionRow {
    harness: Harness::Cc,
    audited: "0.3.272",
    levels: &[
        PermissionLevel {
            access: WorkspaceAccess::ReadOnly,
            derived: PermissionMode::Plan,
            admits: &[PermissionMode::Plan],
        },
        PermissionLevel {
            access: WorkspaceAccess::WorkspaceWrite,
            derived: PermissionMode::AcceptEdits,
            admits: &[
                PermissionMode::Default,
                PermissionMode::AcceptEdits,
                PermissionMode::DontAsk,
                PermissionMode::Auto,
            ],
        },
        PermissionLevel {
            access: WorkspaceAccess::FullAccess,
            derived: PermissionMode::BypassPermissions,
            admits: PermissionMode::ALL,
        },
    ],
}];

/// Whether one harness carries an approval-mode axis at all.
#[must_use]
pub fn has_permission_axis(harness: Harness) -> bool {
    PERMISSION.iter().any(|row| row.harness == harness)
}

/// Every harness that carries one, in [`Harness::ALL`] order — what a refusal
/// names as the place the key does belong.
#[must_use]
pub fn harnesses_with_a_permission_axis() -> Vec<Harness> {
    Harness::ALL
        .iter()
        .copied()
        .filter(|harness| has_permission_axis(*harness))
        .collect()
}

/// One `access:` level's row under one harness, or `None` where the harness has
/// no approval axis.
#[must_use]
pub fn permission_level(
    harness: Harness,
    access: WorkspaceAccess,
) -> Option<&'static PermissionLevel> {
    PERMISSION
        .iter()
        .find(|row| row.harness == harness)?
        .levels
        .iter()
        .find(|level| level.access == access)
}

/// The modes one `access:` level admits under one harness, in
/// [`PermissionMode::ALL`] order. Empty where the harness has no axis.
#[must_use]
pub fn admitted(harness: Harness, access: WorkspaceAccess) -> &'static [PermissionMode] {
    permission_level(harness, access).map_or(&[], |level| level.admits)
}

/// The mode one node **runs under**: the one it states, or the one its `access:`
/// derives (PRD resolved q60 ruling a).
///
/// `None` where the bound harness has no approval axis, which is the answer that
/// claims least: `codex` runs under a sandbox preset and there is no mode to
/// report. A stated mode outside the level's admitted set has already been
/// refused, so this function never reports one — it answers what the composition
/// wrote, and `validate` is what decided the composition may say it.
#[must_use]
pub fn resolved_mode(coder: &Coder) -> Option<PermissionMode> {
    let harness = coder.harness.value;
    if !has_permission_axis(harness) {
        return None;
    }
    if let Some(stated) = &coder.permission_mode {
        return Some(stated.value);
    }
    permission_level(
        harness,
        coder.access.unwrap_or(WorkspaceAccess::WorkspaceWrite),
    )
    .map(|level| level.derived)
}

// ---------------------------------------------------------------------------
// The reserved-option table (PRD resolved q60 ruling b)
// ---------------------------------------------------------------------------

/// What states, in the composition, the bound one reserved SDK option carries.
///
/// The half that makes PRD resolved q60 ruling b's refusal a **repair**. A key
/// on a reserved list is refused because the construct's bounds are not up for
/// renegotiation from inside `settings:` (Decision D140) — and an author who
/// wrote one wanted something. Where a first-class key answers that want, the
/// diagnostic names it; where nothing does, it says so rather than pointing at
/// the nearest key and being wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answered {
    /// A key of the `coder:` block states this bound, written without its colon.
    By(&'static str),
    /// A key of the block **addresses** what states this bound: the key, written
    /// without its colon, and where the value is actually written.
    ///
    /// The distinction earns its variant because [`By`](Self::By)'s repair is
    /// "write that key instead", which is only true of a key an author has not
    /// written. `model:` is **required** on every `coder:` block and is a
    /// registry address (grammar 8.9, Decision D141): the thinking budget a
    /// harness takes lives in the `model.*` definition it names, so an author
    /// sent to `model:` alone would be sent to a line already on their node.
    Through(&'static str, &'static str),
    /// Nothing states it: the option is **excluded** rather than restated, and
    /// this is why.
    Nothing(&'static str),
}

/// One option the generated adapter owns, and what answers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReservedOption {
    /// The option, spelled as the harness's own SDK surface spells it — which
    /// is also the `settings:` key that would reach it.
    pub option: &'static str,
    /// The first-class key that states the same bound, or why none does.
    pub answered: Answered,
}

/// One harness's reserved row.
pub struct ReservedRow {
    /// The harness this row is for.
    pub harness: Harness,
    /// The SDK release the option surface was read out of (PRD 5.12).
    pub audited: &'static str,
    /// Every option the adapter owns, sorted by name — the order the emitted
    /// driver's own list is written in, because the two are one table.
    pub options: &'static [ReservedOption],
}

/// The reserved-option table (PRD resolved q60 ruling b).
///
/// # Why this is a table here and not only a list in the driver
///
/// It was a list in the driver alone, read back out of the TypeScript by
/// `validate` so a **warning** could say which half of Decision D140 a key landed
/// in. PRD resolved q60 ruling b makes it an **error**, and an error is a
/// statement this compiler makes rather than one the runtime performs: it names
/// the option, the bound it would reach, and the key that answers the need. The
/// answer is the part no list of strings could carry, and it is the part an
/// author repairs against. The emitted drivers keep their own lists and keep
/// subtracting at run time — defence in depth, against a binding this compiler
/// release did not write — and `the_reserved_tables_are_one_table` holds the two
/// copies together the way `the_connection_variable_tables_are_one_table` holds
/// the other pair.
///
/// # What the two kinds of row are
///
/// The same two the drivers' own comments draw, and the split survives because
/// it is what an author is told:
///
///  * an option that **spells** a bound another key states — `cwd` is
///    `workspace:`, `permissionMode` is `permission_mode:` (which is the key PRD
///    resolved q60 ruling a exists to give this answer), `sandboxMode` is
///    `access:`, `outputFormat` is `output:`;
///  * an option that **contains** one without spelling it — `extraArgs` is any
///    CLI flag there is, `mcpServers`, `agents` and `skills` put a tool or a
///    whole loop outside `allow_tools:`, the process-spawn family replaces the
///    program that enforces every bound.
///
/// A row's [`Answered`] is which key a reader should write **instead**, and four
/// families answer to nothing: the resume family, which PRD resolved q57 ruling b
/// excludes by name; `fallbackModel` and `approvalPolicy`, which are a ladder and
/// a tier that stop at this boundary; `extraArgs`, which is not one bound to
/// point at but all of them at once; and the **process-spawn family**, which is
/// the one grammar 8.9 says does not widen a bound but replaces the program
/// enforcing all of them — `harness:` is required on every block and names which
/// vendor's adapter runs, never which executable that adapter spawns or what
/// runtime spawns it, so pointing an author at it would name a key they have
/// already written and that cannot say what they asked for.
///
/// A fifth answer is [`Answered::Through`]: `model:` is an **address**, so the
/// one model setting a harness takes — `cc`'s thinking budget, `codex`'s
/// reasoning effort — is stated in the `model.*` definition it names rather than
/// on this block (Decision D141). The key is real and is already on the node,
/// which is exactly why "write it instead" would be the wrong sentence.
pub const RESERVED: &[ReservedRow] = &[
    ReservedRow {
        harness: Harness::Cc,
        audited: "0.3.272",
        options: CC_RESERVED,
    },
    ReservedRow {
        harness: Harness::Codex,
        audited: "0.154.0",
        options: CODEX_RESERVED,
    },
];

/// `cc`'s reserved options: `@anthropic-ai/claude-agent-sdk`'s `Options`.
const CC_RESERVED: &[ReservedOption] = &[
    ReservedOption {
        option: "abortController",
        answered: Answered::By("timeout"),
    },
    ReservedOption {
        option: "additionalDirectories",
        answered: Answered::By("workspace"),
    },
    ReservedOption {
        option: "agent",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "agents",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        // The flag the SDK requires beside `bypassPermissions`, which is the
        // mode `permission_mode:` states and `access: full_access` derives.
        option: "allowDangerouslySkipPermissions",
        answered: Answered::By("permission_mode"),
    },
    ReservedOption {
        option: "allowedTools",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "canUseTool",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "continue",
        answered: Answered::Nothing(RESUME),
    },
    ReservedOption {
        option: "cwd",
        answered: Answered::By("workspace"),
    },
    ReservedOption {
        option: "env",
        answered: Answered::By("env"),
    },
    ReservedOption {
        option: "executable",
        answered: Answered::Nothing(SPAWN),
    },
    ReservedOption {
        option: "executableArgs",
        answered: Answered::Nothing(SPAWN),
    },
    ReservedOption {
        option: "extraArgs",
        answered: Answered::Nothing(
            "an arbitrary command-line flag is every bound at once — \
             `dangerously-skip-permissions` and `add-dir` among them — so no one key answers it",
        ),
    },
    ReservedOption {
        option: "fallbackModel",
        answered: Answered::Nothing(LADDER),
    },
    ReservedOption {
        option: "forkSession",
        answered: Answered::Nothing(RESUME),
    },
    ReservedOption {
        option: "hooks",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "managedSettings",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "maxThinkingTokens",
        answered: Answered::Through("model", THINKING),
    },
    ReservedOption {
        option: "mcpServers",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "model",
        answered: Answered::By("model"),
    },
    ReservedOption {
        option: "outputFormat",
        answered: Answered::By("output"),
    },
    ReservedOption {
        option: "pathToClaudeCodeExecutable",
        answered: Answered::Nothing(SPAWN),
    },
    ReservedOption {
        // The key PRD resolved q60 ruling a exists to hand this row an answer:
        // before it, the modes beyond the three `access:` derives were reachable
        // through no key at all, and `settings:` was where authors went looking.
        option: "permissionMode",
        answered: Answered::By("permission_mode"),
    },
    ReservedOption {
        option: "permissionPromptToolName",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "permissionPrompts",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        // Plan mode's body, which the adapter fills from the node's own
        // instructions (grammar 8.9's `read_only` row).
        option: "planModeInstructions",
        answered: Answered::By("prompt"),
    },
    ReservedOption {
        option: "plugins",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "resume",
        answered: Answered::Nothing(RESUME),
    },
    ReservedOption {
        option: "resumeDropsTurn",
        answered: Answered::Nothing(RESUME),
    },
    ReservedOption {
        option: "resumeSessionAt",
        answered: Answered::Nothing(RESUME),
    },
    ReservedOption {
        option: "sandbox",
        answered: Answered::By("access"),
    },
    ReservedOption {
        option: "sessionId",
        answered: Answered::Nothing(RESUME),
    },
    ReservedOption {
        option: "settingSources",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "settings",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        // The SDK's single switch for turning skills on — and the one that says
        // so: "you do not need to add `'Skill'` to `allowedTools` yourself when
        // using this option". A loop's worth of instructions within reach of a
        // run whose `allow_tools:` never named it, exactly as `plugins` (which
        // carries skills among other things) and `agents` are.
        option: "skills",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        // The fourth member of the process-spawn family: a function called "in
        // place of the default local spawn". From YAML it can only ever be a
        // non-function, so a key spelling it would reach the SDK and die inside
        // the vendor's own code rather than at `validate`.
        option: "spawnClaudeCodeProcess",
        answered: Answered::Nothing(SPAWN),
    },
    ReservedOption {
        option: "systemPrompt",
        answered: Answered::By("prompt"),
    },
    ReservedOption {
        option: "thinking",
        answered: Answered::Through("model", THINKING),
    },
    ReservedOption {
        option: "toolAliases",
        answered: Answered::By("allow_tools"),
    },
    ReservedOption {
        option: "tools",
        answered: Answered::By("allow_tools"),
    },
];

/// `codex`'s reserved options: `@openai/codex-sdk`'s `ThreadOptions`.
const CODEX_RESERVED: &[ReservedOption] = &[
    ReservedOption {
        option: "additionalDirectories",
        answered: Answered::By("workspace"),
    },
    ReservedOption {
        // The per-call approval tier PRD resolved q57 ruling c does not adopt —
        // and the reason `permission_mode:` is refused on this harness rather
        // than mapped onto it (PRD resolved q60 ruling a).
        option: "approvalPolicy",
        answered: Answered::Nothing(
            "a per-call approval tier belongs to an app server this release does not adopt, so a \
             run that escalated out of its sandbox would reach an approver nothing here answers \
             for",
        ),
    },
    ReservedOption {
        option: "model",
        answered: Answered::By("model"),
    },
    ReservedOption {
        option: "modelReasoningEffort",
        answered: Answered::Through("model", REASONING),
    },
    ReservedOption {
        option: "sandboxMode",
        answered: Answered::By("access"),
    },
    ReservedOption {
        option: "workingDirectory",
        answered: Answered::By("workspace"),
    },
];

/// Why the resume family answers to no key (PRD resolved q57 ruling b).
const RESUME: &str = "harness-native resume is a named exclusion rather than a bound: a vendor's \
                      session store is machine-local, and the journal is the complete hub state";

/// …and why a fallback model does (Decision D141).
const LADDER: &str = "a fallback model is the failover ladder, which does not reach inside a \
                      harness run: the harness owns its client and its own retries, and this \
                      compiler's `retry:` wraps whole runs";

/// …and why the process-spawn family does (grammar 8.9, PRD resolved q57 ruling
/// c).
///
/// Not `Answered::By("harness")`, which is the answer this family had and the
/// one that cannot be followed: `harness:` is **required** on every `coder:`
/// block and takes `cc` or `codex` — an author who asked for a different binary,
/// a different JavaScript runtime, or a spawn function of their own would be
/// sent to a key already on their node that cannot name any of those things.
/// Grammar 8.9 puts this family in the other class in so many words: a key there
/// "does not widen one bound, it replaces or re-arms the program that enforces
/// all of them".
const SPAWN: &str = "which program a run is — the binary, the runtime that spawns it, what that \
                     runtime loads first — is not a bound to widen but the thing that enforces \
                     every bound, so `tools`, `canUseTool` and `permissionMode` would be asked \
                     of something the composition never named. `harness:` names which vendor's \
                     adapter runs, never which executable it is";

/// …and where the one model setting each harness takes is written instead — the
/// [`Answered::Through`] rows (Decision D141).
///
/// Two strings rather than one because the `model.*` key is the harness's own:
/// `thinking:` carries `cc`'s budget and `reasoning_effort:` carries `codex`'s
/// effort, and a refusal that named the other harness's key would be the
/// sends-you-to-a-second-diagnostic failure this whole answer exists to avoid.
const THINKING: &str = "`model:` is a registry address, so the one model setting this harness \
                        takes is written in the `model.*` definition it names — `thinking:` in \
                        that definition's own `settings:`, where the provider plugin's schema \
                        checks it (grammar 12.2, Decision D141)";

/// …and `codex`'s half of it.
const REASONING: &str = "`model:` is a registry address, so the one model setting this harness \
                         takes is written in the `model.*` definition it names — \
                         `reasoning_effort:` in that definition's own `settings:`, where the \
                         provider plugin's schema checks it (grammar 12.2, Decision D141)";

/// Every option one harness's adapter owns, in the table's order.
#[must_use]
pub fn reserved_of(harness: Harness) -> &'static [ReservedOption] {
    RESERVED
        .iter()
        .find(|row| row.harness == harness)
        .map_or(&[], |row| row.options)
}

/// The reserved row one `settings:` key would reach, if it reaches one.
#[must_use]
pub fn reserved_option(harness: Harness, key: &str) -> Option<&'static ReservedOption> {
    reserved_of(harness)
        .iter()
        .find(|reserved| reserved.option == key)
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
    ///
    /// **Read as a denylist**, which is the whole of what the guard is worth. An
    /// allowlist of connection-looking names — grammar 4.3's credential fields
    /// plus `headers:` — is satisfied by a row that grows a kind whose extra key
    /// is spelled neither way, and `azure_openai` is exactly that kind: its
    /// `base_url:`, `api_key:` and `headers:` are already facts, so an
    /// allowlist-shaped check passes green while its **required** `api_version:`
    /// — half of what makes that endpoint reachable — is declared, crosses
    /// nothing and raises nothing. That is the near-miss this module's own
    /// documentation and Decision D143 single out, so the keys that are *not*
    /// connection facts are the list, named one by one with why, and everything
    /// else on a spoken kind's row has to be one.
    #[test]
    fn a_spoken_kinds_connection_keys_are_all_facts() {
        // Every key of a `provider.*` that says nothing about where traffic goes
        // or how it authenticates. A key outside this list is a connection key
        // until somebody says otherwise **here**, where saying it is a commit.
        const NOT_A_CONNECTION_KEY: &[&str] = &[
            // The wire itself, which a row names rather than carries.
            "kind",
            // Prose, read by a human and by `visualize`.
            "description",
            // The `openai` plugin's own vocabulary: a billing account, not a
            // connection.
            "organization",
            // Request shaping — grammar 12.1's server-side tool suite, which
            // resolved q30's second tier already governs.
            "server_tools",
        ];

        let spelled: Vec<&str> = ConnectionFact::ALL
            .iter()
            .map(|fact| fact.as_str())
            .collect();
        for row in CONNECTION {
            for kind in row.kinds {
                for key in kind.keys() {
                    if NOT_A_CONNECTION_KEY.contains(key) {
                        continue;
                    }
                    assert!(
                        spelled.contains(key),
                        "`{}` speaks `{}`, whose row takes `{key}:` — a connection key with no \
                         `ConnectionFact` is declared, crosses nothing and raises nothing. Give \
                         it a fact (and a slot, or an honest `None`) before this row claims that \
                         kind — or, if `{key}:` really decides neither where a connection's \
                         traffic goes nor how it authenticates, say so in \
                         `NOT_A_CONNECTION_KEY` above (grammar 8.9, 12.1, Decision D143, PRD \
                         resolved q58 ruling b)",
                        row.harness.as_str(),
                        kind.as_str()
                    );
                }
            }
        }

        // …and the guard bites on the kinds no row speaks *yet*, which is the
        // only direction the loop above cannot reach while the table is honest:
        // a row that grew one of these would have to answer for the key named
        // beside it before this test went green again.
        for (kind, key) in [
            (ProviderKind::AzureOpenAi, "api_version"),
            (ProviderKind::Bedrock, "access_key_id"),
            (ProviderKind::Vertex, "credentials_json"),
        ] {
            assert!(
                kind.keys().contains(&key) && !NOT_A_CONNECTION_KEY.contains(&key),
                "`{}` takes `{key}:`, and this guard would wave it through: the near-miss \
                 Decision D143 names is a kind whose *other* connection keys are all facts \
                 already",
                kind.as_str()
            );
            assert!(
                !spelled.contains(&key),
                "`{key}:` is a `ConnectionFact` now, so `{}` may be worth a row — and this \
                 assertion, which pins the guard rather than the table, has to be re-read with it",
                kind.as_str()
            );
        }
    }

    /// **A sibling is another spelling of a fact this row already carries as a
    /// variable** (PRD resolved q58 ruling c).
    ///
    /// Three things a sibling list gets wrong on its own, each silent:
    ///
    ///  1. a sibling of a fact that reaches **no variable at all**. `codex` maps
    ///     `base_url:` onto a typed option the SDK turns into a `--config`
    ///     override rather than into the environment, so a name claimed for that
    ///     fact would refuse a node `env:` entry over a variable nothing in the
    ///     run writes — the over-rejection direction
    ///     `a_connection_that_sets_no_variable_leaves_a_nodes_env_alone` guards
    ///     one step earlier. The test is *reaches a variable*, not *is a
    ///     [`Slot::Variable`]*: `codex`'s credential is an option the SDK
    ///     injects `CODEX_API_KEY` from, and the names the spawned CLI reads
    ///     beside it are siblings of it;
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
                            "`{}` claims `{variable}` for `{}:`, which reaches no environment \
                             variable at all on this row: a sibling of a fact nothing in the run \
                             spells as a variable refuses a node `env:` entry over a name no part \
                             of this run writes",
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
        // …and the harness whose slots are typed options is the *same* question
        // asked one process further out, which is the half this test pinned
        // backwards once: `codex`'s credential option becomes `CODEX_API_KEY` in
        // the environment the SDK spawns its CLI with, and that CLI — pinned by
        // the SDK's own dependency, not out of reach — supports two more auth
        // variables beside it.
        let codex = variables_read(Harness::Codex, &[ConnectionFact::Credential]);
        for name in ["OPENAI_API_KEY", "CODEX_ACCESS_TOKEN"] {
            assert!(
                codex.iter().any(|(_, held)| *held == name),
                "`{name}` is a supported auth variable of the CLI the `codex` SDK spawns and \
                 nothing claims it for `api_key:`: a node `env:` may hand the run a second \
                 identity while the graph document reports the mapped key"
            );
        }
        // The facts that reach no environment claim nothing on that row — the
        // endpoint is a `--config` override and the header slot does not exist —
        // so a `codex` node's `env:` is left alone over both.
        assert!(variables_read(Harness::Codex, &[ConnectionFact::BaseUrl]).is_empty());
        assert!(variables_read(Harness::Codex, &[ConnectionFact::Headers]).is_empty());
        assert!(variables_read(Harness::Codex, &[]).is_empty());
        for harness in [Harness::DeepAgents, Harness::Native] {
            assert!(variables_read(harness, ConnectionFact::ALL).is_empty());
        }
    }

    /// …and the `codex` half of that audit written out by name (grammar 8.9,
    /// Decision D143, PRD resolved q58 ruling c).
    ///
    /// `the_cc_row_claims_the_whole_endpoint_selection_family`'s argument, on the
    /// other row. The pinned CLI's auth surface is a **set**, named together in
    /// its own string table — `auth.json OPENAI_API_KEY CODEX_API_KEY
    /// CODEX_ACCESS_TOKEN`, beside *"Run codex login or provide an API key
    /// through a supported auth env var."* and *"auth is configured, but
    /// multiple auth env vars are present"* — and a row that claimed some of it
    /// would be a completeness claim with the hole ruling c exists to close.
    /// Spelled out rather than counted, for that test's reason: a count is
    /// satisfied by any three names, and what is asserted is these three.
    ///
    /// The whole set is asserted through [`variables_set`] **and**
    /// [`variables_read`] together, because one member of it is the slot: the
    /// SDK writes `CODEX_API_KEY` itself, so it is claimed as a mapped name
    /// rather than as a sibling, and a reader checking only one list would find
    /// a hole that is not there.
    #[test]
    fn the_codex_row_claims_the_whole_supported_auth_family() {
        let claimed: Vec<&str> = variables_set(Harness::Codex, &[ConnectionFact::Credential])
            .into_iter()
            .chain(variables_read(
                Harness::Codex,
                &[ConnectionFact::Credential],
            ))
            .map(|(_, name)| name)
            .collect();
        for name in ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"] {
            assert!(
                claimed.contains(&name),
                "`{name}` is one of the three auth environment variables the pinned Codex CLI \
                 supports and the `codex` row does not claim it for `api_key:`, so a coder \
                 node's `env:` may set it with no diagnostic and authenticate the run as \
                 somebody the composition never named (grammar 8.9, Decision D143 rule 4)"
            );
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

    /// **Each row and its driver's own variable table are one table** (grammar
    /// 8.9, Decision D143, PRD resolved q58 rulings a and c).
    ///
    /// A row above is what `validate` refuses a node `env:` against;
    /// `CC_CONNECTION_VARIABLES` in `src/harness-cc.ts` and
    /// `CODEX_CONNECTION_VARIABLES` in `src/harness-codex.ts` are those same
    /// lists declared again for the runtime, where each driver **removes** those
    /// names from the environment a run inherits before mapping the connection
    /// over the top. The two halves answer one question from opposite ends — one
    /// at compile time about what the composition wrote, one at run time about
    /// what the host process happened to hold — and a name in one and not the
    /// other is a hole with no diagnostic in it: `validate` stays clean, the
    /// graph document and the journal's request identity go on reporting the
    /// mapped name, and the run talks to whatever the inherited selector chose
    /// or authenticates with whatever key the shell had.
    ///
    /// Both harnesses are compared, name for name and in order, the way
    /// `the_curated_settings_table_is_one_table` compares the other pair of
    /// hand-maintained copies this project keeps. **Both**, because the `codex`
    /// half of it was empty on both sides for a round: a row and a driver that
    /// agree on nothing agree, and the check that would have caught it is the
    /// audit against the pinned artifacts rather than this one.
    #[test]
    fn the_connection_variable_tables_are_one_table() {
        const CC: &str = include_str!("codegen/js/harness-cc.ts");
        const CODEX: &str = include_str!("codegen/js/harness-codex.ts");

        for (harness, file, driver, constant) in [
            (
                Harness::Cc,
                "src/harness-cc.ts",
                CC,
                "CC_CONNECTION_VARIABLES",
            ),
            (
                Harness::Codex,
                "src/harness-codex.ts",
                CODEX,
                "CODEX_CONNECTION_VARIABLES",
            ),
        ] {
            let object = driver
                .split_once(&format!("const {constant}"))
                .unwrap_or_else(|| panic!("`{file}` declares `{constant}`"))
                .1;
            let object = &object[..object.find("};").expect("…and closes the object it opened")];
            for fact in ConnectionFact::ALL.iter().copied() {
                // What the driver's own surface spells this fact — a `match`, so
                // a fourth fact cannot arrive without an answer here.
                let key = match fact {
                    ConnectionFact::BaseUrl => "baseUrl",
                    ConnectionFact::Credential => "credential",
                    ConnectionFact::Headers => "headers",
                };
                let array = object
                    .split_once(&format!("{key}: ["))
                    .unwrap_or_else(|| panic!("`{constant}` in `{file}` answers `{key}`"))
                    .1;
                let array = &array[..array.find(']').expect("…and closes the array it opened")];
                let emitted: Vec<&str> = array.split('"').skip(1).step_by(2).collect();
                let owned: Vec<&str> = variables_set(harness, &[fact])
                    .into_iter()
                    .chain(variables_read(harness, &[fact]))
                    .map(|(_, name)| name)
                    .collect();
                assert_eq!(
                    emitted,
                    owned,
                    "`{constant}.{key}` in `{file}` and the `{}` row's `{}:` slot and siblings \
                     here are two copies of one table and have parted company. A name this row \
                     claims and the driver does not scrub is a name an inherited environment \
                     decides the fact with; a name the driver scrubs and this row does not claim \
                     is an `env:` entry `validate` accepts and the run then silently drops",
                    harness.as_str(),
                    fact.as_str()
                );
            }
        }
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

    /// `docs/grammar.md`, read at test time rather than embedded: the two
    /// tables below are bound to what the section an author reads actually
    /// says.
    fn grammar() -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/grammar.md"),
        )
        .expect("`docs/grammar.md` is readable")
    }

    /// **The approval-mode table is an audit of the pinned SDK's own type**
    /// (grammar 8.9, Decision D146, PRD resolved q60 ruling a).
    ///
    /// [`the_connection_table_is_audited_against_the_pinned_sdks`]'s argument on
    /// the other surface those releases own: a mode list is a claim about
    /// somebody else's `.d.ts`, the release is what this repository pins, and a
    /// claim whose ground moved has to be read again rather than carried
    /// forward. A vendor that adds a seventh mode adds it in a release, so the
    /// bump is what re-opens the reading.
    #[test]
    fn the_permission_table_is_audited_against_the_pinned_sdk() {
        for row in PERMISSION {
            assert_eq!(
                version_of(row.harness),
                row.audited,
                "`{}`'s SDK is pinned at {} and its permission row was audited against {}: read \
                 the release's own `PermissionMode` declaration and the sentence it documents \
                 each member with, decide which `access:` level admits a new one, and move this \
                 version up (grammar 8.9, Decision D146)",
                row.harness.as_str(),
                version_of(row.harness),
                row.audited
            );
        }
    }

    /// **Every level answers, its derived mode is one it admits, and an admitted
    /// set is a set** (grammar 8.9, Decision D146).
    ///
    /// Three ways a hand-written admissibility table goes wrong in silence, and
    /// each costs something different:
    ///
    ///  1. a **missing level**. [`admitted`] answers the empty slice for a row
    ///     it cannot find, and an empty admitted set refuses every mode — so a
    ///     level left out of the table reads, at `validate`, as a level that
    ///     admits nothing, with a message listing nothing;
    ///  2. a **derived mode the level does not admit**. The absent key means the
    ///     derived mode exactly (PRD resolved q60 ruling a), so a level whose
    ///     own default could not be written out longhand would make the two
    ///     forms mean different things — and refuse the composition that spells
    ///     out what it already does;
    ///  3. an **unordered or repeated** admitted set, which is only a message
    ///     defect and is still a defect: the list a refusal prints is this one,
    ///     and PRD G3 makes that a product surface.
    #[test]
    fn a_levels_derived_mode_is_one_it_admits() {
        for row in PERMISSION {
            let answered: Vec<WorkspaceAccess> =
                row.levels.iter().map(|level| level.access).collect();
            assert_eq!(
                answered,
                WorkspaceAccess::ALL.to_vec(),
                "`{}`'s permission row does not answer every `access:` level, in order — an \
                 unanswered level admits nothing at all",
                row.harness.as_str()
            );
            for level in row.levels {
                assert!(
                    level.admits.contains(&level.derived),
                    "`{}`'s `access: {}` derives `{}` and does not admit it, so a node spelling \
                     out the mode it already runs under is refused",
                    row.harness.as_str(),
                    level.access.as_str(),
                    level.derived.as_str()
                );
                let ordered: Vec<PermissionMode> = PermissionMode::ALL
                    .iter()
                    .copied()
                    .filter(|mode| level.admits.contains(mode))
                    .collect();
                assert_eq!(
                    level.admits.to_vec(),
                    ordered,
                    "`{}`'s `access: {}` lists its admitted modes out of `PermissionMode::ALL` \
                     order or lists one twice, and that list is what a refusal prints (PRD G3)",
                    row.harness.as_str(),
                    level.access.as_str()
                );
            }
        }
    }

    /// **The widening bound is the ruling's, level by level** (PRD resolved q60
    /// ruling a).
    ///
    /// [`a_levels_derived_mode_is_one_it_admits`] holds the table's *shape*;
    /// this holds its **content**, written out by name for the reason
    /// `the_cc_row_claims_the_whole_endpoint_selection_family` is: a shape check
    /// is satisfied by any well-formed table, and what the ruling fixed is these
    /// three sets. The sharp member is `bypassPermissions` — the mode that turns
    /// the permission machinery off — which a table drifting one entry would let
    /// under `workspace_write`, where `validate` would then accept a node whose
    /// `access:` says writes are bounded to the workspace and whose run is
    /// bounded by nothing.
    #[test]
    fn each_access_level_admits_exactly_the_modes_the_ruling_names() {
        for (access, derived, admits) in [
            (
                WorkspaceAccess::ReadOnly,
                PermissionMode::Plan,
                &[PermissionMode::Plan][..],
            ),
            (
                WorkspaceAccess::WorkspaceWrite,
                PermissionMode::AcceptEdits,
                &[
                    PermissionMode::Default,
                    PermissionMode::AcceptEdits,
                    PermissionMode::DontAsk,
                    PermissionMode::Auto,
                ][..],
            ),
            (
                WorkspaceAccess::FullAccess,
                PermissionMode::BypassPermissions,
                PermissionMode::ALL,
            ),
        ] {
            assert_eq!(
                admitted(Harness::Cc, access),
                admits,
                "`access: {}` does not admit the set PRD resolved q60 ruling a names",
                access.as_str()
            );
            assert_eq!(
                permission_level(Harness::Cc, access).map(|level| level.derived),
                Some(derived),
                "`access: {}` does not derive the mode resolved q57 shipped, so an existing \
                 composition that states no `permission_mode:` changed meaning",
                access.as_str()
            );
        }
        // …and the one refusal the sets exist for, named rather than derived: a
        // level that bounds writes to the workspace does not admit the mode that
        // bypasses the checks holding them there.
        assert!(
            !admitted(Harness::Cc, WorkspaceAccess::WorkspaceWrite)
                .contains(&PermissionMode::BypassPermissions)
        );
        assert!(
            !admitted(Harness::Cc, WorkspaceAccess::ReadOnly)
                .contains(&PermissionMode::AcceptEdits)
        );
    }

    /// **A harness with no approval axis claims nothing** (PRD resolved q60
    /// ruling a, q57 ruling c).
    ///
    /// `codex`'s per-call approval tier is an app server this release does not
    /// adopt, so the honest content of this table is a missing row — and a
    /// missing row has to read as *refuse the key*, never as *admit none of it
    /// quietly*. [`resolved_mode`] is the surface where that distinction is
    /// visible: `None` is "this harness runs under a sandbox preset and there is
    /// no mode to report", which is what the graph document draws and what
    /// `validate` refuses a stated key against.
    #[test]
    fn a_harness_with_no_approval_axis_reports_no_mode() {
        assert!(has_permission_axis(Harness::Cc));
        for harness in [Harness::Codex, Harness::DeepAgents, Harness::Native] {
            assert!(
                !has_permission_axis(harness),
                "`{}` claims an approval axis, so `permission_mode:` would be mapped onto \
                 something this release does not drive",
                harness.as_str()
            );
            for access in WorkspaceAccess::ALL {
                assert!(admitted(harness, *access).is_empty());
                assert!(permission_level(harness, *access).is_none());
            }
        }
        assert_eq!(harnesses_with_a_permission_axis(), vec![Harness::Cc]);
    }

    /// **The compiler's derived mapping and the driver's are one table**
    /// (grammar 8.9, Decision D146, PRD resolved q60 ruling a).
    ///
    /// `CC_PERMISSION` in `src/harness-cc.ts` is what a run with no stated mode
    /// is actually given; the `derived` column above is what `validate` admits
    /// against, what the graph document draws and what `visualize` prints. Two
    /// hand-maintained accounts of one mapping drift in silence, and this pair
    /// drifts into the gap PRD resolved q60 ruling a is about: the document
    /// would report one mode while the run took another, with every surface that
    /// could have complained told the mapping was carried.
    #[test]
    fn the_permission_tables_are_one_table() {
        const CC: &str = include_str!("codegen/js/harness-cc.ts");

        let object = CC
            .split_once("const CC_PERMISSION")
            .expect("`src/harness-cc.ts` declares `CC_PERMISSION`")
            .1;
        let object = &object[..object.find("};").expect("…and closes the object it opened")];
        for level in PERMISSION
            .iter()
            .find(|row| row.harness == Harness::Cc)
            .expect("`cc` has a permission row")
            .levels
        {
            let written = format!("{}: \"{}\"", level.access.as_str(), level.derived.as_str());
            assert!(
                object.contains(&written),
                "`CC_PERMISSION` in `src/harness-cc.ts` does not map `{}` onto `{}`: the table \
                 here and the driver's are two copies of one mapping, and a run whose derived \
                 mode is not the one `validate` admitted against is a bound the graph document \
                 reports and the run does not hold",
                level.access.as_str(),
                level.derived.as_str()
            );
        }
    }

    /// **The reserved table is an audit of the pinned SDKs' option surfaces**
    /// (PRD resolved q60 ruling b).
    ///
    /// `a_reserved_list_is_audited_against_the_pinned_option_surface` in
    /// `src/codegen/harness.rs` says the same thing about the emitted driver's
    /// copy; this says it about the copy `validate` refuses a composition on. A
    /// vendor's new reach-around arrives in a release, so the pin is what
    /// re-opens both audits.
    #[test]
    fn the_reserved_table_is_audited_against_the_pinned_sdks() {
        for row in RESERVED {
            assert_eq!(
                version_of(row.harness),
                row.audited,
                "`{}`'s SDK is pinned at {} and its reserved row was audited against {}: read the \
                 release's own options, add every one that reaches around `workspace:`, \
                 `access:`, `permission_mode:`, `env:`, `output:`, `prompt:`, `allow_tools:`, \
                 `timeout:` or `model:`, answer each with the key that states it, and move this \
                 version up (grammar 8.9, Decision D146)",
                row.harness.as_str(),
                version_of(row.harness),
                row.audited
            );
        }
        for harness in [Harness::DeepAgents, Harness::Native] {
            assert!(
                reserved_of(harness).is_empty(),
                "a reserved harness has no SDK whose options an adapter could own"
            );
        }
    }

    /// **Each reserved row and its driver's own list are one table** (grammar
    /// 8.9, Decision D146, PRD resolved q60 ruling b).
    ///
    /// `the_connection_variable_tables_are_one_table`'s argument, on the third
    /// pair of hand-maintained copies this project keeps. The row above is what
    /// `validate` **refuses** a `settings:` key against; `CC_RESERVED` and
    /// `CODEX_RESERVED` in the emitted drivers are that same list declared again
    /// for the runtime, where `passthrough` subtracts those names before the SDK
    /// sees them. The two answer one question from opposite ends, and a name in
    /// one and not the other is a hole with no diagnostic in it:
    ///
    ///  * a name the **table** holds and the driver does not scrub is one this
    ///    release refuses at `validate` and an artifact built by an older one
    ///    still carries — the run-time subtraction is the defence in depth PRD
    ///    resolved q60 ruling b keeps for exactly that;
    ///  * a name the **driver** scrubs and the table does not hold is worse: the
    ///    composition compiles, `validate` says the key travels to the SDK
    ///    unverified, and the run then silently drops it.
    #[test]
    fn the_reserved_tables_are_one_table() {
        const CC: &str = include_str!("codegen/js/harness-cc.ts");
        const CODEX: &str = include_str!("codegen/js/harness-codex.ts");

        for (harness, file, driver, constant) in [
            (Harness::Cc, "src/harness-cc.ts", CC, "CC_RESERVED"),
            (
                Harness::Codex,
                "src/harness-codex.ts",
                CODEX,
                "CODEX_RESERVED",
            ),
        ] {
            let opened = format!("const {constant}: readonly string[] = [");
            let array = driver
                .split_once(&opened)
                .unwrap_or_else(|| panic!("`{file}` declares `{constant}`"))
                .1;
            let array = &array[..array.find("];").expect("…and closes the array it opened")];
            let mut emitted: Vec<&str> = array.split('"').skip(1).step_by(2).collect();
            emitted.sort_unstable();
            let held: Vec<&str> = reserved_of(harness)
                .iter()
                .map(|reserved| reserved.option)
                .collect();
            assert_eq!(
                emitted,
                held,
                "`{constant}` in `{file}` and the `{}` reserved row here are two copies of one \
                 list and have parted company: a name the row holds and the driver does not \
                 subtract is refused at compile time and carried at run time, and a name the \
                 driver subtracts and the row does not hold is a key `validate` calls unverified \
                 and the run then drops",
                harness.as_str()
            );
        }
    }

    /// **The table's rows are sorted, and no option is listed twice.**
    ///
    /// [`the_reserved_tables_are_one_table`] sorts the driver's copy before
    /// comparing — the driver groups its own list by *why* each name is on it,
    /// which is worth reading there and is not an order this table can promise —
    /// so the sort is what makes the two comparable, and a duplicate would
    /// survive it silently on one side only.
    #[test]
    fn a_reserved_row_lists_each_option_once_and_in_order() {
        for row in RESERVED {
            let mut sorted: Vec<&str> = row.options.iter().map(|held| held.option).collect();
            let listed = sorted.clone();
            sorted.sort_unstable();
            assert_eq!(
                listed,
                sorted,
                "`{}`'s reserved row is not sorted by option name",
                row.harness.as_str()
            );
            let unique: std::collections::BTreeSet<&str> = listed.iter().copied().collect();
            assert_eq!(
                unique.len(),
                listed.len(),
                "`{}`'s reserved row lists an option twice, so one `settings:` key earns two \
                 refusals",
                row.harness.as_str()
            );
        }
    }

    /// **An answer names a key the `coder:` block really has** (PRD resolved q60
    /// ruling b, resolved q23's binds).
    ///
    /// The refusal's whole worth is the repair it names, and a repair naming a
    /// key the grammar does not have is worse than none: an author follows it,
    /// writes the key, and earns `unknown-key` for their trouble. So the answers
    /// are bound to the surface an author reads them on — grammar 8.9 — rather
    /// than to a second list here that could agree with nothing.
    ///
    /// **Both answering variants are held.** [`Answered::By`] names the key to
    /// write and [`Answered::Through`] names the key that *addresses* where to
    /// write it, and an author has to be able to find either one in 8.9. What
    /// this cannot see is a key that exists and cannot be *followed*: `harness:`
    /// is a row of the table, so an answer naming it passes here while sending
    /// an author to a required key that takes `cc` or `codex` and cannot name an
    /// executable. So
    /// [`the_options_the_ruling_names_are_answered_by_the_keys_it_names`] pins
    /// the process-spawn family to [`Answered::Nothing`] by name.
    ///
    /// **Two spellings count**, because a coder node takes keys from two places
    /// and 8.9 writes them two ways: its own are rows of the block's key table,
    /// and the **common node keys** it inherits are named in the paragraph under
    /// that table ("the whole §9.1 chain — `retry:`/`timeout:`/`on_error:` —
    /// wraps the whole run"). `timeout:` is the answer for an abort controller
    /// and is one of those, so a check reading the table alone would refuse the
    /// truest answer in the row.
    #[test]
    fn a_reserved_options_answer_is_a_key_the_coder_block_takes() {
        let grammar = grammar();
        let section = grammar
            .split_once("### 8.9 `coder`")
            .expect("`docs/grammar.md` has the section")
            .1;
        let section = &section[..section
            .find("\n### ")
            .expect("…and the section ends where the next one opens")];
        let table = &section[..section
            .find("Additional keys are a compile error")
            .expect("…and its key table closes with the unknown-key sentence")];
        for row in RESERVED {
            for held in row.options {
                let (Answered::By(key) | Answered::Through(key, _)) = held.answered else {
                    continue;
                };
                assert!(
                    table.contains(&format!("| `{key}` |"))
                        || section.contains(&format!("`{key}:`")),
                    "`{}`'s `{}` is answered by `{key}:`, and grammar 8.9 neither lists it in the \
                     `coder:` block's key table nor names it as a common node key — a refusal \
                     that names a key the block does not take sends an author from one diagnostic \
                     to another",
                    row.harness.as_str(),
                    held.option
                );
            }
        }
    }

    /// …and the two answers PRD resolved q60 ruling b names out loud, pinned
    /// (grammar 8.9, Decision D146).
    ///
    /// The entry names four repairs by hand — `permission_mode:` for the modes,
    /// `access:` for the sandbox, `allow_tools:` for the toolset, `model:` for
    /// the connection — and the first two are the ones the field report was
    /// about: an author reached for `settings: { permissionMode: … }` because no
    /// key answered, and a `codex` author reaching for `sandboxMode` is the same
    /// move on the other harness. Spelled out rather than counted, because an
    /// answer is a sentence and not a count.
    ///
    /// The families whose answer is **not** a plain "write this key" are pinned
    /// here too — resume, the process-spawn family, and the thinking budget — for
    /// the reason the entry gives them a clause of their own: a table tempted to
    /// point at the nearest key gets each of them wrong, and the nearest key for
    /// the last two (`harness:`, `model:`) is a real row of grammar 8.9's table,
    /// so [`a_reserved_options_answer_is_a_key_the_coder_block_takes`] passes
    /// green on exactly the wrong answer.
    #[test]
    fn the_options_the_ruling_names_are_answered_by_the_keys_it_names() {
        for (harness, option, key) in [
            (Harness::Cc, "permissionMode", "permission_mode"),
            (Harness::Cc, "cwd", "workspace"),
            (Harness::Cc, "tools", "allow_tools"),
            (Harness::Cc, "model", "model"),
            (Harness::Codex, "sandboxMode", "access"),
            (Harness::Codex, "workingDirectory", "workspace"),
        ] {
            assert_eq!(
                reserved_option(harness, option).map(|held| held.answered),
                Some(Answered::By(key)),
                "a `harness: {}` `settings:` key spelling `{option}` is not answered by \
                 `{key}:`, which is the repair PRD resolved q60 ruling b names for it",
                harness.as_str()
            );
        }
        // …and the family that answers to nothing, which a table tempted to
        // point at the nearest key would get wrong: resume is a named exclusion
        // rather than a bound stated elsewhere (PRD resolved q57 ruling b).
        for option in ["resume", "continue", "forkSession", "sessionId"] {
            assert!(
                matches!(
                    reserved_option(Harness::Cc, option).map(|held| held.answered),
                    Some(Answered::Nothing(_))
                ),
                "`{option}` is answered by a key, and harness-native resume is excluded rather \
                 than restated"
            );
        }
        // …and the family that answers to nothing for the *other* reason, which
        // a table pointing at the nearest key gets wrong in the one way this
        // module's own guards cannot see: `harness:` is a real key of grammar
        // 8.9, so `a_reserved_options_answer_is_a_key_the_coder_block_takes`
        // passes green while the refusal sends an author to a **required** key
        // that takes `cc` or `codex` and cannot name an executable, a runtime or
        // a spawn function. Grammar 8.9 puts this family in the other class in
        // so many words — it "does not widen one bound, it replaces or re-arms
        // the program that enforces all of them" — which is `Answered::Nothing`.
        for option in [
            "pathToClaudeCodeExecutable",
            "executable",
            "executableArgs",
            "spawnClaudeCodeProcess",
        ] {
            assert!(
                matches!(
                    reserved_option(Harness::Cc, option).map(|held| held.answered),
                    Some(Answered::Nothing(_))
                ),
                "`{option}` is answered by a key of the block, and no key of the block can say \
                 which program a run is: `harness:` is required, takes `cc` or `codex`, and \
                 names which vendor's adapter runs rather than which executable it is"
            );
        }
        // …and the one that answers *through* a key rather than to one: the
        // thinking budget is written in the `model.*` definition `model:` names
        // (Decision D141), and `model:` is required, so "write `model:` instead"
        // would name a line already on the node.
        for (harness, option) in [
            (Harness::Cc, "thinking"),
            (Harness::Cc, "maxThinkingTokens"),
            (Harness::Codex, "modelReasoningEffort"),
        ] {
            assert!(
                matches!(
                    reserved_option(harness, option).map(|held| held.answered),
                    Some(Answered::Through("model", _))
                ),
                "a `harness: {}` `settings:` key spelling `{option}` is not answered through \
                 `model:`, which is the address the one model setting a harness takes is written \
                 behind",
                harness.as_str()
            );
        }
    }
}
