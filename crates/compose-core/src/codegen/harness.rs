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
//! generated `package.json`, and which this module emits beside each driver as
//! the package it maps over and the version this release pinned it to — held
//! together by `the_emitted_driver_reports_the_version_the_manifest_pins`.
//!
//! **One exception, bounded**: a target's `harnesses:` may move a harness's own
//! SDK to another exact version inside the range this release audited it for
//! ([`AUDITED`], grammar 14.8, PRD resolved q66). Both surfaces then carry the
//! target's version instead — `package.json` in the pin's place, and the
//! constant below — through the one reading [`sdk_version`], so the manifest
//! and the fallback still cannot disagree. The SDK's peers do not move.
//!
//! What a driver reports into the trace's [`HarnessRecord`] is the version
//! **installed**, read off that package's own manifest when the module loads,
//! and the pin only where the manifest cannot be read. The two agree in every
//! project installed from the manifest this compiler wrote; where they do not,
//! the `package.json` was edited after `build` — which `build --check` reports
//! — and a trace naming the pin would have hidden the edit behind the number the
//! compiler meant rather than the one that ran.
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
            ("@anthropic-ai/claude-agent-sdk", "0.3.284"),
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

/// The package one harness's own SDK is — the first pin of its row.
#[must_use]
pub fn package_of(harness: Harness) -> &'static str {
    pins_of(harness).first().map_or("", |(package, _)| *package)
}

/// The version this release pins one harness's own SDK to.
#[must_use]
pub fn version_of(harness: Harness) -> &'static str {
    pins_of(harness).first().map_or("", |(_, version)| *version)
}

/// The SDK version each harness's reserved-list audit was performed at — the
/// **floor** of the range a target's `harnesses:` may pin that SDK inside
/// (grammar 14.8, Decision D151, PRD resolved q66 ruling b).
///
/// Each compiler release declares, per harness SDK, the half-open range
/// `[audited, next minor)`: the version the audit was performed at, forward
/// through that minor, so `0.3.284` admits `0.3.284` up to and excluding
/// `0.4.0` ([`audited_range`]). The default is narrow by design, and widening
/// it is a release decision recorded beside the audit, never a deployment's.
///
/// **What the range promises is precise and small: no *known* reach-around.**
///
///  * every option the audited version accepts was classified at the floor —
///    **both readings** of its option surface, its typed `Options` *and* the
///    names its runtime option reader takes, which is PRD resolved q60's audit
///    as re-performed on 2026-09-29 — the audit test,
///    `a_reserved_list_is_audited_against_the_pinned_option_surface`, is where
///    that inventory is written down;
///  * the driver's **other contracts were verified there** too — the shadowed
///    `canUseTool` warning a bare `allowedTools` entry raises, resolved q60's
///    permission-mode admissibility table, resolved q58's connection table;
///  * and within the SDK's own patch series the compiler **expects** those to
///    hold while stating plainly that **it has not checked them beyond the
///    floor**: a version above it is admitted on that expectation, not on an
///    audit of its own.
///
/// A version **below** the floor is refused as well. An older SDK carries a
/// subset of the audited options, but it is not the version the driver's
/// contracts were verified against, and the reason to go backward — a CLI that
/// works — is a reason to move the floor in a compiler release, not to let a
/// deployment reach below it.
///
/// The floor is not a second number that happens to agree with the audit:
/// `a_range_floor_is_the_version_its_reserved_list_was_audited_at` holds each
/// row equal to the anchor that audit test reads, so a bump of one without the
/// other fails a named test rather than widening the range silently. The pin in
/// [`HARNESS_PINS`] is held to the same anchor by the audit test itself, so in
/// every release the pin is the floor.
pub const AUDITED: &[(Harness, &str)] = &[(Harness::Cc, "0.3.284"), (Harness::Codex, "0.154.0")];

/// The version one harness's reserved-list audit was performed at, if this
/// release audited one — the floor of [`audited_range`].
#[must_use]
pub fn audited_of(harness: Harness) -> Option<&'static str> {
    AUDITED
        .iter()
        .find(|(held, _)| *held == harness)
        .map(|(_, version)| *version)
}

