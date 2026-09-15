//! What a `coder:` node's block says that one file cannot decide (grammar 8.9,
//! Decisions D136–D142, PRD resolved q57).
//!
//! The parser owns the block's *shape* — a harness that is not one of the four
//! names, a missing `workspace:`, an `access:` outside the three presets, a
//! duplicate `allow_tools:` entry — because each of those is one value against a
//! constant. Three rules are left, and each needs something the parser does not
//! have:
//!
//! * **the harness ships in this release.** `deepagents` and `native` are
//!   grammar (they are members of the enum, and a composition written against
//!   one is a composition this grammar can express), and what they lack is a
//!   *driver*. The refusal therefore names the v1 scope rather than the
//!   spelling, which is why it is its own code rather than an
//!   `unknown-variant`.
//! * **the model is a direct binding.** A `model.*` may be a failover route
//!   (grammar 12.2), and a route does not reach inside a harness run: PRD
//!   resolved q57 ruling d stops the provider connection, its auth and q53's
//!   mechanism ladder at this boundary, so a ladder here is a declared policy
//!   with nothing to apply it. Taking the first member silently is the one
//!   answer this compiler will not give. Deciding it needs the *definition* the
//!   address names, which is a whole-composition fact.
//! * **the harness config is checked in two tiers.** Decision D140 holds
//!   `settings:` on resolved q30's terms: the keys the curated table knows are
//!   checked strictly, and everything else is a warning naming what could not be
//!   verified and travels to the SDK unchanged — except the keys the *adapter*
//!   owns, which are dropped instead, because unchecked was never meant to mean
//!   unbounded. The warning says which of the two happened. Both lists are per
//!   harness, so the check needs the `harness:` value beside the key — which the
//!   parser has, but the *tables* belong beside the refusals they produce rather
//!   than scattered through the reader.
//!
//! **What is deliberately not checked here** is the enforcement asymmetry of
//! ruling c. `allow_tools:` on a `codex` node is not a mistake — the list is
//! what the harness is offered, and the sandbox is what bounds it — so the
//! compiler states the asymmetry where an author meets it (grammar 8.9, the
//! `plan` report, and the graph document's `tools_enforced`) rather than warning
//! about a composition that is doing exactly what its author wrote.

use crate::ast::common::Literal;
use crate::ast::flow::Harness;
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::definition::{DefinitionBody, Model};
use crate::ir::flow::{Coder, Node};
use crate::parse::reader::{list, suggest};

use super::{Ctx, FlowCx};

/// What one curated `settings:` key may hold (Decision D140's first tier).
enum Shape {
    /// A boolean.
    Flag,
    /// An integer inside a closed range, inclusive at both ends.
    Integer(i64, i64),
    /// A number strictly above zero — a budget, which a zero would silence.
    Positive,
    /// One of a closed set of words.
    Enum(&'static [&'static str]),
    /// An array of non-empty strings.
    Names,
}

impl Shape {
    /// How a diagnostic names what this shape takes.
    const fn expectation(&self) -> &'static str {
        match self {
            Self::Flag => "a boolean",
            Self::Integer(..) => "an integer",
            Self::Positive => "a number above zero",
            Self::Enum(_) => "one of a closed set of words",
            Self::Names => "an array of non-empty strings",
        }
    }
}

/// The `cc` table: what the Claude Agent SDK's options this release maps by name
/// will take (grammar 8.9, Decision D140).
///
/// Four keys, and the choice of which four is the two-tier bargain working: each
/// is an option with a *meaning* a composition can get wrong in a way a run
/// discovers late — a turn bound of zero, a budget written as a string — and
/// everything the vendor ships beside them travels unchecked under a warning
/// rather than waiting for a compiler release.
///
/// The thinking budget is **not** here on purpose: it is the `anthropic`
/// plugin's `thinking:` key on the node's `model.*`, which the adapter maps down
/// (Decision D141). One option with two places to write it is the collision this
/// table exists to avoid.
const CC_SETTINGS: &[(&str, Shape)] = &[
    ("max_turns", Shape::Integer(1, 1000)),
    ("max_budget_usd", Shape::Positive),
    ("forward_subagent_text", Shape::Flag),
    ("disallowed_tools", Shape::Names),
];

