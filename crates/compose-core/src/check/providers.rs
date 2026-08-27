//! Provider settings and the three capability checks (grammar 11.2, 12).
//!
//! The **credential** rule of grammar 12.1 is not here, and no part of it is:
//! whether an `anthropic` or `openai` provider needs an `api_key:` turns on
//! whether it declares a `base_url:`, which is two literals in one mapping, so
//! it is the parser's (`parse/definition.rs`'s `credential`, Decision D120) and
//! the published schema enforces it — the invariant `parse_invalid.rs`'s
//! `the_parser_rejects_everything_the_published_schema_rejects` holds. The table
//! its message reads is [`ProviderKind::default_endpoint`], which lives on the
//! kind itself in `ast/definition.rs`; a seventh provider kind is added there
//! and nowhere in this file.
//!
//! # Settings
//!
//! `settings:` is the one open object in the logical layer: the editor schema
//! types the common keys and permits the rest, and **the compiler checks the
//! block against the provider plugin's published schema** (grammar 12.2,
//! Decision D40) — which is why `thinking:` on an OpenAI provider is an error
//! at the model definition and not at the first call.
//!
//! v0 ships no plugin registry, so the published schemas are the table below:
//! one row per provider kind, holding the keys that kind accepts and the shape
//! of each. It is the v0 stand-in for what a plugin will publish, it lives in
//! exactly one place so that a registry replaces it rather than joins it, and
//! it is closed in the same sense grammar 12.1's key row is (Decision D106): a
//! key no row names is a key the plugin will not read.
//!
//! # Capabilities
//!
//! Three rules read one capability table (grammar 12.2, 11.2):
//!
//! * an **agent's model** must come from a provider declaring structured output
//!   — agents are structured-output calls, so a model that cannot honor the
//!   contract is a compile error (PRD 5.2);
//! * a **route's members** must be capability-equivalent, so failover cannot
//!   silently break structured output;
//! * a **vector store's `embed.provider`** must serve embeddings (Decision
//!   D116).
//!
//! Every v0 chat provider serves structured output and tool use, so the first
//! two rules cannot fire on a v0 composition and have no negative fixture; they
//! are table-driven all the same, because the table is what a new kind — or a
//! new capability — is added to. The third does fire: `anthropic` publishes no
//! embeddings API, which is why `examples/triage-fanout` embeds through its
//! `openai_compatible` provider rather than its `anthropic` one.

use crate::ast::common::{Address, Literal};
use crate::ast::definition::ProviderKind;
use crate::ast::deploy::PluginValue;
use crate::ast::server_tools::{self, FieldShape, ObjectShape};
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::ir::definition::{DefinitionBody, Model, Provider, ServerTool};

use super::Ctx;

/// What a provider kind can serve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Capabilities {
    /// Constrained decoding against a declared schema — what an agent needs.
    pub(crate) structured_output: bool,
    /// Tool calling.
    pub(crate) tool_use: bool,
    /// An embeddings endpoint — what a `vector` store's `embed:` needs.
    pub(crate) embeddings: bool,
}

/// The v0 capability table (grammar 12.2, 11.2).
pub(crate) const fn capabilities(kind: ProviderKind) -> Capabilities {
    match kind {
        // Anthropic's API has no embeddings endpoint; embedding through an
        // Anthropic connection is not something a plugin could implement.
        ProviderKind::Anthropic => Capabilities {
            structured_output: true,
            tool_use: true,
            embeddings: false,
        },
        ProviderKind::OpenAi
        | ProviderKind::OpenAiCompatible
        | ProviderKind::AzureOpenAi
        | ProviderKind::Bedrock
        | ProviderKind::Vertex => Capabilities {
            structured_output: true,
            tool_use: true,
            embeddings: true,
        },
    }
}

/// The shape of one setting value.
#[derive(Clone, Copy, Debug)]
enum Shape {
    /// A number, inclusive bounds.
    Number(f64, f64),
    /// An integer, inclusive bounds.
    Integer(i64, i64),
    /// A boolean.
    Boolean,
    /// One of a closed set of strings.
    Choice(&'static [&'static str]),
    /// An array of strings.
    Strings,
    /// A nested object with its own keys.
    Object(&'static [(&'static str, Shape)]),
}

/// The keys every provider kind publishes (grammar 12.2).
const COMMON: &[(&str, Shape)] = &[
    ("temperature", Shape::Number(0.0, 2.0)),
    ("top_p", Shape::Number(0.0, 1.0)),
    ("max_tokens", Shape::Integer(1, i64::MAX)),
    ("stop", Shape::Strings),
    ("seed", Shape::Integer(i64::MIN, i64::MAX)),
    ("parallel_tool_calls", Shape::Boolean),
];

const TOP_K: (&str, Shape) = ("top_k", Shape::Integer(1, i64::MAX));
const THINKING: (&str, Shape) = (
    "thinking",
    Shape::Object(&[("budget_tokens", Shape::Integer(1, i64::MAX))]),
);
const REASONING_EFFORT: (&str, Shape) = (
    "reasoning_effort",
    Shape::Choice(&["minimal", "low", "medium", "high"]),
);

/// The keys one kind publishes beyond [`COMMON`].
const fn extra(kind: ProviderKind) -> &'static [(&'static str, Shape)] {
    match kind {
        ProviderKind::Anthropic => &[TOP_K, THINKING],
        ProviderKind::OpenAi | ProviderKind::AzureOpenAi => &[REASONING_EFFORT],
        ProviderKind::OpenAiCompatible | ProviderKind::Vertex => &[TOP_K],
        // Bedrock serves Anthropic's models among others, so its published
        // schema carries their two knobs.
        ProviderKind::Bedrock => &[TOP_K, THINKING],
    }
}

fn published(kind: ProviderKind) -> Vec<(&'static str, Shape)> {
    COMMON
        .iter()
        .copied()
        .chain(extra(kind).iter().copied())
        .collect()
}

