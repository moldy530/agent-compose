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
            "import type {\n  SandboxMode,\n  ThreadItem,\n  ThreadOptions,\n} from \"@openai/codex-sdk\";\n",
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
}