/// The range a target's `harnesses:` may pin one harness's SDK inside:
/// `[audited, next minor)` (grammar 14.8, PRD resolved q66 ruling b).
///
/// `None` for a harness this release does not lower, which has no SDK to pin.
#[must_use]
pub fn audited_range(harness: Harness) -> Option<AuditedRange> {
    let floor = audited_of(harness)?;
    let parsed = SdkVersion::parse(floor).expect("an audited version is an exact version");
    let minor: u64 = parsed
        .minor
        .parse()
        .expect("an audited version's minor fits in a u64");
    Some(AuditedRange {
        floor,
        ceiling: format!("{}.{}.0", parsed.major, minor + 1),
    })
}

/// One harness's audited range (see [`AUDITED`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuditedRange {
    /// The version the reserved-list audit was performed at — included.
    pub floor: &'static str,
    /// The next minor after the floor's, patch `0` — excluded.
    pub ceiling: String,
}

impl std::fmt::Display for AuditedRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}, {})", self.floor, self.ceiling)
    }
}

/// Where one exact version sits against a harness's [`audited_range`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangePlacement {
    /// Below the floor: an older release than the audit was performed at, or a
    /// prerelease of the floor itself.
    Below,
    /// Inside `[floor, next minor)`.
    Inside,
    /// At or past the next minor — a prerelease of the next minor included,
    /// since it belongs to that minor and not to the audited one.
    Above,
}

/// Where `version` sits against `harness`'s audited range, or `None` where it
/// is not an exact version or the harness has no range.
///
/// Semver precedence (semver.org §11), with one reading made explicit: the range
/// is "the audited version, forward through that minor", so a version is inside
/// it only if it **is** that minor — `0.4.0-rc.1` precedes `0.4.0` and is still
/// above `[0.3.284, 0.4.0)`, because it is a release of the minor nobody
/// audited. Build metadata carries no precedence, so `0.3.284+local` is the
/// floor.
#[must_use]
pub fn placement_of(harness: Harness, version: &str) -> Option<RangePlacement> {
    let floor = SdkVersion::parse(audited_of(harness)?)?;
    let asked = SdkVersion::parse(version)?;
    let line = |held: &SdkVersion| {
        (
            numeric(&held.major, &floor.major),
            numeric(&held.minor, &floor.minor),
        )
    };
    Some(match line(&asked) {
        (std::cmp::Ordering::Less, _) | (std::cmp::Ordering::Equal, std::cmp::Ordering::Less) => {
            RangePlacement::Below
        }
        (std::cmp::Ordering::Greater, _)
        | (std::cmp::Ordering::Equal, std::cmp::Ordering::Greater) => RangePlacement::Above,
        (std::cmp::Ordering::Equal, std::cmp::Ordering::Equal) => {
            if asked.precedence(&floor) == std::cmp::Ordering::Less {
                RangePlacement::Below
            } else {
                RangePlacement::Inside
            }
        }
    })
}

/// An exact semantic version, split for comparison (semver.org §2, §9, §10).
///
/// Only what [`placement_of`] needs: the three numbers kept as their digit runs
/// — compared by length first, which is numeric order for runs with no leading
/// zero and never overflows — and the prerelease identifiers. Build metadata is
/// read past, because it carries no precedence.
#[derive(Clone, Debug, PartialEq, Eq)]
struct SdkVersion {
    major: String,
    minor: String,
    patch: String,
    prerelease: Vec<String>,
}

impl SdkVersion {
    /// The version `text` spells, if it is an exact one.
    fn parse(text: &str) -> Option<Self> {
        if !crate::parse::binding::is_exact_version(text) {
            return None;
        }
        let rest = text.split_once('+').map_or(text, |(rest, _)| rest);
        let (core, prerelease) = match rest.split_once('-') {
            Some((core, prerelease)) => (core, prerelease.split('.').map(str::to_string).collect()),
            None => (rest, Vec::new()),
        };
        let mut parts = core.split('.').map(str::to_string);
        Some(Self {
            major: parts.next()?,
            minor: parts.next()?,
            patch: parts.next()?,
            prerelease,
        })
    }