/// Check every model's settings and every capability requirement.
pub(crate) fn check(ctx: &mut Ctx) {
    let ir = ctx.ir;
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Model(Model::Direct(model)) => {
                let Some(provider) = ctx.provider(&model.provider.value) else {
                    continue;
                };
                let kind = provider.kind;
                let wire = wire_of(provider);
                for (key, value) in &model.settings {
                    setting(ctx, key, value, kind, wire, address, &model.provider);
                }
            }
            DefinitionBody::Model(Model::Route(route)) => {
                equivalence(ctx, address, route);
                server_tool_suites(ctx, address, route);
            }
            DefinitionBody::Agent(agent) => {
                structured_output(ctx, &agent.model, address);
            }
            DefinitionBody::Provider(provider) => {
                for tool in &provider.config.server_tools {
                    server_tool(ctx, address, provider.kind, tool);
                }
                suite_collisions(ctx, address, provider);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Server tools (grammar 12.1, Decision D122)
// ---------------------------------------------------------------------------

/// One `server_tools:` entry, two tiers (Decision D122, resolved q30).
///
/// A tool the kind's curated table names is checked **strictly** — every field
/// the table models against its documented shape, every constraint the vendor
/// states — because a config the provider will refuse is a run that fails on its
/// first model call with a 400 and no span. A tool the table does not name is
/// **warned about and carried**: the table is a convenience that buys
/// diagnostics, never a gate, and a server tool the vendor ships tomorrow has to
/// be usable the day it ships (the no-treadmill constraint q30 settles).
///
/// The second tier is reached at two granularities, because the same constraint
/// applies inside a row. A vendor adds parameters to a tool it already ships,
/// and a row is keyed on `type:` alone — so a *field* outside the row is warned
/// about and carried too, rather than refused. What survives as an error is
/// everything the table can actually speak for: a field it models given the
/// wrong kind of value, a value outside a stated range or a closed set, a
/// required field missing, a pair the vendor refuses together.
fn server_tool(ctx: &mut Ctx, address: &str, kind: ProviderKind, tool: &ServerTool) {
    let type_name = tool.type_name.value.as_str();
    let Some(known) = server_tools::lookup(kind, type_name) else {
        let names = server_tools::type_names(kind);
        let help = crate::parse::reader::suggest(type_name, &names).map_or_else(
            || unverifiable(kind, &names),
            |name| format!("did you mean `{name}`?"),
        );
        ctx.push(
            Diagnostic::warning(
                DiagnosticCode::UnknownServerTool,
                tool.type_name.span.clone(),
                format!(
                    "`{address}` declares the server tool `{type_name}`, which is not one this \
                     compiler release knows `{}` serves: its config is unchecked and travels to \
                     the provider as written",
                    kind.as_str()
                ),
            )
            .with_help(help),
        );
        return;
    };
    let subject = format!("server tool `{type_name}` of `{address}`");
    for required in known.shape.required {
        if !tool.config.contains_key(*required) {
            ctx.error(
                DiagnosticCode::MissingKey,
                &tool.span,
                format!("missing required key `{required}` in {subject}"),
            );
        }
    }
    for [left, right] in known.shape.exclusive {
        let (Some(first), Some(second)) = (tool.config.get(*left), tool.config.get(*right)) else {
            continue;
        };
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::ConflictingKeys,
                second.span.clone(),
                format!("{subject} declares both `{left}` and `{right}`"),
            )
            .with_label(first.span.clone(), format!("`{left}` is declared here"))
            .with_help(format!(
                "one is an allow-list and the other is a deny-list, and a tool carrying both has \
                 no answer to which applies: keep `{left}` or `{right}`, not both"
            )),
        );
    }
    for (key, value) in &tool.config {
        let Some(shape) = known.shape.field(key) else {
            ctx.push(
                Diagnostic::warning(
                    DiagnosticCode::UnknownServerToolField,
                    value.span.clone(),
                    format!(
                        "{subject} declares `{key}`, which is not a field this compiler release \
                         knows it has: the value is unchecked and travels to the provider as \
                         written"
                    ),
                )
                .with_help(unverified_field(type_name, &known.shape.names(), key)),
            );
            continue;
        };
        plugin(ctx, &format!("`{key}` of {subject}"), value, shape);
    }
}

/// The help line a second-tier tool carries: what could not be verified, and
/// what the table does know.
fn unverifiable(kind: ProviderKind, names: &[&str]) -> String {
    if names.is_empty() {
        return format!(
            "this compiler release publishes no server-tool table for `{}` — a gateway may serve \
             any vocabulary — so every entry here is carried unchecked (grammar 12.1, \
             Decision D122)",
            kind.as_str()
        );
    }
    format!(
        "the `{}` server tools this release checks are {}; anything else is carried unchecked, \
         so a tool the vendor ships later works here the day it ships (grammar 12.1, \
         Decision D122)",
        kind.as_str(),
        crate::parse::reader::list(names)
    )
}

/// The help line a field the table cannot speak for carries: the near miss when
/// there is one, and otherwise what this release does check on that tool.
///
/// The two cases the one warning covers are a misspelling and a parameter newer
/// than this compiler, and nothing here can tell them apart — so the help names
/// the near miss where one exists and says what the alternative is where one
/// does not, rather than asserting which case the author is in.
fn unverified_field(type_name: &str, names: &[&str], key: &str) -> String {
    if let Some(near) = crate::parse::reader::suggest(key, names) {
        return format!("did you mean `{near}`?");
    }
    if names.is_empty() {
        return format!(
            "this release models no fields on `{type_name}`, so every key beside `type:` is \
             carried unchecked (grammar 12.1, Decision D122)"
        );
    }
    format!(
        "the fields this release checks on `{type_name}` are {}; anything else is carried \
         unchecked, so a parameter the vendor adds later works here the day it ships \
         (grammar 12.1, Decision D122)",
        crate::parse::reader::list(names)
    )
}

/// The slot of the request's `tools` array one `server_tools:` entry occupies.
///
/// The array a request carries is one namespace, and two entries landing in one
/// slot of it are two tools the model is offered under one identity — which the
/// provider surfaces refuse outright, exactly as they refuse the same shape
/// between two *client* tools (`check::bindings`'s `tool-name-collision`). Which
/// key the slot is keyed on is the wire's:
///
/// * a wire that addresses a tool by **name** — the Messages one, where the
///   table pins the name each dated `type:` reaches the request under, and a
///   gateway's, where the `name:` an entry declares is the author's own claim
///   about the vocabulary it is writing for — is keyed on that name;
/// * the **Responses** wire's built-ins carry no `name:` at all, so the `type:`
///   *is* the slot: one connection offers `web_search` once.
///
/// This is a comparison **within one suite**, and that is what makes the
/// gateway arm answerable at all: two entries of one array declaring one
/// `name:` are one tool twice by the author's own reckoning, whichever key the
/// gateway ends up reading. The other half of §11.5's rule — a suite entry
/// against the *agent's* attached tools — needs to know what the wire keys both
/// sides on and so is stated over the Messages wire alone
/// (`check::bindings`'s `server_tool_collisions`, grammar 12.1, D122).
#[derive(PartialEq, Eq)]
enum Slot {
    /// The `name:` the entry reaches the wire under.
    Named(String),
    /// The `type:`, for an entry that takes no name.
    Typed(String),
}

