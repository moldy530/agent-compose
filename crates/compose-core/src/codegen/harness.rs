//! `src/harness.ts`: the drivers the adapter reaches an SDK through
//! (grammar 8.9, PRD resolved q57).
//!
//! [`super::runtime`] holds the **adapter** — the config map, the journal, the
//! stream tap and the output gate — and this module emits the other side of the
//! seam it drives: one `runtime.HarnessDriver` per harness some `coder:` node
//! binds, each a thin mapping over the vendor's own SDK, plus a scripted driver
//! that maps over nothing.
//!
//! # Why this one is not a constant
//!
//! Every other module of the same shape ([`super::runtime`], [`super::stores`])
//! is byte-identical in every project a compiler release builds, and this one
//! deliberately is not: PRD resolved q57 says **emit only the drivers a
//! composition uses**, and a driver is an `import` of an SDK. A project with no
//! coder node would otherwise carry an import of a package its `package.json`
//! does not pin — gate 14 of `tests/generated_code_gates.rs` refuses exactly
//! that — and a project that binds `cc` alone would load the Codex CLI for
//! nothing. [`super::project::dependencies`] reads the same set, so the pins and
//! the imports cannot disagree.
//!
//! The **file** is emitted for every composition all the same, because
//! [`super::EMITTED_PATHS`] is the compiler's whole claim on an output directory
//! and a path that came and went with a node kind would make that claim depend
//! on the composition. What varies is its contents: with no coder node it is the
//! prelude, the scripted driver, and an empty registry.
//!
//! # Where the versions come from
//!
//! [`HARNESS_PINS`], which is the same table [`super::project`] writes into the
//! generated `package.json`. A driver reports its own `sdk@version` into the
//! trace's [`HarnessRecord`], so the number a reader sees in a trace is the
//! number the manifest pinned — held together by
//! `the_emitted_driver_reports_the_version_the_manifest_pins`.
//!
//! [`HarnessRecord`]: https://docs.rs/ "docs/trace.md §7.6"

use std::collections::BTreeSet;

use crate::ast::flow::Harness;
use crate::ir::Ir;
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::NodeKind;

use super::names;

/// The prelude: the doc comment, the scripted driver, and the shared helpers.
const PRELUDE: &str = include_str!("js/harness.ts");

/// The `cc` driver.
const CC: &str = include_str!("js/harness-cc.ts");

/// The `codex` driver.
const CODEX: &str = include_str!("js/harness-codex.ts");

/// The SDK each harness is reached through, pinned exactly (PRD 5.12).
///
/// The same discipline [`super::project::PINS`] is under and for the same
/// reason: a harness SDK is what a coder node's whole behaviour is, so a release
/// that let one float would change what a compiled graph does with no commit
/// saying so. Pinned **per harness** rather than in the project-wide list
/// because a composition that binds neither harness declares neither package.
///
/// Each entry is the package, the version, and the peer dependencies that
/// package's own types need resolved — pinned here for the reason
/// `@langchain/core` is pinned beside `@langchain/langgraph`: a peer left to the
/// installer is a peer two projects built by one compiler release can resolve
/// differently.
pub const HARNESS_PINS: &[(Harness, &[(&str, &str)])] = &[
    (
        Harness::Cc,
        &[
            ("@anthropic-ai/claude-agent-sdk", "0.3.272"),
            // The Agent SDK's own peers: its `.d.ts` imports message types from
            // the first and MCP types from the second, so a project that
            // installed neither would fail `tsc` on an import it never wrote.
            ("@anthropic-ai/sdk", "0.126.0"),
            ("@modelcontextprotocol/sdk", "1.30.0"),
        ],
    ),
    (
        Harness::Codex,
        &[
            ("@openai/codex-sdk", "0.154.0"),
            // The Codex SDK's `index.d.ts` imports `ContentBlock` from the MCP
            // SDK, which its own manifest declares only as a dev dependency —
            // so a consumer that did not pin it cannot type-check.
            ("@modelcontextprotocol/sdk", "1.30.0"),
        ],
    ),
];

/// The packages one harness's driver brings, pinned.
#[must_use]
pub fn pins_of(harness: Harness) -> &'static [(&'static str, &'static str)] {
    HARNESS_PINS
        .iter()
        .find(|(held, _)| *held == harness)
        .map_or(&[], |(_, pins)| *pins)
}

/// The version this release pins one harness's own SDK to.
#[must_use]
pub fn version_of(harness: Harness) -> &'static str {
    pins_of(harness).first().map_or("", |(_, version)| *version)
}

/// Every harness some `coder:` node of this composition binds, sorted.
///
/// Sorted by the enum's own order rather than by first use, for
/// [`super::EMITTED_PATHS`]' reason one level down: what a project imports has
/// to be a function of *what* it binds rather than of which flow bound it first,
/// or moving a node between two files would rewrite this module.
#[must_use]
pub fn bound(ir: &Ir) -> Vec<Harness> {
    let mut held: BTreeSet<Harness> = BTreeSet::new();
    for definition in ir.definitions.values() {
        let DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        for node in &flow.nodes {
            if let NodeKind::Coder { coder } = &node.kind {
                held.insert(coder.harness.value);
            }
        }
    }
    held.into_iter().collect()
}