/// …and the `codex` table, over the Codex SDK's own thread options.
const CODEX_SETTINGS: &[(&str, Shape)] = &[
    ("network_access", Shape::Flag),
    ("web_search", Shape::Enum(&["disabled", "cached", "live"])),
    ("skip_git_repo_check", Shape::Flag),
];

/// The keys one harness's **adapter owns**, read off the driver that owns them
/// (grammar 8.9, Decision D140).
///
/// The reserved list lives in the emitted driver, because that is where it is
/// applied: `passthrough` drops these rather than handing them to the SDK. It
/// is read here rather than copied here for the reason
/// [`the_curated_settings_table_is_one_table`] states about the other list —
/// two hand-maintained copies of one document drift in silence — and it is read
/// at all so the warning can say which half of Decision D140 a key landed in.
/// A key that is dropped and a key that travels are two different things to be
/// told, and an author who is told the wrong one debugs a run for an option
/// that never reached it.
fn reserved(harness: Harness) -> &'static [String] {
    static CC: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        names_of(include_str!("../codegen/js/harness-cc.ts"), "CC_RESERVED")
    });
    static CODEX: std::sync::LazyLock<Vec<String>> = std::sync::LazyLock::new(|| {
        names_of(
            include_str!("../codegen/js/harness-codex.ts"),
            "CODEX_RESERVED",
        )
    });
    match harness {
        Harness::Cc => &CC,
        Harness::Codex => &CODEX,
        // …and a reserved harness has no driver to read one off, which is the
        // same answer [`table`] gives one key along.
        Harness::DeepAgents | Harness::Native => &[],
    }
}

/// The strings of one `readonly string[]` in a driver's source.
fn names_of(source: &str, declaration: &str) -> Vec<String> {
    let opened = format!("const {declaration}: readonly string[] = [");
    let Some((_, rest)) = source.split_once(&opened) else {
        return Vec::new();
    };
    let Some((body, _)) = rest.split_once("];") else {
        return Vec::new();
    };
    body.split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// One harness's curated table.
const fn table(harness: Harness) -> &'static [(&'static str, Shape)] {
    match harness {
        Harness::Cc => CC_SETTINGS,
        Harness::Codex => CODEX_SETTINGS,
        // A reserved harness is refused before a settings key is read, so its
        // table is empty rather than invented: a warning naming keys nobody can
        // use would be a second complaint about a node that is already refused.
        Harness::DeepAgents | Harness::Native => &[],
    }
}

/// Check one `coder:` node.
pub(crate) fn coder_node(ctx: &mut Ctx<'_>, cx: &FlowCx<'_>, node: &Node, coder: &Coder) {
    let subject = format!("`{}` node `{}`", cx.address, node.id.value);
    if !harness_ships(ctx, &subject, coder) {
        return;
    }
    model_is_direct(ctx, &subject, coder);
    settings(ctx, &subject, coder);
}

/// The harness has a driver in this release (PRD resolved q57).
///
/// Answers whether the rest of the block is worth checking: a reserved harness
/// has no curated settings table and no driver, so a second diagnostic about a
/// key of one would be noise on top of the refusal that matters.
fn harness_ships(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) -> bool {
    if coder.harness.value.ships_in_v1() {
        return true;
    }
    let shipping: Vec<&str> = Harness::ALL
        .iter()
        .filter(|harness| harness.ships_in_v1())
        .map(|harness| harness.as_str())
        .collect();
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnsupportedHarness,
            coder.harness.span.clone(),
            format!(
                "`harness: {}` is reserved: this release lowers {} and no other",
                coder.harness.value.as_str(),
                list(&shipping)
            ),
        )
        .with_label(
            coder.span.clone(),
            format!("the `coder` block of {subject}"),
        )
        .with_help(
            "`deepagents` and `native` are spelled by the grammar so that a composition written \
             against one is refused by name rather than by a suggestion list; what they lack is a \
             driver, and the set grows by a resolved question rather than by a release adding a \
             name (grammar 8.9, PRD resolved q57)",
        ),
    );
    false
}