/// The `name:` one entry reaches a name-addressed wire under, where that is
/// knowable: the canonical one the table pins for a `type:` it has a row for,
/// and otherwise the literal the entry declares.
///
/// `None` for a name that is not decidable here — an interpolated one, whose
/// value is whatever the process is started with (grammar 4.3 class 2), or one
/// the strict tier has already refused for its shape — and `None` on the
/// Responses wire, whose built-ins have no name.
pub(crate) fn wire_name(kind: ProviderKind, tool: &ServerTool) -> Option<String> {
    if kind == ProviderKind::OpenAi {
        return None;
    }
    if let Some(canonical) = server_tools::canonical_name(kind, tool.type_name.value.as_str()) {
        return Some(canonical.to_string());
    }
    match &tool.config.get("name")?.value {
        PluginValue::Text(text) if text.references.is_empty() => Some(text.as_str().to_string()),
        _ => None,
    }
}

/// [`Slot`] for one entry, or `None` where the wire's key for it is not
/// decidable at compile time.
///
/// **The `type:` is a slot only for a tool the table has a row for**, and that
/// is the whole of what keeps this rule out of the second tier's way. Every
/// tabled tool is a singleton on its wire — one connection offers `web_search`
/// once — but nothing here could claim that of a `type:` this release has never
/// seen, and a vendor whose next built-in is declared once *per instance*, told
/// apart by a label rather than by a name, would meet a compiler refusing the
/// composition the vendor documents. So an unknown-tier entry is compared on the
/// literal `name:` it declares, which is the author's own claim about the wire's
/// name space, and on nothing else.
fn slot(kind: ProviderKind, tool: &ServerTool) -> Option<Slot> {
    let type_name = tool.type_name.value.as_str();
    if server_tools::lookup(kind, type_name).is_some() {
        return Some(match server_tools::canonical_name(kind, type_name) {
            Some(name) => Slot::Named(name.to_string()),
            None => Slot::Typed(type_name.to_string()),
        });
    }
    wire_name(kind, tool).map(Slot::Named)
}

/// Two entries of one suite that reach the wire as one tool (grammar 12.1,
/// 11.5, Decision D122).
///
/// The strict tier checks each entry against its own row and would not see this:
/// `code_execution_20250522` and `code_execution_20250825` are two valid
/// configs of two real tools, and the table pins **one** `name:` across both, so
/// a request carrying them both carries `code_execution` twice and is answered
/// 400. That is the failure the canonical-name pinning exists to move to compile
/// time (`ast/server_tools.rs`'s `WEB_SEARCH_NAME`), and pinning the name alone
/// only moves half of it: the other half is a comparison between entries.
///
/// The code is [`DiagnosticCode::ToolNameCollision`] rather than one of this
/// module's own, because the rule is the same rule — the model is offered two
/// different things under one name — reached from the provider's side.
fn suite_collisions(ctx: &mut Ctx, address: &str, provider: &Provider) {
    let tools = &provider.config.server_tools;
    for (position, tool) in tools.iter().enumerate() {
        let Some(here) = slot(provider.kind, tool) else {
            continue;
        };
        let Some(first) = tools[..position]
            .iter()
            .find(|earlier| slot(provider.kind, earlier).as_ref() == Some(&here))
        else {
            continue;
        };
        let type_name = tool.type_name.value.as_str();
        let (message, help) = if first.type_name.value == tool.type_name.value {
            (
                format!("`{address}` declares the server tool `{type_name}` twice"),
                "the suite reaches the request's `tools` array as written, so one `type:` \
                 declared twice offers the model one tool twice: drop one of the two entries \
                 (grammar 12.1, Decision D122)"
                    .to_string(),
            )
        } else {
            let Slot::Named(name) = &here else {
                continue;
            };
            (
                format!(
                    "`{address}` declares `{}` and `{type_name}`, which are one `{name}` tool on \
                     the model's side",
                    first.type_name.value
                ),
                format!(
                    "the wire pairs each dated `type:` with one fixed `name:` — both of these are \
                     `{name}` — and refuses a `tools` array carrying one name twice: keep one of \
                     the two revisions (grammar 12.1, Decision D122)"
                ),
            )
        };
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::ToolNameCollision,
                tool.span.clone(),
                message,
            )
            .with_label(first.span.clone(), "the first is declared here")
            .with_help(help),
        );
    }
}