/// `src/harness.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let harnesses = bound(ir);
    let mut contents = super::header(ir, "// ");

    contents.push_str("\nimport * as runtime from \"./runtime.ts\";\n");
    if harnesses.contains(&Harness::Cc) {
        contents.push_str("import { query } from \"@anthropic-ai/claude-agent-sdk\";\n");
        contents.push_str(
            "import type {\n  Options,\n  PermissionMode,\n  SDKMessage,\n} from \"@anthropic-ai/claude-agent-sdk\";\n",
        );
    }
    if harnesses.contains(&Harness::Codex) {
        contents.push_str("import { Codex } from \"@openai/codex-sdk\";\n");
        contents.push_str(
            "import type {\n  CodexOptions,\n  SandboxMode,\n  ThreadItem,\n  ThreadOptions,\n} from \"@openai/codex-sdk\";\n",
        );
    }
    contents.push_str(PRELUDE);

    for harness in &harnesses {
        contents.push('\n');
        // The version a driver reports into the trace is the one the manifest
        // pins, emitted here rather than written into the TypeScript so the two
        // cannot drift (see the module header).
        contents.push_str(&names::doc(
            "",
            &[format!(
                "The `{}` SDK version this compiler release pins — the number \
                 `package.json` declares and the number a `HarnessRecord` reports.",
                harness.as_str()
            )],
        ));
        contents.push_str(&format!(
            "const {}: string = {};\n\n",
            version_constant(*harness),
            names::string(version_of(*harness))
        ));
        contents.push_str(match harness {
            Harness::Cc => CC,
            Harness::Codex => CODEX,
            Harness::DeepAgents | Harness::Native => unreachable!(
                "`validate` refuses a reserved harness, so no composition reaches codegen with one"
            ),
        });
    }

    contents.push('\n');
    contents.push_str(&names::doc(
        "",
        &[
            "The drivers this composition uses, by harness — what `runtime.runCoder` resolves \
             a `coder:` node's harness against."
                .to_string(),
            String::new(),
            "A harness with no driver here is one no node binds, and a run that reached one \
             would be `runtime.HarnessUnavailable`: the adapter refuses at the call rather \
             than dispatching into nothing."
                .to_string(),
        ],
    ));
    if harnesses.is_empty() {
        contents.push_str("export const DRIVERS: runtime.HarnessDrivers = {};\n");
    } else {
        contents.push_str("export const DRIVERS: runtime.HarnessDrivers = {\n");
        for harness in &harnesses {
            contents.push_str(&format!(
                "  {}: {},\n",
                harness.as_str(),
                driver_constant(*harness)
            ));
        }
        contents.push_str("};\n");
    }

    super::GeneratedFile {
        path: "src/harness.ts".to_string(),
        contents,
    }
}

/// The name of the emitted constant holding one harness's pinned SDK version.
fn version_constant(harness: Harness) -> &'static str {
    match harness {
        Harness::Cc => "CC_SDK_VERSION",
        Harness::Codex => "CODEX_SDK_VERSION",
        Harness::DeepAgents | Harness::Native => "",
    }
}