    /// Semver precedence (semver.org §11).
    fn precedence(&self, other: &Self) -> std::cmp::Ordering {
        numeric(&self.major, &other.major)
            .then_with(|| numeric(&self.minor, &other.minor))
            .then_with(|| numeric(&self.patch, &other.patch))
            .then_with(
                || match (self.prerelease.is_empty(), other.prerelease.is_empty()) {
                    (true, true) => std::cmp::Ordering::Equal,
                    // A version with a prerelease precedes the same version
                    // without one.
                    (true, false) => std::cmp::Ordering::Greater,
                    (false, true) => std::cmp::Ordering::Less,
                    (false, false) => {
                        for (left, right) in self.prerelease.iter().zip(&other.prerelease) {
                            let ordering = identifier(left, right);
                            if ordering != std::cmp::Ordering::Equal {
                                return ordering;
                            }
                        }
                        self.prerelease.len().cmp(&other.prerelease.len())
                    }
                },
            )
    }
}

/// Two numeric identifiers with no leading zero, in numeric order.
fn numeric(left: &str, right: &str) -> std::cmp::Ordering {
    left.len().cmp(&right.len()).then_with(|| left.cmp(right))
}

/// Two prerelease identifiers (semver.org §11.4): numeric ones numerically, and
/// below every alphanumeric one; alphanumeric ones in ASCII order.
fn identifier(left: &str, right: &str) -> std::cmp::Ordering {
    let digits = |held: &str| held.bytes().all(|byte| byte.is_ascii_digit());
    match (digits(left), digits(right)) {
        (true, true) => numeric(left, right),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => left.cmp(right),
    }
}

/// The version this target's `harnesses:` pins one harness's SDK to **in place
/// of** the compiler's pin, where it declares one (grammar 14.8, PRD resolved
/// q66 ruling d).
///
/// Only an entry `validate` would accept is read — an exact version inside the
/// audited range, for a harness this release lowers — so a caller holding an
/// artifact that was never checked still cannot write an unaudited version into
/// a manifest. An entry that states the pin itself is no override and answers
/// `None`: it changes nothing `build` writes.
#[must_use]
pub fn override_of(ir: &Ir, harness: Harness) -> Option<&str> {
    let entry = ir.deploy.harnesses.as_ref()?.get(harness.as_str())?;
    let version = entry.sdk_version.value.as_str();
    (placement_of(harness, version) == Some(RangePlacement::Inside)
        && version != version_of(harness))
    .then_some(version)
}

/// The version of one harness's SDK this target's project installs: the
/// target's override where it declares one, and the compiler's pin otherwise.
///
/// The one reading of `harnesses:` every emitted surface shares —
/// `package.json`, the driver's fallback constant in `src/harness.ts`, the
/// README's pins table, and the `module:` agreement rule — so none of them can
/// name a version the others do not.
#[must_use]
pub fn sdk_version(ir: &Ir, harness: Harness) -> &str {
    override_of(ir, harness).unwrap_or_else(|| version_of(harness))
}