/// One config value against its documented shape.
fn plugin(ctx: &mut Ctx, subject: &str, value: &Spanned<PluginValue>, shape: FieldShape) {
    match (shape, &value.value) {
        (FieldShape::Integer(low, high), PluginValue::Int(number)) => {
            if *number < low || *number > high {
                ctx.error(
                    DiagnosticCode::ValueOutOfRange,
                    &value.span,
                    format!(
                        "{subject} is {number}, outside {}",
                        bounds(low as f64, high as f64)
                    ),
                );
            }
        }
        (FieldShape::Number(low, high), PluginValue::Int(number)) => {
            range(ctx, subject, &value.span, *number as f64, low, high);
        }
        (FieldShape::Number(low, high), PluginValue::Float(number)) => {
            range(ctx, subject, &value.span, *number, low, high);
        }
        (FieldShape::Boolean, PluginValue::Bool(_)) => {}
        (FieldShape::Text, PluginValue::Text(_)) => {}
        (FieldShape::Choice(choices), PluginValue::Text(text)) => {
            if !text.references.is_empty() {
                // A **knob** that embeds an `${ENV}` reference is not decided
                // here: what it says is whatever the process is started with,
                // and the whole point of grammar 4.3 class 2 is that the
                // compiler does not read it (PRD 5.9). `search_context_size:
                // ${SEARCH_DEPTH}` is a staging deployment searching shallowly,
                // and refusing it would be refusing the class.
                //
                // A **pin** is the one closed set where that reasoning inverts,
                // because the set has one member: the value is the table's,
                // decided by the entry's own `type:`, and an `${ENV}` is
                // redundant where the process holds that constant and a refused
                // request everywhere else. See `FieldShape::pinned`.
                if let Some(only) = shape.pinned() {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::UnexpectedEnvRef,
                            value.span.clone(),
                            format!(
                                "{subject} is `{}`, and this field takes one value: `{only}`",
                                text.as_str()
                            ),
                        )
                        .with_help(format!(
                            "the wire decides this key from the entry's `type:` and refuses a \
                             request that spells it otherwise, so it is read at compile time \
                             rather than at process start: write `{only}` (grammar 12.1, 4.3, \
                             Decision D122)"
                        )),
                    );
                }
                return;
            }
            if !choices.contains(&text.as_str()) {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        value.span.clone(),
                        format!(
                            "{subject} is `{}`, which the provider does not accept",
                            text.as_str()
                        ),
                    )
                    .with_help(format!(
                        "the accepted values are {}",
                        crate::parse::reader::list(choices)
                    )),
                );
            }
        }
        (FieldShape::Strings, PluginValue::Sequence(items)) => {
            for item in items {
                if !matches!(item.value, PluginValue::Text(_)) {
                    ctx.error(
                        DiagnosticCode::TypeMismatch,
                        &item.span,
                        format!(
                            "{subject} takes an array of strings, and this entry is {}",
                            describe_value(&item.value)
                        ),
                    );
                }
            }
        }
        (FieldShape::Object(nested), PluginValue::Mapping(_))
        | (FieldShape::TextOrObject(nested), PluginValue::Mapping(_)) => {
            object(ctx, subject, value, nested);
        }
        (FieldShape::TextOrObject(_), PluginValue::Text(_)) => {}
        // A documented field whose interior this vocabulary cannot state — a
        // union, or a recursive shape. The row knows the tool *has* it, which is
        // the whole of what it buys: no warning, and no claim about the value.
        (FieldShape::Opaque, _) => {}
        (shape, found) => {
            let mut report = Diagnostic::error(
                DiagnosticCode::TypeMismatch,
                value.span.clone(),
                format!(
                    "{subject} takes {}, found {}",
                    shape.description(),
                    describe_value(found)
                ),
            );
            // The one mismatch here that is a *rule* rather than a slip, and
            // the one an author is walked into: `server_tools:` is provider
            // config, so grammar 4.3 class 2 says its string values
            // interpolate — and a field the table types as anything else is
            // read *here*, where there is nothing to read. Saying only that a
            // string was found names the symptom; the reason the value is a
            // string is the `${ENV}` the author wrote on purpose, and the
            // second tier walks them into it (`max_usages: ${N}` validates
            // with a warning whose help says "did you mean `max_uses`?").
            if let PluginValue::Text(text) = found
                && !text.references.is_empty()
            {
                report = report.with_help(
                    "a field the curated table models is read at compile time, and an `${ENV}` \
                     reference reaches the wire as the string it expands to rather than as the \
                     value: only the fields the table types as strings interpolate (grammar 12.1, \
                     4.3 class 2)",
                );
            }
            ctx.push(report);
        }
    }
}

/// A nested config object: its required fields, and each field it declares.
///
/// A key the nested shape does not name is the same case as one its tool does
/// not name, one level down — a vendor grows `user_location:` a field, and the
/// table is a release behind — so it carries the same warning rather than an
/// error (see [`server_tool`]).
fn object(ctx: &mut Ctx, subject: &str, value: &Spanned<PluginValue>, shape: &ObjectShape) {
    let PluginValue::Mapping(entries) = &value.value else {
        return;
    };
    for required in shape.required {
        if !entries
            .iter()
            .any(|entry| entry.key.value.as_str() == *required)
        {
            ctx.error(
                DiagnosticCode::MissingKey,
                &value.span,
                format!("missing required key `{required}` in {subject}"),
            );
        }
    }
    for entry in entries {
        let key = entry.key.value.as_str();
        let Some(nested) = shape.field(key) else {
            let names = shape.names();
            let help = crate::parse::reader::suggest(key, &names).map_or_else(
                || {
                    format!(
                        "the keys this release checks there are {}; anything else is carried \
                         unchecked (grammar 12.1, Decision D122)",
                        crate::parse::reader::list(&names)
                    )
                },
                |near| format!("did you mean `{near}`?"),
            );
            ctx.push(
                Diagnostic::warning(
                    DiagnosticCode::UnknownServerToolField,
                    entry.key.span.clone(),
                    format!(
                        "{subject} declares `{key}`, which is not a key this compiler release \
                         knows it has: the value is unchecked and travels to the provider as \
                         written"
                    ),
                )
                .with_help(help),
            );
            continue;
        };
        plugin(ctx, &format!("`{key}` of {subject}"), &entry.value, nested);
    }
}

/// How a diagnostic names one config value it found.
const fn describe_value(value: &PluginValue) -> &'static str {
    match value {
        PluginValue::Null => "null",
        PluginValue::Bool(_) => "a boolean",
        PluginValue::Int(_) => "an integer",
        PluginValue::Float(_) => "a number",
        PluginValue::Text(_) => "a string",
        PluginValue::Sequence(_) => "a sequence",
        PluginValue::Mapping(_) => "a mapping",
    }
}