/// A coder node's `model:` is a **direct** binding (Decision D141).
fn model_is_direct(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) {
    let address = coder.model.value.to_string();
    let Some(definition) = ctx.ir.definitions.get(&address) else {
        // An address that names nothing has already been reported by the
        // resolver, and reporting its *form* on top of that would be a second
        // complaint about one mistake.
        return;
    };
    let DefinitionBody::Model(Model::Route(route)) = &definition.body else {
        return;
    };
    let members = route.route.len();
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            coder.model.span.clone(),
            format!("`{address}` is a failover route, and {subject} runs a harness"),
        )
        .with_label(
            definition.address.span.clone(),
            format!("`{address}` is defined here, with {members} members"),
        )
        .with_help(
            "the provider connection does not reach inside a harness run: the harness owns its \
             client, its auth and its own retries, and this compiler's `retry:` wraps whole runs. \
             A route here would be a failover policy with nothing to apply it to, so bind a \
             direct `model.*` and let the node's `retry:` be the ladder (grammar 8.9, 12.2, PRD \
             resolved q57 ruling d)",
        ),
    );
}

/// The harness config, in Decision D140's two tiers.
fn settings(ctx: &mut Ctx<'_>, subject: &str, coder: &Coder) {
    let harness = coder.harness.value;
    let known = table(harness);
    for setting in &coder.settings {
        let key = setting.key.value.as_str();
        let Some((_, shape)) = known.iter().find(|(name, _)| *name == key) else {
            let names: Vec<&str> = known.iter().map(|(name, _)| *name).collect();
            // Which half of D140 the key landed in. An **owned** option is
            // dropped by the adapter rather than passed, so telling its author
            // that it travels unchanged would send them looking for an option
            // the SDK never saw — and the reason it is dropped is the one worth
            // reading: the bound it would have reached around.
            let owned = reserved(harness).iter().any(|name| name == key);
            ctx.push(
                Diagnostic::warning(
                    DiagnosticCode::UnknownHarnessSetting,
                    setting.key.span.clone(),
                    format!(
                        "`{key}` is not a `harness: {}` setting this release can check, so its \
                         value is unverified",
                        harness.as_str()
                    ),
                )
                .with_label(coder.harness.span.clone(), "the harness is bound here")
                .with_optional_help(if owned {
                    Some(format!(
                        "`{key}` is an option the generated adapter owns, so it is dropped rather \
                         than passed: what it would reach around is what `workspace:`, `access:`, \
                         `env:`, `output:`, `prompt:`, `allow_tools:`, `timeout:` and `model:` \
                         state, and the bound is the node's (grammar 8.9, Decision D140)"
                    ))
                } else {
                    suggest(key, &names)
                        .map(|name| format!("did you mean `{name}`?"))
                        .or_else(|| {
                            Some(format!(
                                "it travels to the SDK unchanged, which is what keeps a harness \
                                 option usable the day the vendor ships it; the keys this release \
                                 checks are {} (grammar 8.9, Decision D140)",
                                list(&names)
                            ))
                        })
                }),
            );
            continue;
        };
        check_shape(ctx, subject, harness, &setting.key, &setting.value, shape);
    }
}