/// …and of the driver itself.
fn driver_constant(harness: Harness) -> &'static str {
    match harness {
        Harness::Cc => "CC_DRIVER",
        Harness::Codex => "CODEX_DRIVER",
        Harness::DeepAgents | Harness::Native => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    /// A composition with no coder node still gets the file, and it names no SDK.
    ///
    /// The half of "emit only the drivers a composition uses" that a golden
    /// cannot show, because a golden of a composition with no coder node is
    /// exactly the project this is about: the claim is about what is *absent*.
    #[test]
    fn a_composition_with_no_coder_node_imports_no_sdk() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.contains("export const DRIVERS: runtime.HarnessDrivers = {};"));
        for (_, pins) in HARNESS_PINS {
            for (package, _) in *pins {
                assert!(
                    !emitted.contains(package),
                    "a composition with no coder node names `{package}`"
                );
            }
        }
        assert!(
            emitted.contains("export function scriptedDriver("),
            "the scripted driver is emitted into every project"
        );
    }

    /// Every SDK option a driver sets from the **node's own bounds** is on that
    /// driver's reserved list, so `settings:` cannot reach around one.
    ///
    /// Decision D140 leaves `settings:` open on purpose — a harness option the
    /// vendor ships tomorrow has to be usable the day it ships — and the
    /// unverified keys therefore travel to the SDK unchanged. The bound on that
    /// is the reserved list: a key spelling the working directory, the
    /// permission mode or sandbox preset, the environment, the output schema,
    /// the tool allowlist, the abort signal or the model is dropped, because
    /// each of those is what `workspace:`, `access:`, `env:`, `output:`,
    /// `allow_tools:`, `timeout:` and `model:` say.
    ///
    /// A hand-written list goes stale the first time a driver learns a new
    /// option, and *stale* here means the one open surface silently becomes the
    /// way around a closed one — `access: read_only` with a `settings:` key
    /// spelling the SDK's own permission field would run unbounded, and no
    /// check in this compiler would have an opinion. So the list is checked
    /// against the driver rather than trusted: every option assigned from
    /// anything but `run.settings` has to be reserved, which makes a new
    /// adapter-owned option a failing test rather than a hole.
    #[test]
    fn a_settings_key_cannot_reach_an_option_the_adapter_owns() {
        for harness in Harness::ALL.iter().filter(|held| held.ships_in_v1()) {
            let (source, curated, reserved) = driver_source(*harness);
            let name = harness.as_str();
            assert!(
                source.contains(&format!("passthrough(run, {curated}, {reserved})")),
                "`{name}`'s driver does not hand `passthrough` its reserved list"
            );
            let held = quoted_list(source, reserved);
            assert!(!held.is_empty(), "`{name}`'s reserved list is empty");
            for option in adapter_owned(source) {
                assert!(
                    held.contains(&option),
                    "`{name}`'s driver sets `{option}` from the node's own bounds, and \
                     `{reserved}` does not hold it — an unverified `settings:` key spelling \
                     `{option}` would override it (grammar 8.9, Decision D140)"
                );
            }
        }
    }

    /// …and the other direction, which the one above cannot see: the reserved
    /// list is an **inventory of the pinned SDK's option surface**, re-opened
    /// by the pin (grammar 8.9, Decision D140).
    ///
    /// [`a_settings_key_cannot_reach_an_option_the_adapter_owns`] reads the
    /// list off the driver, and that reading has a floor: an option the driver
    /// never *assigns* is one it can never require. The options that reach
    /// furthest are exactly those — an Agent SDK run takes `extraArgs`, which
    /// is any CLI flag there is, `settings` and `settingSources`, which are
    /// permission rules, `mcpServers` and `agents`, which are tools and loops
    /// outside `allow_tools:`, and the `resume` family PRD resolved q57 ruling
    /// b excludes by name — and a driver that assigns none of them would leave
    /// every one of them passable. So the list is written out here as well,
    /// against the version it was read against: a vendor's new option arrives
    /// with a version bump, so the bump is what fails this test and re-opens
    /// the audit rather than quietly widening the surface.
    #[test]
    fn a_reserved_list_is_audited_against_the_pinned_option_surface() {
        for harness in Harness::ALL.iter().filter(|held| held.ships_in_v1()) {
            let (source, _, reserved) = driver_source(*harness);
            let (audited, expected) = audited_surface(*harness);
            let name = harness.as_str();
            assert_eq!(
                version_of(*harness),
                audited,
                "`{name}`'s SDK is pinned at {} and `{reserved}` was audited against {audited}: \
                 read the release's own options, add every one that reaches around \
                 `workspace:`, `access:`, `env:`, `output:`, `prompt:`, `allow_tools:`, \
                 `timeout:` or `model:`, and move this version up (grammar 8.9)",
                version_of(*harness)
            );
            let held: Vec<String> = quoted_list(source, reserved).into_iter().collect();
            let want: Vec<String> = expected.iter().map(|held| (*held).to_string()).collect();
            assert_eq!(
                held, want,
                "`{reserved}` is not the audited inventory of {audited}'s option surface"
            );
        }
    }

    /// …and the one option both audits above could each have let through, held
    /// to the sentence that names it: **roots beside the working directory are
    /// dropped, under every harness** (grammar 8.9, `explain
    /// unknown-harness-setting`).
    ///
    /// It is the option that made the wider half of the dropped set worth
    /// writing down, and neither test above can see it slip. The first reads
    /// the driver, and a driver that assigns the option *from a settings key*
    /// is assigning it from `run.settings` — which is exactly what that test
    /// treats as settings-derived and exempts. The second compares one
    /// hand-written inventory per harness, so a name missing from the driver
    /// and from the inventory together is two halves of one omission agreeing.
    ///
    /// What the omission costs is the containment statement itself: a node
    /// whose `workspace:` named one directory, under an `access:` preset that
    /// bounds writes to it, handed the SDK a second writable root the
    /// composition never wrote. `cc` drops it and says so in the warning; a
    /// harness that promoted it to a checked key would make `access:` mean one
    /// thing under one harness and another under the other, which PRD resolved
    /// q57 ruling c's "stated per harness, never implied equivalent" is about
    /// the *enforcement* of, not an invitation to differ about the bound.
    #[test]
    fn roots_beside_the_workspace_are_dropped_under_every_harness() {
        for harness in Harness::ALL.iter().filter(|held| held.ships_in_v1()) {
            let (source, curated, reserved) = driver_source(*harness);
            let name = harness.as_str();
            assert!(
                quoted_list(source, reserved).contains("additionalDirectories"),
                "`{reserved}` does not hold `additionalDirectories`, so a `settings:` key on a \
                 `harness: {name}` node reaches the SDK with sandbox roots beside `workspace:` \
                 (grammar 8.9's dropped set)"
            );
            for held in quoted_list(source, curated) {
                assert!(
                    !held
                        .replace('_', "")
                        .eq_ignore_ascii_case("additionaldirectories"),
                    "`{curated}` holds `{held}`, which is the root-widening option grammar 8.9 \
                     says is dropped rather than checked"
                );
            }
        }
    }

    /// …and the option family that reaches furthest of all: **what program the
    /// harness is, and what its runtime loads before it, are not a `settings:`
    /// key's to choose** (grammar 8.9, PRD resolved q57 ruling c).
    ///
    /// Written out by name for [`roots_beside_the_workspace_are_dropped_under_every_harness`]'s
    /// reason, one turn sharper. Every other reserved name is a bound *inside*
    /// the run — a root, a mode, a tool set — and the two audits above can at
    /// least argue about it from the driver. This family is the run's own
    /// executable: `pathToClaudeCodeExecutable` is which binary is spawned,
    /// `executable` is the JavaScript runtime that spawns it, `executableArgs`
    /// is what that runtime is handed first (`--import` among them), and
    /// `spawnClaudeCodeProcess` is a function the SDK calls **instead of** the
    /// default local spawn — the same reach by the shortest route. A key here
    /// does not widen a bound — it replaces or re-arms the
    /// program that *enforces* every bound, so `options.tools`,
    /// `options.canUseTool` and `permissionMode` would be asked of something
    /// else entirely while `enforcesTools`, the graph document's
    /// `tools_enforced` and grammar 8.9 all went on saying the list holds.
    ///
    /// Neither audit above can see it slip, which is why it is a sentence of
    /// its own: an adapter never *assigns* these — it wants the SDK's own
    /// executable — so a list read off the driver can never require them, and a
    /// hand-written inventory that forgot them agrees with a driver that never
    /// mentioned them. That is the two-halves-of-one-omission failure
    /// [`a_reserved_list_is_audited_against_the_pinned_option_surface`]'s own
    /// doc comment warns about, so this is the half that does not depend on
    /// either.
    #[test]
    fn what_program_a_harness_run_is_cannot_be_chosen_by_a_settings_key() {
        // Per harness, the options of its pinned SDK that choose the program a
        // run is or what its runtime loads first. A vendor's names, so they are
        // written where the reason for them is; a harness whose SDK has none —
        // `codex` spawns its own CLI and takes no such option in
        // `ThreadOptions` — carries an empty row and says so.
        const SPAWN: &[(Harness, &[&str])] = &[
            (
                Harness::Cc,
                &[
                    "pathToClaudeCodeExecutable",
                    "executable",
                    "executableArgs",
                    "spawnClaudeCodeProcess",
                ],
            ),
            (Harness::Codex, &[]),
        ];

        for harness in Harness::ALL
            .iter()
            .copied()
            .filter(|held| held.ships_in_v1())
        {
            let (source, curated, reserved) = driver_source(harness);
            let name = harness.as_str();
            let (_, spawn) = SPAWN
                .iter()
                .find(|(held, _)| *held == harness)
                .expect("every harness that ships names its SDK's process-spawn options");
            let held = quoted_list(source, reserved);
            for option in *spawn {
                assert!(
                    held.contains(*option),
                    "`{reserved}` does not hold `{option}`, so a `settings:` key on a \
                     `harness: {name}` node chooses what program the run is — and `tools`, \
                     `canUseTool` and `permissionMode` would then bound a program the \
                     composition never named (grammar 8.9, PRD resolved q57 ruling c)"
                );
                assert!(
                    !quoted_list(source, curated).contains(*option),
                    "`{curated}` holds `{option}`, which is not a harness option to check but \
                     the harness itself"
                );
            }
        }

        // …and `codex`'s empty row above is true for a reason worth holding:
        // that SDK's spawn surface is on its **client** (`codexPathOverride`,
        // and the `--config` overrides beside it) rather than on a thread's
        // options, so a `settings:` key cannot reach it — the driver builds that
        // object from a literal and `passthrough` feeds the thread alone. The
        // day it were built from `run.settings`, the row would silently stop
        // being empty.
        let (codex, _, _) = driver_source(Harness::Codex);
        let constructed = codex
            .split_once("new Codex(")
            .expect("the `codex` driver constructs its client")
            .1;
        let constructed = &constructed[..constructed.find(';').unwrap_or(constructed.len())];
        assert!(
            !constructed.contains("run.settings") && !constructed.contains("passthrough"),
            "the `codex` client is built from the node's `settings:`, so a key spelling \
             `codexPathOverride` would choose what program the run is (grammar 8.9)"
        );
        // …and through the **builder** the construction calls, because Decision
        // D143 put the connection on that object and the call is now an
        // indirection: a guard that read the `new Codex(…)` expression alone
        // would be satisfied by any function name at all, whatever that function
        // then read.
        let body = codex
            .split_once("export function codexClient(")
            .expect("the `codex` client is built by `codexClient`")
            .1;
        let body = &body[..body.find("\n}\n").unwrap_or(body.len())];
        assert!(
            !body.contains("run.settings") && !body.contains("passthrough"),
            "`codexClient` reads the node's `settings:`, so an unverified key spelling \
             `codexPathOverride`, `config` or `env` would choose what program the run is and what \
             it may reach (grammar 8.9, Decisions D140, D143)"
        );
        // …and through the one function `codexClient` itself calls, for the same
        // reason the builder is read rather than the `new Codex(…)` expression:
        // the environment that object carries is assembled there, and an
        // indirection a guard stops at is an indirection the guard does not
        // cover.
        let environment = codex
            .split_once("function codexEnvironment(")
            .expect("the `codex` client's environment is assembled by `codexEnvironment`")
            .1;
        let environment = &environment[..environment.find("\n}\n").unwrap_or(environment.len())];
        assert!(
            !environment.contains("run.settings") && !environment.contains("passthrough"),
            "`codexEnvironment` reads the node's `settings:`, so an unverified key would decide \
             what the spawned CLI's environment holds (grammar 8.9, Decisions D140, D143)"
        );
    }

    /// A `cc` run keeps the **harness's own** system prompt and appends the
    /// node's `prompt:` to it (grammar 8.9, PRD resolved q57).
    ///
    /// The SDK reads a bare string there as a *custom* prompt and replaces the
    /// preset with it, and the preset is the vendor's own agent instructions —
    /// the thing PRD resolved q57 gives as the reason this is a kind rather
    /// than a bundle of parts, and the thing `codex` keeps for free because its
    /// instructions ride the turn. A composition's `prompt:` reads the same
    /// under both harnesses only if this mapping does.
    ///
    /// Pinned on the emitted source rather than on a run, because what it is
    /// about is the option the SDK is handed: the scripted driver a runtime
    /// test drives stands in for the SDK and never sees it.
    #[test]
    fn a_cc_run_appends_the_nodes_prompt_to_the_harnesss_own() {
        assert!(
            CC.contains(
                "systemPrompt: { type: \"preset\", preset: \"claude_code\", append: run.instructions }"
            ),
            "the `cc` driver does not ask for the harness's own system prompt with the node's \
             `prompt:` appended"
        );
        assert!(
            !CC.contains("systemPrompt: run.instructions"),
            "the `cc` driver hands the SDK a bare `systemPrompt`, which replaces the preset the \
             model was post-trained against (grammar 8.9)"
        );
    }

    /// **The three options that answer to a `cc` run's approval mode read the
    /// resolved mode, not the `access:` preset** (grammar 8.9, Decision D146,
    /// PRD resolved q60 ruling a).
    ///
    /// `generated_code_gates`' `a_harness_run_is_contained_journaled_and_recorded`
    /// drives this end to end and pins what each of the six combinations is
    /// handed; this is the source-reading half, and it is here for the reason the
    /// spawn-family guard is: it names the shape that is wrong rather than the
    /// values that are right, so a driver that reverts to keying a flag off
    /// `run.access` fails on the line itself rather than on one case somebody
    /// forgot to add.
    ///
    /// The wrong shape is the one this driver shipped while `access:` was the
    /// only axis, and it reads as an obvious simplification: arm
    /// `allowDangerouslySkipPermissions` when the preset is `full_access`, fill
    /// `planModeInstructions` when it is `read_only`. Under a stated mode both
    /// are wrong in the same direction — a `full_access` node that asked for
    /// `plan` would be handed the flag for a mode it is not under and the
    /// vendor's default code-implementation body instead of its own `prompt:`.
    #[test]
    fn a_cc_runs_mode_decides_its_mode_bound_options_rather_than_its_access_preset() {
        assert!(
            CC.contains("return run.permissionMode ?? CC_PERMISSION[run.access];"),
            "the `cc` driver does not resolve `permission_mode:` against the mode its `access:` \
             level derives, so either a stated mode reaches nothing or an absent one changed \
             meaning (grammar 8.9, Decision D146)"
        );
        for (option, guard) in [
            ("allowDangerouslySkipPermissions", "bypassPermissions"),
            ("planModeInstructions", "plan"),
        ] {
            assert!(
                CC.contains(&format!("if (mode === \"{guard}\") options.{option} =")),
                "`options.{option}` is not set from the run's resolved mode: the SDK ties it to \
                 `{guard}` and this driver would be tying it to an `access:` preset, which a \
                 stated `permission_mode:` supersedes (grammar 8.9, Decision D146)"
            );
        }
        assert!(
            !CC.contains("if (run.access === "),
            "the `cc` driver still branches on `run.access` for an option its resolved mode \
             decides — `access:` chooses the *default* mode and nothing else (Decision D146)"
        );
    }

    /// A call the `cc` permission surface denied is **one** tool event, and a
    /// `"refused"` one (`docs/trace.md` §7.6.3).
    ///
    /// The denial is taped `refused` where the driver learns of it — in the
    /// permission callback, or off the `permission_denied` frame the SDK reports
    /// a denial it made **without** asking the callback (a `dontAsk` mode
    /// denial, `auto`'s classifier) — and the SDK then hands the model that same
    /// denial as the `tool_result` answering the `tool_use`. Taping that too
    /// would put a second event in the record for one call, under an outcome
    /// that describes a call which ran — `completed` is "handed its model the
    /// tool's result" and `failed` is an execution that failed, and a refusal
    /// is neither. Correlating them needs the id, so the id is what is pinned
    /// here, at both places a denial is learned of. What the taping produces
    /// for each shape of message is `generated_code_gates`'
    /// `a_harness_run_is_contained_journaled_and_recorded`, which feeds the
    /// real `ccEvents` the SDK's own frames.
    #[test]
    fn a_cc_call_the_allowlist_denied_is_one_tool_event() {
        assert!(
            CC.contains("refused.add(ask.toolUseID);"),
            "the `cc` permission callback does not remember which call it denied, so nothing \
             downstream can tell the denial's own `tool_result` from a tool's answer"
        );
        assert!(
            CC.contains(
                "if (message.type === \"system\" && message.subtype === \"permission_denied\") {"
            ) && CC.contains("refused.add(message.tool_use_id);"),
            "the `cc` driver does not tape the SDK's `permission_denied` frame, so a denial the SDK \
             made without the callback — every one under `dontAsk`, and `auto`'s classifier — \
             reaches the trace as its error `tool_result`, a `failed` call that never executed"
        );
        assert!(
            CC.contains("if (refused.has(block.tool_use_id)) {"),
            "the `cc` driver tapes a `tool_result` for a call it already taped as `refused`"
        );
    }

    /// `allow_tools:` on a `codex` node reaches the run (grammar 8.9, Decision
    /// D138).
    ///
    /// This harness bounds at the sandbox, so the list is what it is *offered*
    /// — and offering is something the driver does. A `ThreadOptions` has no
    /// tool-set field, so the one place a list can reach the harness is the
    /// turn; a driver that put it nowhere would make D138's own argument for
    /// accepting the key on this harness ("the list is still what the harness
    /// is offered") an argument for a key with no effect at all.
    #[test]
    fn a_codex_run_is_offered_the_list_that_does_not_bound_it() {
        assert!(
            CODEX.contains("runStreamed(codexTurn(run)"),
            "the `codex` driver does not build its turn through `codexTurn`"
        );
        assert!(
            CODEX.contains("run.allowTools.join(\", \")"),
            "`codexTurn` does not name the offered tools, so `allow_tools:` reaches nothing on a \
             `codex` node"
        );
    }

    /// **The connection table and the drivers are one table** (grammar 8.9,
    /// Decision D143, PRD resolved q58).
    ///
    /// `crate::harness::CONNECTION` is what `validate` refuses a composition on
    /// and what the graph document draws; the driver below it is what actually
    /// calls the SDK. Two hand-maintained accounts of one mapping drift in
    /// silence, and this one drifts in the worst direction available: the table
    /// says a fact crosses, `validate` accepts the composition on that basis, the
    /// driver maps nothing, and the run reaches the vendor endpoint the gateway
    /// deployment existed to avoid — with no diagnostic anywhere, because every
    /// surface that could have complained was told the fact was carried.
    ///
    /// Read in both directions, because each miss is a different lie:
    ///
    ///  1. a fact the row gives a slot is **read** by that driver, and the slot's
    ///     own name appears in it. A row that promised a carry nothing performs
    ///     is the failure above;
    ///  2. a fact the row gives **no** slot is not read at all. A driver that
    ///     mapped one anyway would be carrying a fact `validate` refuses the
    ///     composition over, so the error would be a compile error about a
    ///     capability the release has — and the honest narrowness Decision D143
    ///     insists on would be a lie in the other direction.
    #[test]
    fn a_driver_maps_exactly_the_connection_facts_its_row_names() {
        use crate::harness::{ConnectionFact, Slot, slot_of};

        for harness in Harness::ALL
            .iter()
            .copied()
            .filter(|held| held.ships_in_v1())
        {
            let (source, _, _) = driver_source(harness);
            let name = harness.as_str();
            for fact in ConnectionFact::ALL.iter().copied() {
                // The field of `runtime.HarnessConnection` this fact resolves
                // into — the one name both sides of the seam agree on.
                let field = match fact {
                    ConnectionFact::BaseUrl => "connection.baseUrl",
                    ConnectionFact::Credential => "connection.credential",
                    ConnectionFact::Headers => "connection.headers",
                };
                let reads = source.contains(field);
                let Some(slot) = slot_of(harness, fact) else {
                    assert!(
                        !reads,
                        "`{name}`'s driver reads `{field}`, and its connection row says this \
                         harness has no slot for `{}:` — so `validate` refuses a composition \
                         over a fact the driver would in fact have carried",
                        fact.as_str()
                    );
                    continue;
                };
                assert!(
                    reads,
                    "`{name}`'s connection row gives `{}:` a slot and its driver never reads \
                     `{field}`: `validate` accepts the composition and the run is pointed \
                     nowhere (grammar 8.9, Decision D143)",
                    fact.as_str()
                );
                let spelled = match slot {
                    Slot::Variable(variable) => variable,
                    Slot::Option { name, .. } => name,
                };
                assert!(
                    source.contains(spelled),
                    "`{name}`'s row maps `{}:` onto `{spelled}` and its driver never writes that \
                     name, so the two accounts of one mapping have parted company",
                    fact.as_str()
                );
            }
        }
    }

    /// The SDK release one driver's reserved list was read against, and the
    /// list itself, sorted as [`quoted_list`] returns it.
    fn audited_surface(harness: Harness) -> (&'static str, &'static [&'static str]) {
        match harness {
            // `@anthropic-ai/claude-agent-sdk`'s `Options`.
            Harness::Cc => (
                "0.3.272",
                &[
                    "abortController",
                    "additionalDirectories",
                    "agent",
                    "agents",
                    "allowDangerouslySkipPermissions",
                    "allowedTools",
                    "canUseTool",
                    "continue",
                    "cwd",
                    "env",
                    "executable",
                    "executableArgs",
                    "extraArgs",
                    "fallbackModel",
                    "forkSession",
                    "hooks",
                    "managedSettings",
                    "maxThinkingTokens",
                    "mcpServers",
                    "model",
                    "outputFormat",
                    "pathToClaudeCodeExecutable",
                    "permissionMode",
                    "permissionPromptToolName",
                    "permissionPrompts",
                    "planModeInstructions",
                    "plugins",
                    "resume",
                    "resumeDropsTurn",
                    "resumeSessionAt",
                    "sandbox",
                    "sessionId",
                    "settingSources",
                    "settings",
                    "skills",
                    "spawnClaudeCodeProcess",
                    "systemPrompt",
                    "thinking",
                    "toolAliases",
                    "tools",
                ],
            ),
            // `@openai/codex-sdk`'s `ThreadOptions`, which is closed and small:
            // six of its eleven fields are a bound this node states — five that
            // spell one and `additionalDirectories`, which contains one — and
            // the rest are the vendor's own vocabulary D140 leaves open.
            Harness::Codex => (
                "0.154.0",
                &[
                    "additionalDirectories",
                    "approvalPolicy",
                    "model",
                    "modelReasoningEffort",
                    "sandboxMode",
                    "workingDirectory",
                ],
            ),
            Harness::DeepAgents | Harness::Native => ("", &[]),
        }
    }

    /// The source of one harness's driver, and the names of the two lists the
    /// emitted `passthrough` reads.
    fn driver_source(harness: Harness) -> (&'static str, &'static str, &'static str) {
        match harness {
            Harness::Cc => (CC, "CC_SETTINGS", "CC_RESERVED"),
            Harness::Codex => (CODEX, "CODEX_SETTINGS", "CODEX_RESERVED"),
            Harness::DeepAgents | Harness::Native => ("", "", ""),
        }
    }

    /// The strings of one `readonly string[]` constant.
    fn quoted_list(source: &str, name: &str) -> BTreeSet<String> {
        let opened = format!("const {name}: readonly string[] = [");
        let Some(start) = source.find(&opened) else {
            return BTreeSet::new();
        };
        let rest = &source[start + opened.len()..];
        let body = rest.split_once("];").map_or(rest, |(body, _)| body);
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    }

    /// The SDK options one driver sets from something other than `run.settings`.
    ///
    /// Two shapes, because the drivers write both: a key of the options object
    /// literal (everything before the `...passthrough` spread), and a later
    /// `options.<key> = <value>` whose value does not come from a settings key.
    /// The second needs the local `const`s tracked, since a curated setting
    /// reaches its option through one.
    fn adapter_owned(source: &str) -> BTreeSet<String> {
        let mut from_settings: BTreeSet<&str> = BTreeSet::new();
        let mut owned: BTreeSet<String> = BTreeSet::new();
        let mut literal = false;
        for line in source.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("const options") {
                literal = true;
                continue;
            }
            if literal {
                if trimmed.starts_with("...passthrough(") {
                    literal = false;
                    continue;
                }
                if let Some((key, _)) = trimmed.split_once(':')
                    && !key.is_empty()
                    && key.chars().all(|held| held.is_ascii_alphanumeric())
                {
                    owned.insert(key.to_string());
                }
                continue;
            }
            if let Some(rest) = trimmed.strip_prefix("const ") {
                if let Some((bound, value)) = rest.split_once(" = ")
                    && value.contains("run.settings")
                {
                    from_settings.insert(bound);
                }
                continue;
            }
            let Some(at) = line.find("options.") else {
                continue;
            };
            let Some((key, value)) = line[at + "options.".len()..].split_once(" = ") else {
                continue;
            };
            if key.is_empty() || !key.chars().all(|held| held.is_ascii_alphanumeric()) {
                continue;
            }
            let settings_derived = value.contains("run.settings")
                || value
                    .split(|held: char| !held.is_ascii_alphanumeric() && held != '_')
                    .any(|word| from_settings.contains(word));
            if !settings_derived {
                owned.insert(key.to_string());
            }
        }
        owned
    }

    /// The version a driver reports is the version the manifest pins.
    ///
    /// Two surfaces read one table: `package.json` declares the pin, and the
    /// emitted driver reports `sdk@version` into every `HarnessRecord`. A reader
    /// joining a trace to a manifest compares those two strings, so they are
    /// held to one source here rather than by two constants agreeing on the day
    /// they were written.
    #[test]
    fn the_emitted_driver_reports_the_version_the_manifest_pins() {
        for (harness, pins) in HARNESS_PINS {
            let (package, version) = pins[0];
            assert_eq!(
                version_of(*harness),
                version,
                "`{}`'s own SDK is the first pin of its row",
                harness.as_str()
            );
            assert!(
                !package.is_empty() && !version.is_empty(),
                "every pin names a package at a version"
            );
        }
    }

    /// **A driver that says it enforces `allow_tools:` narrows what its loop can
    /// reach, rather than resting on a callback an `access:` preset lifts.**
    ///
    /// Four surfaces carry that claim: `HarnessDriver.enforcesTools` in the
    /// artifact, the graph document's `tools_enforced` (`docs/graph.md` §5.8),
    /// the `plan` report, and `docs/grammar.md` 8.9. PRD resolved q57 ruling c
    /// makes stating the asymmetry this compiler's job — which is worth nothing
    /// while the enforcing side does not enforce.
    ///
    /// The shape that nearly broke it is worth naming, because the next driver
    /// can repeat it: `access: full_access` lowers to the Agent SDK's
    /// `bypassPermissions`, which that SDK documents as bypassing **all**
    /// permission checks, so a run under that preset never reaches `canUseTool`.
    /// An allowlist enforced by the callback alone would be a bound four
    /// surfaces assert and the one node asking for the least containment does
    /// not hold. The answer is the SDK's own — "to restrict which tools are
    /// available, use the `tools` option instead" — so the list is also the
    /// **available set**, which no permission mode widens.
    ///
    /// Read in three directions:
    ///
    ///  1. the claim in the artifact is the claim on the page: a driver's
    ///     `enforcesTools` and the graph document's `tools_enforced` for the
    ///     same harness are one answer, not two constants that agreed once;
    ///  2. a harness that claims it sets the option that fixes the **available**
    ///     set from `run.allowTools`, which is the layer `access:` cannot reach;
    ///  3. every option it sets from the list is on that driver's reserved list,
    ///     so an unverified `settings:` key cannot hand the bound back.
    #[test]
    fn a_driver_that_enforces_the_allowlist_narrows_the_tools_it_offers() {
        // The SDK option that fixes what a loop may reach at all, per harness
        // that claims enforcement. A vendor's name, so it is written where the
        // reason for it is.
        const AVAILABLE_SET: &[(Harness, &str)] = &[(Harness::Cc, "tools")];

        for harness in Harness::ALL.iter().copied().filter(|h| h.ships_in_v1()) {
            let (source, _, reserved) = driver_source(harness);
            let name = harness.as_str();
            let claims = declared_enforcement(source);
            assert_eq!(
                claims,
                documented_enforcement(harness),
                "`{name}`'s driver says `enforcesTools: {claims}` and the graph document says \
                 otherwise — `tools_enforced` is what an author reads, and a run is what holds"
            );
            let set = from_allow_tools(source);
            for option in &set {
                assert!(
                    quoted_list(source, reserved).contains(option),
                    "`{name}`'s driver sets `{option}` from `allow_tools:` and `{reserved}` does \
                     not hold it — a `settings:` key spelling `{option}` would widen the bound"
                );
            }
            let Some((_, available)) = AVAILABLE_SET.iter().find(|(held, _)| *held == harness)
            else {
                assert!(
                    !claims,
                    "`{name}` claims to enforce `allow_tools:` and this test does not know which \
                     of its SDK's options fixes the available tool set — a claim nothing checks \
                     is how the `full_access` hole opened"
                );
                continue;
            };
            assert!(
                claims,
                "`{name}` is listed as enforcing and does not say so"
            );
            assert!(
                set.contains(*available),
                "`{name}`'s driver does not set `{available}` from `run.allowTools`, so the only \
                 thing narrowing its tools is its permission callback — which \
                 `access: full_access` bypasses outright (grammar 8.9, PRD resolved q57 ruling c)"
            );
        }
    }

    /// What one driver constant declares for `enforcesTools`.
    fn declared_enforcement(source: &str) -> bool {
        let rest = source
            .split_once("enforcesTools: ")
            .expect("every driver declares whether it enforces the allowlist")
            .1;
        let value = &rest[..rest.find(',').expect("…as one field of the object")];
        value
            .trim()
            .parse()
            .expect("…and declares it as a literal boolean")
    }

    /// **A `cc` run never pairs bare `allowedTools` with its permission
    /// callback, and carries the list's per-call answer through the option its
    /// mode reads** (grammar 8.9, Decisions D138 and D146, PRD resolved q57
    /// ruling c and q60 ruling a).
    ///
    /// `allow_tools:` reaches the Agent SDK as two layers: `tools`, the
    /// availability bound no permission mode widens, and a per-call gate whose
    /// answer is the list's own — `allow` inside it, so a bounded run neither
    /// prompts about nor denies what it allows, and `deny` outside it. Which
    /// option carries the gate is the mode's to say:
    ///
    ///  * every mode that consults a callback (`default`, `acceptEdits`,
    ///    `plan`, and `auto` once its classifier hands a question back) gets
    ///    `canUseTool`, which also tapes a denial as `"refused"` — and **no**
    ///    `allowedTools` beside it: a bare entry there approves the whole tool
    ///    before the callback is consulted, so the pinned SDK
    ///    (`@anthropic-ai/claude-agent-sdk` 0.3.272) reports the pairing from
    ///    `query()` as a shadowed callback, `CLAUDE_SDK_CAN_USE_TOOL_SHADOWED`;
    ///  * `dontAsk`, which the SDK documents as "deny if not pre-approved" and
    ///    whose CLI denies a would-ask call **without** consulting
    ///    `canUseTool`, gets the list as `allowedTools` and no callback. A
    ///    callback there is dead, and with nothing pre-approved every in-list
    ///    tool that needs a permission is denied — the regression this test
    ///    keeps out, and one a check that only asked the callback what it
    ///    answers could never see.
    ///
    /// Read off the **emitted** module rather than the driver constant, so the
    /// prelude and the `passthrough` it holds are covered too, and read as code:
    /// a comment naming the option to say why it is absent is the comment doing
    /// its job. Code may name `allowedTools` in two places — the reserved-list
    /// entry, which drops a `settings:` key spelling it, and the `dontAsk`
    /// branch — and the two gate options must be the two arms of one
    /// `mode === "dontAsk"` test, so no mode is handed both. The runtime half,
    /// mode by mode, is `generated_code_gates`'
    /// `a_harness_run_is_contained_journaled_and_recorded`, which calls the real
    /// option builder.
    #[test]
    fn bare_allowed_tools_are_never_paired_with_the_permission_callback() {
        let emitted = module(&one_coder_node(Harness::Cc)).contents;
        let code: Vec<&str> = emitted
            .lines()
            .map(str::trim)
            .filter(|line| {
                !(line.starts_with("//") || line.starts_with("/*") || line.starts_with('*'))
            })
            .collect();
        let mentions: Vec<&str> = code
            .iter()
            .copied()
            .filter(|line| {
                line.split(|held: char| !held.is_ascii_alphanumeric() && held != '_' && held != '$')
                    .any(|word| word == "allowedTools")
            })
            .collect();
        assert_eq!(
            mentions,
            ["\"allowedTools\",", "options.allowedTools = [...allowed];"],
            "the emitted `cc` driver names `allowedTools` in code outside its reserved list and its \
             `dontAsk` branch: a bare entry beside `canUseTool` approves the whole tool before the \
             callback is consulted, which the pinned SDK reports from `query()` as \
             `CLAUDE_SDK_CAN_USE_TOOL_SHADOWED` (grammar 8.9, Decision D138)"
        );
        let gate = [
            "if (mode === \"dontAsk\") {",
            "options.allowedTools = [...allowed];",
            "} else {",
            "options.canUseTool = (name, _input, ask) => {",
        ];
        assert!(
            code.windows(gate.len()).any(|window| window == gate),
            "the `cc` driver's two gate options are no longer the two arms of one \
             `mode === \"dontAsk\"` test: either some mode is handed `allowedTools` beside the \
             callback it shadows, or `dontAsk` — which denies a would-ask call without consulting \
             `canUseTool` — is handed the callback instead of the pre-approval, and every in-list \
             tool that needs a permission is denied (Decision D146)"
        );
        assert!(
            quoted_list(&emitted, "CC_RESERVED").contains("allowedTools"),
            "`CC_RESERVED` does not hold `allowedTools`, so a `settings:` key spelling it would \
             pair bare entries with the permission callback the driver sets"
        );
        assert!(
            from_allow_tools(&emitted).contains("tools"),
            "the `cc` driver no longer sets `tools` from `allow_tools:`, which is the bound \
             `access: full_access` cannot lift (PRD resolved q57 ruling c)"
        );
        assert!(
            emitted.contains(
                "if (allowed.includes(name)) return Promise.resolve({ behavior: \"allow\" as const });"
            ),
            "the `cc` permission callback no longer answers `allow` for a call inside the list, \
             so under a mode that consults it an in-list call stops to ask"
        );
    }

    /// …and what the graph document says about the same harness, which is the
    /// surface an author reads (`docs/graph.md` §5.8).
    fn documented_enforcement(harness: Harness) -> bool {
        crate::graph(&one_coder_node(harness))
            .flows
            .iter()
            .flat_map(|flow| &flow.nodes)
            .find_map(|node| node.coder.as_ref())
            .expect("the composition has a coder node")
            .tools_enforced
    }

    /// A composition whose one flow runs one `coder:` node on `harness`, under
    /// the preset that asks for the least containment and with an
    /// `allow_tools:` list — the node every enforcement claim is about.
    fn one_coder_node(harness: Harness) -> Ir {
        ir_of(&format!(
            "version: \"0.1\"
provider.p:
  kind: anthropic
  api_key: ${{K}}
model.m:
  provider: provider.p
  id: some-model
flow.f:
  outputs: {{}}
  nodes:
    build:
      coder:
        harness: {}
        model: model.m
        workspace: \"'/srv/checkout'\"
        access: full_access
        prompt: Do the work.
        output:
          summary: {{ type: string }}
        allow_tools: [Read]
      input: \"'go'\"
  edges:
    - {{ from: start, to: build }}
    - {{ from: build, to: end }}
",
            harness.as_str()
        ))
    }

    /// The option keys one driver sets from the node's `allow_tools:`.
    ///
    /// The same two shapes [`adapter_owned`] reads, narrowed to the ones whose
    /// value comes from `run.allowTools` — directly, or through the local
    /// `const` a driver binds it to first.
    fn from_allow_tools(source: &str) -> BTreeSet<String> {
        let mut bound: BTreeSet<&str> = BTreeSet::new();
        let mut set: BTreeSet<String> = BTreeSet::new();
        for line in source.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("const ")
                && let Some((held, value)) = rest.split_once(" = ")
                && value.contains("run.allowTools")
            {
                bound.insert(held);
            }
            let Some(at) = line.find("options.") else {
                continue;
            };
            let Some((key, value)) = line[at + "options.".len()..].split_once(" = ") else {
                continue;
            };
            if key.is_empty() || !key.chars().all(|held| held.is_ascii_alphanumeric()) {
                continue;
            }
            let from_list = value.contains("run.allowTools")
                || value
                    .split(|held: char| !held.is_ascii_alphanumeric() && held != '_')
                    .any(|word| bound.contains(word));
            if from_list {
                set.insert(key.to_string());
            }
        }
        set
    }
}