/// A route's members declare one server-tool suite (grammar 12.2,
/// Decision D122).
///
/// **A warning, not an error.** Failover capability is per-chain-member by
/// construction — each provider in a chain declares its own array — so a route
/// whose members differ is a composition where which tools the model was offered
/// depends on which member answered. That is a real thing to know and a legal
/// thing to want (a fallback vendor that has no web search is still a fallback),
/// so q30 makes it visible rather than refused.
///
/// **Whole entries, not `type:` strings.** Two members that both declare
/// `web_search_20250305` and give it `max_uses: 1` and `max_uses: 99` offered
/// the model materially different tools, which is the subject of this warning;
/// and the runtime already treats the two ladders as different calls, since a
/// journaled model call's request identity carries the whole config object
/// (`docs/durability.md` §3.1). A copy-then-edit of one provider is exactly how
/// that shape arrives, so it is the shape the check has to see.
fn server_tool_suites(ctx: &mut Ctx, address: &str, route: &crate::ir::definition::RouteModel) {
    let mut baseline: Option<(String, Vec<Entry>, Span)> = None;
    for member in &route.route {
        let Some((provider, definition)) = provider_of(ctx, &member.value) else {
            continue;
        };
        let suite: Vec<Entry> = definition.config.server_tools.iter().map(entry).collect();
        match &baseline {
            None => baseline = Some((provider, suite, member.span.clone())),
            Some((first, want, at)) => {
                if *want == suite {
                    continue;
                }
                ctx.push(
                    Diagnostic::warning(
                        DiagnosticCode::MismatchedServerTools,
                        member.span.clone(),
                        format!(
                            "`{address}` fails over from `{first}` to `{provider}`, which \
                             declares a different `server_tools:` suite: {}",
                            difference(want, &suite)
                        ),
                    )
                    .with_label(
                        at.clone(),
                        format!("`{first}` is the first member's provider"),
                    )
                    .with_help(
                        "a server tool runs on the provider's side, so each member of a route \
                         offers its own suite and which tools the model had depends on which \
                         member served the call: declare the same suite, field for field, on \
                         both providers, or keep this one knowing the difference (grammar 12.2, \
                         Decision D122)",
                    ),
                );
            }
        }
    }
}

/// One `server_tools:` entry as the comparison reads it: the `type:` the two
/// suites are matched up by, and a canonical rendering of everything beside it.
///
/// A rendering rather than the values themselves because the values are spanned,
/// and two members that wrote the same config in two files wrote it at two
/// positions — the wire cannot see that and neither may this check.
#[derive(PartialEq, Eq)]
struct Entry {
    type_name: String,
    config: String,
}

/// [`Entry`] for one declared tool.
fn entry(tool: &ServerTool) -> Entry {
    Entry {
        type_name: tool.type_name.value.clone(),
        config: tool
            .config
            .iter()
            .map(|(key, value)| format!("{key}={}", rendered(&value.value)))
            .collect::<Vec<_>>()
            .join(","),
    }
}

/// One config value, canonically: same value, same string.
///
/// A wire object is a **set** of fields, so a nested mapping's keys are sorted
/// here — the top level already is, since the IR keeps it in a `BTreeMap`. An
/// interpolated string renders as what it says rather than as what it resolves
/// to, which is the only reading available at compile time and the right one:
/// two providers naming `${SEARCH_DOMAINS}` declared the same offer.
fn rendered(value: &PluginValue) -> String {
    match value {
        PluginValue::Null => "null".to_string(),
        PluginValue::Bool(flag) => flag.to_string(),
        PluginValue::Int(number) => number.to_string(),
        PluginValue::Float(number) => number.to_string(),
        PluginValue::Text(text) => format!("{:?}", text.as_str()),
        PluginValue::Sequence(items) => format!(
            "[{}]",
            items
                .iter()
                .map(|item| rendered(&item.value))
                .collect::<Vec<_>>()
                .join(",")
        ),
        PluginValue::Mapping(entries) => {
            let mut fields: Vec<String> = entries
                .iter()
                .map(|entry| format!("{}={}", entry.key.value, rendered(&entry.value.value)))
                .collect();
            fields.sort();
            format!("{{{}}}", fields.join(","))
        }
    }
}

/// What two suites disagree about, as one clause a reader can act on.
fn difference(first: &[Entry], second: &[Entry]) -> String {
    let named = |suite: &[Entry], other: &[Entry]| -> Vec<String> {
        suite
            .iter()
            .filter(|entry| !other.iter().any(|tool| tool.type_name == entry.type_name))
            .map(|entry| entry.type_name.clone())
            .collect()
    };
    let missing = named(first, second);
    let extra = named(second, first);
    // A tool both members declare and configure differently, which is the half a
    // `type:`-only comparison cannot see.
    let mut reconfigured: Vec<String> = Vec::new();
    for entry in first {
        if reconfigured.contains(&entry.type_name) {
            continue;
        }
        let configs = |suite: &[Entry]| -> Vec<String> {
            suite
                .iter()
                .filter(|tool| tool.type_name == entry.type_name)
                .map(|tool| tool.config.clone())
                .collect()
        };
        let theirs = configs(second);
        if !theirs.is_empty() && configs(first) != theirs {
            reconfigured.push(entry.type_name.clone());
        }
    }
    let mut clauses = Vec::new();
    if !missing.is_empty() {
        clauses.push(format!(
            "it does not declare {}",
            crate::parse::reader::list(&missing)
        ));
    }
    if !extra.is_empty() {
        clauses.push(format!(
            "it declares {}",
            crate::parse::reader::list(&extra)
        ));
    }
    if !reconfigured.is_empty() {
        clauses.push(format!(
            "it configures {} differently",
            crate::parse::reader::list(&reconfigured)
        ));
    }
    if clauses.is_empty() {
        // Same tools, same configs, different order — which is a difference the
        // wire can see, since the array reaches the request as written.
        return "the same tools in a different order".to_string();
    }
    clauses.join(", and ")
}

/// The provider one direct `model.*` member resolves to.
fn provider_of<'a>(ctx: &Ctx<'a>, model: &Address) -> Option<(String, &'a Provider)> {
    let Some(Model::Direct(direct)) = ctx.model(model) else {
        return None;
    };
    ctx.provider(&direct.provider.value)
        .map(|provider| (direct.provider.value.to_string(), provider))
}

/// Which HTTP surface a connection speaks, where its kind alone does not say.
///
/// One kind forks: an `openai` provider that declares `server_tools:` issues
/// **every** one of its calls to the Responses API, because that is the only
/// OpenAI surface the built-in tool suite exists on (Decision D122). The fork is
/// the compiler's own — one key beside another in one mapping decides it — which
/// is what makes the settings that surface has no equivalent of a compile-time
/// question rather than a 400 on the first model call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Wire {
    /// Whatever the kind's row says: Messages, Chat Completions, or the Azure
    /// spelling of the latter.
    OfItsKind,
    /// OpenAI's Responses API.
    Responses,
}

/// [`Wire`] for one resolved provider.
fn wire_of(provider: &Provider) -> Wire {
    if provider.kind == ProviderKind::OpenAi && !provider.config.server_tools.is_empty() {
        Wire::Responses
    } else {
        Wire::OfItsKind
    }
}