/// The packages one harness's driver brings **for this target**: its own SDK at
/// [`sdk_version`], and the SDK's pinned peers exactly as [`HARNESS_PINS`] pins
/// them — an override moves the SDK alone (PRD resolved q66 ruling d).
#[must_use]
pub fn pins_for(ir: &Ir, harness: Harness) -> Vec<(&'static str, &str)> {
    pins_of(harness)
        .iter()
        .enumerate()
        .map(|(index, (package, version))| {
            if index == 0 {
                (*package, sdk_version(ir, harness))
            } else {
                (*package, *version)
            }
        })
        .collect()
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

    contents.push_str("\nimport fs from \"node:fs\";\n");
    contents.push_str("import path from \"node:path\";\n");
    contents.push_str("import { fileURLToPath } from \"node:url\";\n\n");
    contents.push_str("import * as runtime from \"./runtime.ts\";\n");
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
        // The package a driver maps over and the version this release pinned it
        // to, emitted here rather than written into the TypeScript so neither
        // can drift from the manifest (see the module header).
        contents.push_str(&names::doc(
            "",
            &[format!(
                "The `{}` SDK — the package `package.json` declares, and the one a \
                 `HarnessRecord` names.",
                harness.as_str()
            )],
        ));
        contents.push_str(&format!(
            "const {}: string = {};\n\n",
            package_constant(*harness),
            names::string(package_of(*harness))
        ));
        // The version this target installs, which is the pin unless the
        // target's `harnesses:` moved it inside the audited range (grammar
        // 14.8, PRD resolved q66): the fallback a record reads has to be the
        // number `package.json` declares, or a project whose manifest cannot
        // be read would name a release the target never installed.
        let first = match override_of(ir, *harness) {
            None => format!(
                "The `{}` SDK version this compiler release pins — the number \
                 `package.json` declares.",
                harness.as_str()
            ),
            Some(_) => format!(
                "The `{name}` SDK version this target's `harnesses.{name}` declares, \
                 in place of this compiler release's pin `{}` — the number \
                 `package.json` declares.",
                version_of(*harness),
                name = harness.as_str()
            ),
        };
        contents.push_str(&names::doc(
            "",
            &[
                first,
                String::new(),
                "A `HarnessRecord` reports the version **installed** instead \
                 (`installedVersion`), and this one only where the installed \
                 package's manifest cannot be read: in a project installed from \
                 the manifest `build` wrote the two are one number."
                    .to_string(),
            ],
        ));
        contents.push_str(&format!(
            "const {}: string = {};\n\n",
            version_constant(*harness),
            names::string(sdk_version(ir, *harness))
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

/// The name of the emitted constant holding one harness's SDK package.
fn package_constant(harness: Harness) -> &'static str {
    match harness {
        Harness::Cc => "CC_SDK",
        Harness::Codex => "CODEX_SDK",
        Harness::DeepAgents | Harness::Native => "",
    }
}

/// …and the one holding the version this release pinned it to.
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
    ///
    /// **The surface is what the SDK reads, not only what it types.** The
    /// Agent SDK's bundle reads option names its `Options` declaration never
    /// mentions, and `passthrough` forwards those as readily as any other, so
    /// an audit of the `.d.ts` alone is a floor of its own (see
    /// [`audited_surface`]'s `cc` arm for the eight that audit missed).
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
                 read the release's own options — the typed surface and every name its bundle \
                 reads off the options object beside it — add every one that reaches around \
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
            // `@anthropic-ai/claude-agent-sdk`'s option surface, which is **two
            // readings**: `Options` in `sdk.d.ts`, and the names the bundle's
            // option reader in `sdk.mjs` — the function `query()` hands its
            // options to — actually takes. The second is the wider one. It
            // reads eight names no `Options` member declares, and `passthrough`
            // forwards an unknown `settings:` key unchanged, so each reaches the
            // SDK as a declared option would: `appendSubagentSystemPrompt`,
            // `getHostAuthToken`, `getOAuthToken`, `resolvePermissionModeInCli`,
            // `webSearchIsolationExemptMcpServers` and `workspaceTrust`, reserved
            // (`harness.rs`'s rows say which bound each reaches), and
            // `rapidFollowupPreempt` and `workload`, left open — a follow-up
            // rendering declaration for a run that is handed no follow-up, and
            // a billing-attribution tag inside the CLI's own header. The same
            // eight at 0.3.272 and at 0.3.284; an earlier audit read `Options`
            // alone and let all eight through.
            //
            // 0.3.284's `Options` adds two members to 0.3.272's and removes
            // none — the same two the reader adds, so the two readings moved
            // together this time: `projectConfigRoot`, reserved (the settings
            // family read from another tree), and `verbatimPrompts`, left open
            // with both of its settings read against the pinned CLI (2.1.284)
            // rather than against its documentation:
            //
            //  * **on**, it skips the CLI's turn-start attachment pass "on
            //    current CLIs", per the SDK — which, under `read_only`'s `plan`,
            //    is where the plan-mode reminder holding the node read-only
            //    arrives. The pinned CLI still sends that reminder,
            //    `planModeInstructions` and all, on a `plan` run's first request
            //    (observed: only the token-budget reminder went missing), so on
            //    narrows. **Re-verify that at the next bump, before moving this
            //    version**: a CLI that stops sending it puts `verbatimPrompts`
            //    on `CC_RESERVED`;
            //  * **off** — the default, and where the driver leaves it — the CLI
            //    acts on two things in `run.input` before a model reads it, and
            //    both reach around a bound. It expands an `@path` mention into a
            //    synthetic `Read` result on the first request: a file outside
            //    `workspace:` reaches the model with no tool call, past `tools`,
            //    `canUseTool` and the mode (observed at this pin and at 0.3.272,
            //    under `acceptEdits` and `plan`, with `Read` outside `tools`).
            //    And it dispatches a prompt opening with `/<command>` as one of
            //    its own commands: `/model <id>` switches the session's model,
            //    so the run's request goes out on `<id>` rather than on what
            //    `model:` mapped to (observed at this pin and at 0.3.272), and
            //    the init message's `slash_commands` lists `config`, `mcp`,
            //    `effort`, `fast` and `agents` among the others the same text
            //    reaches, none of them audited. That is a hole the driver does
            //    not close: turning the option on for every run also drops the
            //    first turn's `CLAUDE.md`, skill and tool listings, so whether
            //    the adapter owns it is a PRD question — one question, with both
            //    halves in it — rather than this audit's. `harness-cc.ts` says
            //    all of it.
            Harness::Cc => (
                "0.3.284",
                &[
                    "abortController",
                    "additionalDirectories",
                    "agent",
                    "agents",
                    "allowDangerouslySkipPermissions",
                    "allowedTools",
                    "appendSubagentSystemPrompt",
                    "canUseTool",
                    "continue",
                    "cwd",
                    "env",
                    "executable",
                    "executableArgs",
                    "extraArgs",
                    "fallbackModel",
                    "forkSession",
                    "getHostAuthToken",
                    "getOAuthToken",
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
                    "projectConfigRoot",
                    "resolvePermissionModeInCli",
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
                    "webSearchIsolationExemptMcpServers",
                    "workspaceTrust",
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

    /// The package and pin a driver is emitted with are the ones the manifest
    /// declares.
    ///
    /// Two surfaces read one table: `package.json` declares the pin, and the
    /// emitted module carries the same package and version as the constants a
    /// driver names itself by and falls back to. A reader joining a trace to a
    /// manifest compares those strings, so they are held to one source here
    /// rather than by two constants agreeing on the day they were written.
    #[test]
    fn the_emitted_driver_reports_the_version_the_manifest_pins() {
        for (harness, pins) in HARNESS_PINS {
            let (package, version) = pins[0];
            assert_eq!(
                (package_of(*harness), version_of(*harness)),
                (package, version),
                "`{}`'s own SDK is the first pin of its row",
                harness.as_str()
            );
            assert!(
                !package.is_empty() && !version.is_empty(),
                "every pin names a package at a version"
            );
            let emitted = module(&one_coder_node(*harness)).contents;
            for (constant, value) in [
                (package_constant(*harness), package),
                (version_constant(*harness), version),
            ] {
                assert!(
                    emitted.contains(&format!(
                        "const {constant}: string = {};",
                        names::string(value)
                    )),
                    "`{}`'s module does not carry `{constant}` as the manifest's `{value}`",
                    harness.as_str()
                );
            }
        }
    }

    /// **A driver reports the SDK version installed, not the one compiled in**
    /// (`docs/trace.md` §7.6).
    ///
    /// The pin is what `package.json` declares; what ran is whatever the
    /// project's install resolved, and the two part company exactly when the
    /// manifest was edited after `build` — the edit `build --check` reports, and
    /// the one a trace naming the pin would hide. So each driver names its
    /// package by the emitted constant and takes its `version` from
    /// `installedVersion`, which reads the installed package's own manifest and
    /// answers the pin only where it cannot. Read off the source for the reason
    /// the other driver guards here are: it names the shape that is wrong — a
    /// `version:` that *is* the constant — rather than one value that is right.
    /// `generated_code_gates`' `a_harness_record_names_the_sdk_version_that_ran`
    /// is the run: stand-in SDKs installed at a version no release pins, and the
    /// records that name it.
    #[test]
    fn a_driver_reports_the_installed_sdk_version_rather_than_the_pin() {
        for harness in Harness::ALL.iter().copied().filter(|h| h.ships_in_v1()) {
            let (source, _, _) = driver_source(harness);
            let (package, version) = (package_constant(harness), version_constant(harness));
            assert!(
                source.contains(&format!("  sdk: {package},\n"))
                    && source.contains(&format!(
                        "  version: installedVersion({package}, {version}),\n"
                    )),
                "`{}`'s driver does not report the installed version of `{package}` with \
                 `{version}` as the fallback, so a record from an edited install names a release \
                 that did not run (docs/trace.md §7.6)",
                harness.as_str()
            );
            assert!(
                !source.contains(&format!("version: {version},")),
                "`{}`'s driver reports the compiled-in pin as the version that ran",
                harness.as_str()
            );
        }
        assert!(
            PRELUDE.contains("function installedVersion(sdk: string, pinned: string): string {")
                && PRELUDE.contains("import.meta.resolve(sdk)"),
            "the prelude does not resolve the installed package the way the driver's own \
             `import` did, so the version it reads may belong to some other copy"
        );
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
    ///    (`@anthropic-ai/claude-agent-sdk` 0.3.284, whose `sdk.mjs` carries
    ///    0.3.272's check unchanged: the same two messages, raised from `query()`
    ///    on `canUseTool` set beside `bypassPermissions` or beside a bare
    ///    `allowedTools` entry) reports the pairing as a shadowed callback,
    ///    `CLAUDE_SDK_CAN_USE_TOOL_SHADOWED`;
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

    /// **Each range floor is the version its reserved list was audited at**
    /// (grammar 14.8, Decision D151, PRD resolved q66 ruling b).
    ///
    /// The drift test the ruling asks for. [`AUDITED`] is what `validate`
    /// holds a target's `harnesses:` to, and
    /// [`a_reserved_list_is_audited_against_the_pinned_option_surface`] is where
    /// the audit that range rests on is written down, anchored on a version of
    /// its own. Two numbers for one fact drift the day a bump moves one of them:
    /// an audit re-performed at a newer release with the floor left behind would
    /// keep refusing versions the audit now covers, and — the direction that
    /// matters — a floor moved forward without the audit would admit a range
    /// nobody read, with the reserved list still describing the old surface. So
    /// neither moves alone: a bump of either one fails here, by name.
    #[test]
    fn a_range_floor_is_the_version_its_reserved_list_was_audited_at() {
        for harness in Harness::ALL.iter().copied() {
            let (audited, _) = audited_surface(harness);
            let name = harness.as_str();
            if !harness.ships_in_v1() {
                assert_eq!(
                    audited_of(harness),
                    None,
                    "`{name}` is reserved: it has no driver, no audit and so no range a target \
                     could pin its SDK inside"
                );
                continue;
            }
            assert_eq!(
                audited_of(harness),
                Some(audited),
                "`{name}`'s audited range starts at {:?} and its reserved list was audited at \
                 {audited}: the floor is the audited version, so move both together — re-audit the \
                 release's option surface under both readings, then move `AUDITED` and the audit \
                 anchor in one change (grammar 14.8, PRD resolved q66 ruling b)",
                audited_of(harness)
            );
        }
        let mut rows: Vec<Harness> = AUDITED.iter().map(|(harness, _)| *harness).collect();
        let listed = rows.len();
        rows.sort();
        rows.dedup();
        assert_eq!(rows.len(), listed, "`AUDITED` names one harness twice");
    }

    /// **Every document that states the audited ranges states this release's**
    /// (grammar 14.8, PRD resolved q66).
    ///
    /// Three documents print the table — the grammar, the `targets` topic an
    /// author on a laptop reads, and the `explain` page a refusal points at —
    /// because the range is the one thing an author asking "which version may I
    /// pin?" needs, and it is release data. Prose does not compile, so the table
    /// is read back here: a bump that moved the pin and the floor together would
    /// otherwise leave three documents naming a range `validate` no longer
    /// enforces, and the refusal's own explanation contradicting the refusal.
    #[test]
    fn every_document_stating_the_audited_ranges_states_this_releases() {
        let documents = [
            (
                "docs/grammar.md §14.8",
                include_str!("../../../../docs/grammar.md"),
            ),
            (
                "`agent-compose docs targets`",
                include_str!("../../../../docs/topics/targets.md"),
            ),
            (
                "`agent-compose explain harness-sdk-outside-audited-range`",
                include_str!("../docs/codes/harness-sdk-outside-audited-range.md"),
            ),
        ];
        for (place, text) in documents {
            for harness in Harness::ALL.iter().copied().filter(|h| h.ships_in_v1()) {
                let range = audited_range(harness).expect("a shipping harness has a range");
                let row = format!(
                    "| `{}` | `{}` | `{}` | `{range}` |",
                    harness.as_str(),
                    package_of(harness),
                    version_of(harness)
                );
                assert!(
                    text.contains(&row),
                    "{place} does not state `{}`'s pin and audited range as this release \
                     declares them — expected the row {row}",
                    harness.as_str()
                );
            }
        }
    }

    /// The range is `[audited, next minor)`, stated here in the spelling a
    /// refusal prints it in — and in every release the pin is its floor.
    #[test]
    fn an_audited_range_runs_from_the_audited_version_through_that_minor() {
        assert_eq!(
            audited_range(Harness::Cc).map(|range| range.to_string()),
            Some("[0.3.284, 0.4.0)".to_string())
        );
        assert_eq!(
            audited_range(Harness::Codex).map(|range| range.to_string()),
            Some("[0.154.0, 0.155.0)".to_string())
        );
        for harness in Harness::ALL.iter().copied() {
            let Some(range) = audited_range(harness) else {
                assert!(!harness.ships_in_v1(), "a harness that ships has a range");
                continue;
            };
            assert_eq!(
                range.floor,
                version_of(harness),
                "`{}`'s pin is not the floor of its audited range, so a target that declares no \
                 `harnesses:` entry would build a version the range does not start at",
                harness.as_str()
            );
            assert_eq!(
                placement_of(harness, range.floor),
                Some(RangePlacement::Inside),
                "the floor is inside its own range"
            );
            assert_eq!(
                placement_of(harness, &range.ceiling),
                Some(RangePlacement::Above),
                "the next minor is outside the range it closes"
            );
        }
    }

    /// Where a version sits is **semver precedence**, not string order, and the
    /// range is one minor's releases (grammar 14.8).
    ///
    /// Each row is a spelling a string comparison or a sloppier range gets wrong:
    /// `0.3.99` sorts after `0.3.284` as text and is older; `0.30.0` shares the
    /// floor's prefix and is a different minor; `0.4.0-rc.1` precedes `0.4.0` and
    /// is still the next minor, which nobody audited; a prerelease of the floor
    /// precedes the floor; build metadata carries no precedence at all.
    #[test]
    fn a_version_is_placed_by_semver_precedence_within_the_audited_minor() {
        use RangePlacement::{Above, Below, Inside};
        for (version, expected) in [
            ("0.3.284", Some(Inside)),
            ("0.3.285", Some(Inside)),
            ("0.3.1000", Some(Inside)),
            ("0.3.284+local.build", Some(Inside)),
            ("0.3.290-beta.1", Some(Inside)),
            ("0.3.284-rc.1", Some(Below)),
            ("0.3.283", Some(Below)),
            ("0.3.99", Some(Below)),
            ("0.2.999", Some(Below)),
            ("0.0.1", Some(Below)),
            ("0.4.0", Some(Above)),
            ("0.4.0-rc.1", Some(Above)),
            ("0.30.0", Some(Above)),
            ("1.0.0", Some(Above)),
            ("^0.3.284", None),
            ("~0.3.284", None),
            ("0.3.x", None),
            ("0.3", None),
            ("latest", None),
            ("0.3.284 ", None),
            ("00.3.284", None),
        ] {
            assert_eq!(
                placement_of(Harness::Cc, version),
                expected,
                "`{version}` against `cc`'s range {:?}",
                audited_range(Harness::Cc).map(|range| range.to_string())
            );
        }
        // …and the reserved harnesses have no range to be placed against.
        assert_eq!(placement_of(Harness::DeepAgents, "0.1.0"), None);

        // semver.org §11's own ordering example, which is the prerelease rule the
        // floor's own prereleases are placed by.
        let ordered = [
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            let (left, right) = (
                SdkVersion::parse(pair[0]).expect("an exact version"),
                SdkVersion::parse(pair[1]).expect("an exact version"),
            );
            assert_eq!(
                left.precedence(&right),
                std::cmp::Ordering::Less,
                "`{}` precedes `{}` (semver.org §11)",
                pair[0],
                pair[1]
            );
            assert_eq!(right.precedence(&left), std::cmp::Ordering::Greater);
        }
    }

    /// **A target's `sdk_version:` moves its harness's SDK, and only that
    /// package, in every surface that names it** (grammar 14.8, PRD resolved
    /// q66 ruling d).
    ///
    /// `package.json` declares the target's version in the pin's place, the
    /// driver's fallback constant is that same number — so a record from an
    /// install whose manifest cannot be read names the release the target asked
    /// for rather than one it never installed — and the SDK's pinned peers stay
    /// where the compiler put them, because a patch series resolves against
    /// them. An entry stating the pin itself is no override and changes nothing
    /// `build` writes.
    #[test]
    fn a_targets_sdk_version_moves_the_sdk_alone_in_every_surface_that_names_it() {
        let moved = crate::codegen::test_support::ir_of_mesh(
            &coder_composition(Harness::Cc),
            "version: \"0.1\"\nharnesses:\n  cc:\n    sdk_version: \"0.3.285\"\n",
        );
        assert_eq!(override_of(&moved, Harness::Cc), Some("0.3.285"));
        assert_eq!(sdk_version(&moved, Harness::Cc), "0.3.285");
        let pins: Vec<(&str, &str)> = pins_of(Harness::Cc)
            .iter()
            .enumerate()
            .map(|(index, (package, version))| {
                (*package, if index == 0 { "0.3.285" } else { *version })
            })
            .collect();
        assert_eq!(
            pins_for(&moved, Harness::Cc),
            pins,
            "the peers moved with the SDK"
        );

        let manifest: serde_json::Value =
            serde_json::from_str(&crate::codegen::project::package_json(&moved).contents)
                .expect("strict JSON");
        for (package, version) in &pins {
            assert_eq!(
                manifest["dependencies"][*package],
                serde_json::Value::from(*version),
                "`package.json` declares `{package}` at a version other than the target's"
            );
        }
        let emitted = module(&moved).contents;
        assert!(
            emitted.contains("const CC_SDK_VERSION: string = \"0.3.285\";"),
            "the driver's fallback is not the version `package.json` declares: {emitted}"
        );
        assert!(
            emitted.contains(
                "this target's `harnesses.cc` declares, in place of this compiler \
                 release's pin `0.3.284`"
            ),
            "the constant does not say whose number it is: {emitted}"
        );

        // …and an entry that states the pin itself is no override at all.
        let stated = crate::codegen::test_support::ir_of_mesh(
            &coder_composition(Harness::Cc),
            &format!(
                "version: \"0.1\"\nharnesses:\n  cc:\n    sdk_version: \"{}\"\n",
                version_of(Harness::Cc)
            ),
        );
        let plain = crate::codegen::test_support::ir_of_mesh(
            &coder_composition(Harness::Cc),
            "version: \"0.1\"\n",
        );
        assert_eq!(override_of(&stated, Harness::Cc), None);
        assert_eq!(module(&stated).contents, module(&plain).contents);
        assert_eq!(
            crate::codegen::project::package_json(&stated).contents,
            crate::codegen::project::package_json(&plain).contents
        );
    }

    /// An entry `validate` would refuse never reaches a manifest, even from an
    /// artifact nobody checked (grammar 14.8).
    ///
    /// `build` refuses a composition the validator does, so on the command line
    /// this is unreachable; what it guards is the reading itself. [`override_of`]
    /// is the one place a target's version enters the emitted project, and a
    /// reading that took the text on trust would write an unaudited release into
    /// `package.json` for any caller that emits without checking first.
    #[test]
    fn an_entry_outside_the_range_is_never_read_as_an_override() {
        let mut ir = crate::codegen::test_support::ir_of_mesh(
            &coder_composition(Harness::Cc),
            "version: \"0.1\"\nharnesses:\n  cc:\n    sdk_version: \"0.3.285\"\n",
        );
        for refused in ["0.4.0", "0.3.283", "^0.3.285", "${CC_SDK}", "latest"] {
            ir.deploy
                .harnesses
                .as_mut()
                .and_then(|section| section.entries.get_mut("cc"))
                .expect("the entry the deploy file declared")
                .sdk_version
                .value = refused.to_string();
            assert_eq!(override_of(&ir, Harness::Cc), None, "`{refused}` was read");
            assert_eq!(sdk_version(&ir, Harness::Cc), version_of(Harness::Cc));
        }
    }

    /// A composition whose one flow runs one `coder:` node on `harness`, over a
    /// provider whose wire that harness speaks — so, unlike [`one_coder_node`],
    /// one the validator accepts under either harness.
    fn coder_composition(harness: Harness) -> String {
        let kind = match harness {
            Harness::Codex => "openai",
            _ => "anthropic",
        };
        format!(
            "version: \"0.1\"
provider.p:
  kind: {kind}
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
        prompt: Do the work.
        output:
          summary: {{ type: string }}
      input: \"'go'\"
  edges:
    - {{ from: start, to: build }}
    - {{ from: build, to: end }}
",
            harness.as_str()
        )
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