/// One curated key's value, strictly.
fn check_shape(
    ctx: &mut Ctx<'_>,
    subject: &str,
    harness: Harness,
    key: &Spanned<String>,
    value: &Spanned<Literal>,
    shape: &Shape,
) {
    let refuse = |ctx: &mut Ctx<'_>, code: DiagnosticCode, message: String, help: String| {
        ctx.push(
            Diagnostic::error(code, value.span.clone(), message)
                .with_label(key.span.clone(), "the setting is declared here")
                .with_help(help),
        );
    };
    match (shape, &value.value) {
        (Shape::Flag, Literal::Bool(_))
        | (Shape::Positive, Literal::Int(_) | Literal::Float(_)) => {}
        (Shape::Integer(low, high), Literal::Int(held)) => {
            if !(*low..=*high).contains(held) {
                refuse(
                    ctx,
                    DiagnosticCode::ValueOutOfRange,
                    format!(
                        "`{}` of {subject} is {held}, outside {low}..={high}",
                        key.value
                    ),
                    format!(
                        "`harness: {}` takes `{}` in that range (grammar 8.9, Decision D140)",
                        harness.as_str(),
                        key.value
                    ),
                );
            }
        }
        (Shape::Enum(words), Literal::String(held)) => {
            if !words.contains(&held.as_str()) {
                ctx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        value.span.clone(),
                        format!("`{held}` is not a `{}` of {subject}", key.value),
                    )
                    .with_label(key.span.clone(), "the setting is declared here")
                    .with_optional_help(
                        suggest(held, words)
                            .map(|name| format!("did you mean `{name}`?"))
                            .or_else(|| Some(format!("`{}` takes {}", key.value, list(*words)))),
                    ),
                );
            }
        }
        (Shape::Names, Literal::Sequence(items)) => {
            for item in items {
                let empty = match &item.value {
                    Literal::String(held) => held.is_empty(),
                    _ => true,
                };
                if empty {
                    ctx.push(
                        Diagnostic::error(
                            DiagnosticCode::WrongType,
                            item.span.clone(),
                            format!("every entry of `{}` of {subject} is a tool name", key.value),
                        )
                        .with_label(key.span.clone(), "the setting is declared here")
                        .with_help("a tool name is a non-empty string (grammar 8.9)"),
                    );
                }
            }
        }
        _ => {
            refuse(
                ctx,
                DiagnosticCode::WrongType,
                format!(
                    "`{}` of {subject} takes {}, found {}",
                    key.value,
                    shape.expectation(),
                    value.value.description()
                ),
                format!(
                    "`harness: {}` maps this key onto an SDK option of that type (grammar 8.9, \
                     Decision D140)",
                    harness.as_str()
                ),
            );
        }
    }
    // A budget of zero is a run that can make no call at all, which is a
    // configuration nobody writes on purpose: the key exists to bound spend, and
    // `0` silences the node rather than bounding it.
    if matches!(shape, Shape::Positive) {
        let held = match &value.value {
            Literal::Int(held) => Some(*held as f64),
            Literal::Float(held) => Some(*held),
            _ => None,
        };
        if let Some(held) = held
            && held <= 0.0
        {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::ValueOutOfRange,
                    value.span.clone(),
                    format!("`{}` of {subject} is not above zero", key.value),
                )
                .with_label(key.span.clone(), "the setting is declared here")
                .with_help(
                    "a budget of zero stops the run before its first call rather than bounding \
                     what it may spend; omit the key to leave the run unbounded (grammar 8.9)",
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Harness, reserved, table};

    /// The **reserved** list a warning reads is the one the adapter applies.
    ///
    /// [`reserved`](super::reserved) parses the driver's own declaration rather
    /// than restating it, so what can go wrong is the parse: a list read as
    /// empty would make every dropped key claim it travelled to the SDK, which
    /// is the one thing the reading exists to prevent. Two of the options that
    /// reach furthest are named here, plus the closed harness's own, so a parse
    /// that silently found nothing fails.
    #[test]
    fn a_warning_reads_the_reserved_list_the_adapter_applies() {
        let cc = reserved(Harness::Cc);
        for option in ["cwd", "extraArgs", "settings", "resume"] {
            assert!(
                cc.iter().any(|held| held == option),
                "`CC_RESERVED` is read without `{option}`, so a `settings:` key spelling it would \
                 be warned about as one that travels to the SDK"
            );
        }
        let codex = reserved(Harness::Codex);
        for option in ["sandboxMode", "workingDirectory", "approvalPolicy"] {
            assert!(
                codex.iter().any(|held| held == option),
                "`CODEX_RESERVED` is read without `{option}`"
            );
        }
        for held in [Harness::DeepAgents, Harness::Native] {
            assert!(
                reserved(held).is_empty(),
                "a reserved harness has no driver to read a list off"
            );
        }
    }

    /// The names one emitted `<HARNESS>_SETTINGS` array lists, in order.
    fn emitted(module: &str, declaration: &str) -> Vec<String> {
        let array = module
            .split_once(&format!("const {declaration}: readonly string[] = ["))
            .unwrap_or_else(|| panic!("the emitted module declares `{declaration}`"))
            .1;
        let array = &array[..array.find("];").expect("…and closes the array it opened")];
        array
            .split(',')
            .map(|piece| piece.trim().trim_matches('"').to_string())
            .filter(|piece| !piece.is_empty())
            .collect()
    }

    /// **The two curated `settings:` tables are one table.**
    ///
    /// [`CC_SETTINGS`](super::CC_SETTINGS) and
    /// [`CODEX_SETTINGS`](super::CODEX_SETTINGS) here are the keys `validate`
    /// checks strictly (Decision D140's first tier). `CC_SETTINGS` and
    /// `CODEX_SETTINGS` in `src/harness-cc.ts` and `src/harness-codex.ts` are
    /// those same two lists declared again for the runtime, where `passthrough`
    /// reads them to hold a curated key back from the SDK because the driver
    /// beside it maps that key by name. Two hand-maintained copies of one
    /// document drift in silence — the argument
    /// `the_harness_model_settings_table_is_one_table` in `src/codegen/graph.rs`
    /// is written under, put on the other pair.
    ///
    /// Read in **three** directions, because a curated key has to be in three
    /// places and a lapse in any one of them is silent:
    ///
    ///  1. every key this compiler checks strictly is in the emitted list. Drop
    ///     it there and `passthrough` stops holding the key back, so the value
    ///     travels to the SDK *beside* the option the driver already set from
    ///     it — two spellings of one option, which is the collision the curated
    ///     list exists to prevent.
    ///  2. the emitted list holds no key this compiler does not check. That is
    ///     the direction which makes `validate` **lie**: a key the runtime
    ///     treats as curated and this table never learned earns
    ///     `unknown-harness-setting` — "its value is unverified" — while the
    ///     driver reads it and applies it all the same.
    ///  3. each key is really read by that harness's driver, which is the whole
    ///     reason either list exists: a key checked strictly here, filtered out
    ///     of the passthrough there, and mapped by nobody is a setting
    ///     `validate` verifies and the run then silently drops.
    #[test]
    fn the_curated_settings_table_is_one_table() {
        for (harness, declaration, driver, module) in [
            (
                Harness::Cc,
                "CC_SETTINGS",
                include_str!("../codegen/js/harness-cc.ts"),
                "src/harness-cc.ts",
            ),
            (
                Harness::Codex,
                "CODEX_SETTINGS",
                include_str!("../codegen/js/harness-codex.ts"),
                "src/harness-codex.ts",
            ),
        ] {
            let checked: Vec<String> = table(harness)
                .iter()
                .map(|(key, _)| (*key).to_string())
                .collect();
            assert_eq!(
                emitted(driver, declaration),
                checked,
                "`{declaration}` in `{module}` and `{declaration}` in `src/check/coder.rs` are \
                 two copies of one table and have parted company: a key in one and not the other \
                 is either a setting `validate` checks and the passthrough duplicates, or one it \
                 warns is unverified while the driver applies it"
            );
            for key in &checked {
                assert!(
                    driver.contains(&format!("run.settings[\"{key}\"]")),
                    "`{key}` is a curated `harness: {}` setting and `{module}` never reads it, so \
                     the value is checked strictly, filtered out of the passthrough, and applied \
                     to nothing",
                    harness.as_str()
                );
            }
        }

        // A reserved harness has no driver and no curated table, which is the
        // answer that claims least: `validate` refuses the composition before a
        // settings key of one is ever read.
        for reserved in [Harness::DeepAgents, Harness::Native] {
            assert!(
                table(reserved).is_empty(),
                "a reserved harness has no SDK to take a setting"
            );
        }
    }
}