/// The published `settings:` keys the **Responses** wire has no equivalent of.
///
/// Two of grammar 12.2's keys are Chat Completions' and are not carried over:
/// there is no `stop` and no `seed` on `POST /v1/responses`. The `openai` plugin
/// publishes both — a provider without a suite still takes them — so the row
/// they are refused on is the connection's wire rather than its kind
/// (`WIRE-NOTES` (19), Decision D122).
const RESPONSES_HAS_NO: &[&str] = &["stop", "seed"];

/// One `settings:` entry against the provider kind's published schema.
fn setting(
    ctx: &mut Ctx,
    key: &str,
    value: &Spanned<Literal>,
    kind: ProviderKind,
    wire: Wire,
    model: &str,
    provider: &Spanned<Address>,
) {
    // A knob this connection's **wire** does not have, on a kind that publishes
    // it. Refused here rather than left to the request, for the reason the
    // strict server-tool tier exists: a setting the service will not read is
    // otherwise a run that dies on its first model call with a 400 and no span,
    // and the alternative failure — a `stop` sequence quietly never applying —
    // is worse. The kind's own key row is untouched: move the suite off this
    // provider and both keys come back.
    if wire == Wire::Responses && RESPONSES_HAS_NO.contains(&key) {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownKey,
                value.span.clone(),
                format!(
                    "`{key}` is not a setting the `{}` provider plugin publishes on the \
                     Responses wire",
                    kind.as_str()
                ),
            )
            .with_label(
                provider.span.clone(),
                format!(
                    "`{}` declares `server_tools:`, which moves every call it serves onto \
                     `POST /v1/responses`",
                    provider.value
                ),
            )
            .with_help(format!(
                "the Responses API has no `{key}`, and refuses a request carrying one: drop it \
                 from `{model}`, or declare the suite on a second provider and leave this \
                 connection on Chat Completions (grammar 12.1, Decision D122)"
            )),
        );
        return;
    }
    let schema = published(kind);
    let Some((_, shape)) = schema.iter().find(|(name, _)| *name == key) else {
        let known: Vec<&str> = schema.iter().map(|(name, _)| *name).collect();
        let help = crate::parse::reader::suggest(key, &known).map_or_else(
            || {
                format!(
                    "the `{}` provider plugin publishes {}",
                    kind.as_str(),
                    crate::parse::reader::list(&known)
                )
            },
            |name| format!("did you mean `{name}`?"),
        );
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnknownKey,
                value.span.clone(),
                format!(
                    "`{key}` is not a setting the `{}` provider plugin publishes",
                    kind.as_str()
                ),
            )
            .with_label(provider.span.clone(), "the provider is named here")
            .with_help(help),
        );
        return;
    };
    literal(ctx, &format!("`{key}` of `{model}`"), value, *shape);
}

fn literal(ctx: &mut Ctx, subject: &str, value: &Spanned<Literal>, shape: Shape) {
    match (shape, &value.value) {
        (Shape::Number(low, high), Literal::Int(number)) => {
            range(ctx, subject, &value.span, *number as f64, low, high);
        }
        (Shape::Number(low, high), Literal::Float(number)) => {
            range(ctx, subject, &value.span, *number, low, high);
        }
        (Shape::Integer(low, high), Literal::Int(number)) => {
            if *number < low || *number > high {
                ctx.error(
                    DiagnosticCode::ValueOutOfRange,
                    &value.span,
                    format!(
                        "{subject} is {number}, outside {}",
                        bounds(low as f64, high as f64)
                    ),
                );
            }
        }
        (Shape::Boolean, Literal::Bool(_)) => {}
        (Shape::Choice(choices), Literal::String(text)) => {
            if !choices.contains(&text.as_str()) {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        value.span.clone(),
                        format!("{subject} is `{text}`, which the provider plugin does not accept"),
                    )
                    .with_help(format!(
                        "the accepted values are {}",
                        crate::parse::reader::list(choices)
                    )),
                );
            }
        }
        (Shape::Strings, Literal::Sequence(items)) => {
            for item in items {
                if !matches!(item.value, Literal::String(_)) {
                    ctx.error(
                        DiagnosticCode::TypeMismatch,
                        &item.span,
                        format!(
                            "{subject} takes an array of strings, and this entry is {}",
                            item.value.description()
                        ),
                    );
                }
            }
        }
        (Shape::Object(keys), Literal::Mapping(entries)) => {
            for entry in entries {
                let Some((_, shape)) = keys.iter().find(|(name, _)| *name == entry.key.value)
                else {
                    let known: Vec<&str> = keys.iter().map(|(name, _)| *name).collect();
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::UnknownKey,
                            entry.key.span.clone(),
                            format!("`{}` is not a key of {subject}", entry.key.value),
                        )
                        .with_help(format!("it takes {}", crate::parse::reader::list(&known))),
                    );
                    continue;
                };
                literal(
                    ctx,
                    &format!("`{}` of {subject}", entry.key.value),
                    &entry.value,
                    *shape,
                );
            }
        }
        (shape, found) => {
            ctx.error(
                DiagnosticCode::TypeMismatch,
                &value.span,
                format!(
                    "{subject} takes {}, found {}",
                    describe(shape),
                    found.description()
                ),
            );
        }
    }
}

fn range(ctx: &mut Ctx, subject: &str, at: &Span, value: f64, low: f64, high: f64) {
    if value < low || value > high {
        ctx.error(
            DiagnosticCode::ValueOutOfRange,
            at,
            format!("{subject} is {value}, outside {}", bounds(low, high)),
        );
    }
}

fn bounds(low: f64, high: f64) -> String {
    match (low, high) {
        (low, high) if high == i64::MAX as f64 => format!("the range {low} and above"),
        (low, high) if low == i64::MIN as f64 => format!("the range up to {high}"),
        (low, high) => format!("the range {low} to {high}"),
    }
}

const fn describe(shape: Shape) -> &'static str {
    match shape {
        Shape::Number(..) => "a number",
        Shape::Integer(..) => "an integer",
        Shape::Boolean => "a boolean",
        Shape::Choice(_) => "a string",
        Shape::Strings => "an array of strings",
        Shape::Object(_) => "a mapping",
    }
}

