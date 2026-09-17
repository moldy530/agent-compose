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
//! # Where each slot was read
//!
//! Every slot below was verified against the pinned SDK's own `.d.ts` and
//! documented contract, exactly as resolved q57 verified its API shapes — never
//! recalled. [`CONNECTION`] records the release each row was read against, and
//! `the_connection_table_is_audited_against_the_pinned_sdks` fails when the pin
//! moves, so a vendor's new slot arrives with the bump rather than behind it.

use crate::ast::flow::Harness;
use crate::ir::Ir;
use crate::ir::definition::{DefinitionBody, Model, Provider};
use crate::ir::flow::Coder;

/// One fact of a provider connection, as grammar 12.1 spells it.
///
/// Three, and no more: these are the keys of a `provider.*` that say **where**
/// the traffic goes and **how** it authenticates. Everything else in a provider
/// definition is either a plugin's own vocabulary (`region:`, `project:`, an
/// `api_version:`) or a request-shaping key (`server_tools:`), and neither is a
/// connection fact a harness client has a place for — the harness opens its own
/// connection and speaks its own wire.
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

/// One harness's row: the SDK release it was read against, and a slot per fact.
pub struct ConnectionRow {
    /// The harness this row is for.
    pub harness: Harness,
    /// The SDK version the row was audited against (PRD 5.12).
    pub audited: &'static str,
    /// Each fact's slot, `None` where this harness's SDK tier offers none.
    pub slots: &'static [(ConnectionFact, Option<Slot>)],
}

/// The connection table (PRD resolved q58 ruling b).
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
pub const CONNECTION: &[ConnectionRow] = &[
    ConnectionRow {
        harness: Harness::Cc,
        audited: "0.3.272",
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
    },
    ConnectionRow {
        harness: Harness::Codex,
        audited: "0.154.0",
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
    },
];

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

/// The `ANTHROPIC_CUSTOM_HEADERS` value for a provider's headers.
///
/// The runtime splits the variable on newlines and each line on its first colon,
/// so a header is one `Name: value` line. Written here rather than only in the
/// driver because [`CONNECTION`]'s account of the slot is normative and a second
/// spelling of the encoding is a second thing to keep true.
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
                 header contract — and move this version up (grammar 8.9, Decision D143)",
                row.harness.as_str(),
                version_of(row.harness),
                row.audited
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
