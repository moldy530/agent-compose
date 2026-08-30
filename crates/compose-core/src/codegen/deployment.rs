//! `src/deployment.ts`: what the deploy layer declares, lowered for the two
//! processes that read it (grammar §14.1, §14.2, `docs/distributed.md` §9.1).
//!
//! # Why this is a module of the project rather than a table on the hub
//!
//! `docs/distributed.md` §9.1 makes the environment partition ship **in the
//! artifact**, and says why: "One partition is emitted into the generated
//! project, so the hub reading it and a worker reading it are reading one answer
//! under one artifact hash — there is no second derivation to disagree, and the
//! handshake that pins the artifact (§4.1) pins the manifest with it." A hub that
//! computed its placements' manifests from a spec it holds and a worker that
//! computed its own from the tree it unpacked would be two derivations of one
//! fact, and §3.1's `env_ok` — a subset of a list the hub already computed — has
//! no meaning unless both ends read the same list.
//!
//! So this file is emitted for **every** project, mesh or not. A composition with
//! no `placements:` emits an empty list and a hub manifest that is the whole of
//! its environment, which is what such a deployment has always had; the modules
//! that read it ([`super::mesh`]) then mount nothing. Emitting it conditionally
//! would make `src/mesh.ts` — a constant of the compiler release — import a module
//! that sometimes exists.
//!
//! # What is *not* here
//!
//! The artifact's own hash and file list, which are [`super::artifact`]'s: this
//! module's contents are **inside** the hash, so a placement renamed or a
//! variable moved between manifests moves the hash, which is the property
//! §4.1's handshake needs.

use std::collections::BTreeSet;

use crate::ir::Ir;

use super::env::{Partition, Process, References};
use super::names;

/// `src/deployment.ts`.
#[must_use]
pub fn module(ir: &Ir, partition: &Partition) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents.push_str(DECLARATIONS);

    contents.push_str("export const placements: readonly PlacementManifest[] = [");
    let placements: Vec<&Process> = partition
        .processes()
        .filter(|process| !matches!(process, Process::Hub))
        .collect();
    if placements.is_empty() {
        contents.push_str("];\n");
    } else {
        contents.push('\n');
        for process in placements {
            let name = process.name();
            contents.push_str(&format!(
                "  {{\n    name: {},\n    members: [\n",
                names::string(name)
            ));
            for member in members(ir, name) {
                contents.push_str(&format!("      {},\n", names::string(&member)));
            }
            contents.push_str("    ],\n    environment: [\n");
            for variable in References::for_process(ir, partition, process).names() {
                contents.push_str(&format!("      {},\n", names::string(variable)));
            }
            contents.push_str("    ],\n  },\n");
        }
        contents.push_str("];\n");
    }

    contents.push_str("\nexport const hubEnvironment: readonly string[] = [");
    let hub: Vec<String> = References::for_process(ir, partition, &Process::Hub)
        .names()
        .map(str::to_string)
        .collect();
    if hub.is_empty() {
        contents.push_str("];\n");
    } else {
        contents.push('\n');
        for variable in hub {
            contents.push_str(&format!("  {},\n", names::string(&variable)));
        }
        contents.push_str("];\n");
    }

    let token = ir
        .deploy
        .hub
        .as_ref()
        .and_then(|hub| hub.join_token.as_ref())
        .map(|token| token.value.name.clone());
    contents.push_str(&format!(
        "\nexport const joinTokenEnv: string | undefined = {};\n",
        token.as_deref().map_or_else(
            || "undefined".to_string(),
            |name| names::string(name).to_string()
        )
    ));

    let base = ir
        .deploy
        .hub
        .as_ref()
        .and_then(|hub| hub.public_url.as_ref())
        .map(|url| url.value.clone());
    contents.push_str(&format!(
        "\nexport const publicUrl: string | undefined = {};\n",
        base.as_deref().map_or_else(
            || "undefined".to_string(),
            |url| names::string(url).to_string()
        )
    ));

    super::GeneratedFile {
        path: "src/deployment.ts".to_string(),
        contents,
    }
}

/// The members of one placement, in the order the deploy file writes them.
fn members(ir: &Ir, name: &str) -> Vec<String> {
    let Some(section) = ir.deploy.placements.as_ref() else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    section
        .entries
        .values()
        .filter(|placement| placement.name.value.as_str() == name)
        .flat_map(|placement| placement.members.iter())
        .map(|member| member.value.to_string())
        // A member written twice is already a compile error (Decision D129);
        // emitting it twice would make the artifact say something the deploy
        // file's own diagnostic denies.
        .filter(|address| seen.insert(address.clone()))
        .collect()
}

const MODULE_DOC: &str = "\
//
// What this artifact's deploy layer declares: the placements a worker may claim,
// the environment partition of `docs/distributed.md` §9.1, and the hub's own two
// keys (grammar §14.1, §14.2).
//
// The partition is emitted here rather than derived at either end because §9.1
// requires one answer under one artifact hash: the hub checks a join's `env_ok`
// against the claimed placements' lists, and the worker reads the list it is
// reporting against out of the very tree the hub served it.
//
// `hubEnvironment` is the hub's own side of that partition, stated as names.
// **It is not what `readEnvironment()` reads**: `./env.ts` carries the same
// variables with the spec sites that wrote each one, because a launch that is
// short a key should name where the key was asked for, and that is the list
// `./index.ts` checks at start. This one is the manifest form — names only, like
// a placement's — so a reader of the artifact, or of the tarball a worker
// unpacked, can see what the hub holds beside what each placement holds without
// reading two shapes. The two are one derivation filtered two ways
// (`References::for_process`), and a test pins them equal.
//
// Both are narrower than the composition's whole environment exactly when a
// placement takes something off the hub's — the least-privilege line PRD 5.10
// draws and §9.1 computes: the hub cannot leak what it never held.
";