/// An agent's model resolves to providers that can serve structured output
/// (grammar 12.2, PRD 5.2).
fn structured_output(ctx: &mut Ctx, model: &Spanned<Address>, agent: &str) {
    for (address, provider) in providers_of(ctx, &model.value) {
        if capabilities(provider.kind).structured_output {
            continue;
        }
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::MissingCapability,
                model.span.clone(),
                format!(
                    "`{agent}` needs structured output, and `{}` is served by `{address}`, which does not publish it",
                    model.value
                ),
            )
            .with_help(
                "an agent is one LLM call with structured output, so every model it references must come from a provider that can honor the contract (grammar 12.2, PRD 5.2)",
            ),
        );
    }
}

/// A route's members are capability-equivalent (grammar 12.2).
fn equivalence(ctx: &mut Ctx, address: &str, route: &crate::ir::definition::RouteModel) {
    let mut baseline: Option<(String, Capabilities)> = None;
    for member in &route.route {
        let Some((provider, definition)) = providers_of(ctx, &member.value).into_iter().next()
        else {
            continue;
        };
        let capabilities = capabilities(definition.kind);
        match &baseline {
            None => baseline = Some((provider, capabilities)),
            Some((first, want)) => {
                if inference(*want) != inference(capabilities) {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::MissingCapability,
                            member.span.clone(),
                            format!(
                                "`{address}` fails over to `{}`, which is not capability-equivalent to `{first}`",
                                member.value
                            ),
                        )
                        .with_help(
                            "every member of a route must publish the same inference capabilities, so a failover cannot silently break structured output (grammar 12.2)",
                        ),
                    );
                }
            }
        }
    }
}

/// The capabilities a `model.*` reference needs of its provider: what a chat
/// call uses, never what an embedding call does.
const fn inference(capabilities: Capabilities) -> (bool, bool) {
    (capabilities.structured_output, capabilities.tool_use)
}

/// A `vector` store's embedding connection must serve embeddings (grammar
/// 11.2, Decision D116).
pub(crate) fn require_embeddings(ctx: &mut Ctx, provider: &Spanned<Address>, store: &str) {
    let Some(definition) = ctx.provider(&provider.value) else {
        return;
    };
    if capabilities(definition.kind).embeddings {
        return;
    }
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::MissingCapability,
            provider.span.clone(),
            format!(
                "`{store}` embeds through `{}`, whose `{}` plugin publishes no embeddings",
                provider.value,
                definition.kind.as_str()
            ),
        )
        .with_help(
            "`embed.provider` names the connection that turns text into a vector, and the storage backend never computes them: name a provider whose kind serves embeddings (grammar 11.2, Decision D116)",
        ),
    );
}