const DECLARATIONS: &str = r#"
/** One placement, and the environment a worker claiming it has to satisfy. */
export interface PlacementManifest {
  /** The name a worker claims at join (`docs/distributed.md` §3.1). */
  readonly name: string;
  /**
   * `members:`, in the order the deploy file writes them (grammar §14.1).
   *
   * For a **reader**, not for a caller: the scheduler queues work to a placement
   * by name and never consults this list (`docs/distributed.md` §2), and the
   * `environment` beside it is already the answer to what a worker must satisfy.
   * It is here because a worker holds no YAML — the artifact is the only thing
   * it is served — so "what did I just claim" is otherwise unanswerable on the
   * machine that claimed it.
   */
  readonly members: readonly string[];
  /**
   * The variables of every component that can execute in this placement's
   * process, sorted by name — `docs/distributed.md` §9.1's partition.
   *
   * Names only. A manifest carries no value and no site: what a join reports is
   * a subset of this list (§9.2), and a report scoped to anything wider would be
   * telling the hub about credentials that are none of its business.
   */
  readonly environment: readonly string[];
}

"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn a_composition_with_no_placements_emits_an_empty_list_and_its_whole_environment() {
        let ir = ir_of(
            "version: \"0.1\"\n\
provider.p:\n  kind: openai\n  api_key: ${SOME_KEY}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
agent.a:\n  model: model.m\n  prompt: Hello.\n  input: { q: { type: string } }\n  output: { a: { type: string } }\n",
        );
        let emitted = module(&ir, &Partition::of(&ir)).contents;
        assert!(
            emitted.contains("export const placements: readonly PlacementManifest[] = [];"),
            "{emitted}"
        );
        assert!(emitted.contains("  \"SOME_KEY\",\n"), "{emitted}");
        assert!(
            emitted.contains("export const joinTokenEnv: string | undefined = undefined;"),
            "{emitted}"
        );
    }

    /// `hubEnvironment` and `src/env.ts`'s list name the same variables.
    ///
    /// The artifact states the hub's manifest twice — once as names here, once
    /// with the spec sites that wrote each one in `src/env.ts`, which is the
    /// list `readEnvironment()` refuses on. Both come out of
    /// `References::for_process(…, Hub)`, so they cannot disagree today; this is
    /// what would notice if one of the two calls were changed and not the other,
    /// which would leave a worker's unpacked tree stating a hub manifest the hub
    /// itself does not check.
    #[test]
    fn the_hub_manifest_and_the_launch_check_name_one_list() {
        let ir = crate::codegen::test_support::ir_of_mesh(
            "version: \"0.1\"\n\
provider.vendor:\n  kind: openai\n  api_key: ${VENDOR_KEY}\n\
model.smart:\n  provider: provider.vendor\n  id: some-model\n\
tool.sign:\n  description: Sign one artifact.\n  input: { path: { type: string } }\n  output: { signature: { type: string } }\n  exec:\n    command: codesign\n    env:\n      KEYCHAIN_PASSWORD: ${KEYCHAIN_PASSWORD}\n\
agent.signer:\n  model: model.smart\n  prompt: Sign what you are given.\n  tools: [tool.sign]\n  input: { path: { type: string } }\n  output: { verdict: { type: string } }\n\
flow.release:\n  inputs:\n    path: { type: string }\n  outputs: {}\n  nodes:\n    sign:\n      agent: agent.signer\n      input:\n        path: \"input.path\"\n  edges:\n    - { from: start, to: sign }\n    - { from: sign, to: end }\n",
            "version: \"0.1\"\n\
hub:\n  join_token: ${MESH_TOKEN}\n\
placements:\n  mac:\n    members: [agent.signer]\n",
        );
        let partition = Partition::of(&ir);
        let held = References::for_process(&ir, &partition, &Process::Hub);
        let hub: Vec<&str> = held.names().collect();
        assert_eq!(hub, ["MESH_TOKEN"], "the worked example of §9.1");

        let emitted = module(&ir, &partition).contents;
        let manifest = emitted
            .split_once("export const hubEnvironment: readonly string[] = [")
            .expect("the module declares the hub's list")
            .1
            .split_once("];")
            .expect("the list is closed")
            .0;
        let stated: Vec<String> = manifest
            .lines()
            .filter_map(|line| line.trim().strip_suffix(','))
            .map(|entry| entry.trim_matches('"').to_string())
            .collect();
        assert_eq!(stated, hub, "{emitted}");

        // …and the module `readEnvironment()` is generated from, which is the
        // same filter under a different shape.
        let checked = super::super::env::module(&ir, &held).contents;
        for variable in &hub {
            assert!(
                checked.contains(&format!("name: \"{variable}\",")),
                "`{variable}` is on the hub's manifest and not in its launch check: {checked}"
            );
        }
        assert!(
            !checked.contains("\"KEYCHAIN_PASSWORD\""),
            "the hub never runs `tool.sign`, so its launch check may not ask for its \
             secret: {checked}"
        );
    }
}