/// The providers a `model.*` reference resolves to: one for a direct model,
/// one per member for a route.
///
/// The whole width of the ladder, because that is the granularity a route's
/// per-member declarations have: any member may serve the call, so a rule about
/// what the connection publishes — a capability, or a server-tool suite — has
/// to hold of each.
pub(crate) fn providers_of<'a>(ctx: &Ctx<'a>, model: &Address) -> Vec<(String, &'a Provider)> {
    match ctx.model(model) {
        Some(Model::Direct(_)) => provider_of(ctx, model).into_iter().collect(),
        Some(Model::Route(route)) => route
            .route
            .iter()
            .filter_map(|member| provider_of(ctx, &member.value))
            .collect(),
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind v0 admits, so the two tests below read the whole table.
    const V0: &[ProviderKind] = &[
        ProviderKind::Anthropic,
        ProviderKind::OpenAi,
        ProviderKind::OpenAiCompatible,
        ProviderKind::AzureOpenAi,
        ProviderKind::Bedrock,
        ProviderKind::Vertex,
    ];

    #[test]
    fn every_v0_kind_serves_structured_output() {
        for kind in V0 {
            assert!(
                capabilities(*kind).structured_output,
                "{} must serve structured output for agents to be bindable to it",
                kind.as_str()
            );
        }
    }

    /// Why [`equivalence`] has nothing to reject on a v0 composition, stated
    /// over the table rather than left as a remark: every kind publishes the
    /// same pair of inference capabilities, so every route is equivalent by
    /// construction and no legal spec can reach that diagnostic. It is the
    /// evidence `tests/static_check_inventory.rs` carries for that rule in place of a
    /// negative fixture, and the day a kind arrives that lacks one of the two,
    /// this test fails and the fixture becomes writable (grammar 12.2).
    #[test]
    fn every_v0_kind_publishes_the_same_inference_capabilities() {
        let baseline = inference(capabilities(ProviderKind::Anthropic));
        assert_eq!(baseline, (true, true));
        for kind in V0 {
            assert_eq!(
                inference(capabilities(*kind)),
                baseline,
                "{} differs, so a route mixing it with another kind is now rejectable",
                kind.as_str()
            );
        }
    }

    /// The one capability a v0 kind lacks, and the reason Decision D116's check
    /// has anything to reject.
    #[test]
    fn anthropic_publishes_no_embeddings() {
        assert!(!capabilities(ProviderKind::Anthropic).embeddings);
        assert!(capabilities(ProviderKind::OpenAiCompatible).embeddings);
    }

    /// The Responses wire refuses exactly the keys the `openai` plugin
    /// **publishes** and that wire does not have.
    ///
    /// Both halves matter. A key the plugin does not publish would already be
    /// `unknown-key` on every `openai` provider, so listing one here would be
    /// dead code wearing a more specific message; and a key that quietly left
    /// [`COMMON`] would turn this rule's diagnostic back into the generic one
    /// without any test noticing. The pair is the launch scope of Decision
    /// D122's wire fork, so it is pinned rather than merely derived.
    #[test]
    fn the_responses_wire_refuses_only_keys_the_openai_plugin_publishes() {
        let openai: Vec<&str> = published(ProviderKind::OpenAi)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        for key in RESPONSES_HAS_NO {
            assert!(
                openai.contains(key),
                "`{key}` is refused on the Responses wire, and the `openai` plugin does not \
                 publish it — so the refusal can never fire and `unknown-key` answers first"
            );
        }
        assert_eq!(RESPONSES_HAS_NO, ["stop", "seed"]);
    }

    /// Every code one composition reports, in the order the report is sorted
    /// into. The `version:` line is prepended so a case is only its providers.
    fn codes(body: &str) -> Vec<String> {
        let ir = crate::codegen::test_support::ir_of(&format!("version: \"0.1\"\n{body}"));
        crate::check::check(&ir)
            .iter()
            .map(|diagnostic| diagnostic.code.to_string())
            .collect()
    }

    /// The suite-collision rule keys a `type:` only where the curated table has
    /// a row for it, and the second tier is where that matters.
    ///
    /// A tabled tool is a singleton on its wire — one connection offers
    /// `web_search` once — and the row is what says so. Nothing could say it of
    /// a `type:` this release has never seen, and a vendor whose next built-in
    /// is declared once *per instance*, told apart by a label rather than a
    /// name, must not meet a compiler refusing the shape that vendor documents.
    /// So an unknown-tier entry is compared on the literal `name:` it declares
    /// and on nothing else, which is also the one thing a gateway's vocabulary
    /// cannot make the compiler wrong about: two entries the author named the
    /// same are two entries the author named the same.
    #[test]
    fn the_second_tier_collides_on_a_name_it_was_given_and_never_on_a_type() {
        // Tabled, and its wire gives it no `name:`: the `type:` is the slot.
        assert!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: web_search
    - type: web_search
"#
            )
            .contains(&"tool-name-collision".to_string())
        );
        // The same shape one tier down — a vendor's next built-in, declared
        // twice because that is how it is used — is carried, warned about, and
        // not refused.
        assert_eq!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: remote_toolset
      server_label: docs
    - type: remote_toolset
      server_label: tickets
"#
            ),
            ["unknown-server-tool", "unknown-server-tool"],
            "an unknown `type:` is not a slot this release can claim is single"
        );
        // A gateway serves no table at all, so the same holds there — until the
        // author names two entries alike, which is their own claim rather than
        // the compiler's.
        assert_eq!(
            codes(
                r#"
provider.g:
  kind: openai_compatible
  base_url: ${GATEWAY_URL}
  server_tools:
    - type: retrieval
      name: search_docs
    - type: retrieval
      name: search_tickets
"#
            ),
            ["unknown-server-tool", "unknown-server-tool"]
        );
        assert_eq!(
            codes(
                r#"
provider.g:
  kind: openai_compatible
  base_url: ${GATEWAY_URL}
  server_tools:
    - type: retrieval
      name: search
    - type: lookup
      name: search
"#
            ),
            // Source order: the collision is reported against the entry written
            // second, whose own `unknown-server-tool` warning sits one line
            // further down.
            [
                "unknown-server-tool",
                "tool-name-collision",
                "unknown-server-tool"
            ]
        );
    }

    /// A closed set of **more than one** is a deployment knob, and interpolates
    /// like every other class 2 provider string (grammar 12.1, 4.3).
    ///
    /// The value is a string either way — `FieldShape::description` says so of
    /// `Choice` and `Text` alike — so nothing here is read at compile time
    /// except when the compiler can see what was written. A staging deployment
    /// that searches shallowly and a production one that does not is the whole
    /// of why class 2 exists, and refusing this would be refusing the class.
    ///
    /// Held against the published schema by
    /// `tests/schema_conformance.rs`'s
    /// `the_published_schema_interpolates_a_closed_set_the_table_does_not_pin`,
    /// which is the other authority an author's editor consults: the two
    /// disagreeing means a red squiggle on YAML that validates.
    #[test]
    fn a_closed_set_of_several_values_is_a_knob_a_deployment_may_turn() {
        assert_eq!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: web_search
      search_context_size: ${SEARCH_DEPTH}
    - type: image_generation
      quality: ${IMAGE_QUALITY}
"#
            ),
            Vec::<String>::new(),
            "what the process is started with is not the compiler's to read (PRD 5.9)"
        );
        // …and the set is still closed for a value the compiler *can* read.
        assert_eq!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: web_search
      search_context_size: medum
"#
            ),
            ["unknown-variant"]
        );
    }

    /// A closed set of **one** is a pin rather than a knob, and inverts every
    /// clause of the test above (`FieldShape::pinned`).
    ///
    /// The only legal value is a constant the curated table already holds,
    /// decided by the entry's own `type:` — so a reference here is redundant
    /// where the process happens to hold that constant and a 400 everywhere
    /// else, which is exactly the failure the pin was added to move to compile
    /// time. Skipping it because the value carries an `${ENV}` is that pin not
    /// holding, on either of the two wires that has one and at either depth.
    #[test]
    fn a_closed_set_of_one_value_is_decided_by_the_table_and_not_the_environment() {
        assert_eq!(
            codes(
                r#"
provider.a:
  kind: anthropic
  api_key: ${K}
  server_tools:
    - type: web_search_20250305
      name: ${WS_NAME}
"#
            ),
            ["unexpected-env-ref"]
        );
        // Nested, one level down: `user_location:` is an `approximate` one, and
        // the row's own `cache_control:` is `ephemeral`.
        assert_eq!(
            codes(
                r#"
provider.a:
  kind: anthropic
  api_key: ${K}
  server_tools:
    - type: web_search_20250305
      name: web_search
      user_location:
        type: ${LOCATION_KIND}
      cache_control:
        type: ${CACHE_KIND}
"#
            ),
            ["unexpected-env-ref", "unexpected-env-ref"]
        );
        // And on the Responses wire, whose `container:` is written both ways —
        // a container id interpolates, the object form's `type:` does not.
        assert_eq!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: code_interpreter
      container: ${CONTAINER_ID}
"#
            ),
            Vec::<String>::new(),
            "`container:` in its string form is a class 2 value like any other"
        );
        assert_eq!(
            codes(
                r#"
provider.o:
  kind: openai
  api_key: ${K}
  server_tools:
    - type: code_interpreter
      container:
        type: ${CONTAINER_KIND}
"#
            ),
            ["unexpected-env-ref"]
        );
    }

    #[test]
    fn thinking_is_published_by_the_kinds_that_serve_it() {
        let anthropic: Vec<&str> = published(ProviderKind::Anthropic)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(anthropic.contains(&"thinking"));
        assert!(anthropic.contains(&"max_tokens"));
        let openai: Vec<&str> = published(ProviderKind::OpenAi)
            .iter()
            .map(|(name, _)| *name)
            .collect();
        assert!(!openai.contains(&"thinking"));
        assert!(openai.contains(&"reasoning_effort"));
    }
}
