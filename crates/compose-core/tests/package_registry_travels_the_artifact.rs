//! A `package_registry:` reaches every machine that installs — through the
//! machinery the artifact already has (grammar §14.6, PRD resolved q59 rulings
//! b–d).
//!
//! The gap this key closes is a *distribution* gap rather than a configuration
//! one. A `bunfig.toml` placed by hand beside a built project works on the
//! machine it was placed on and is invisible everywhere else: the manifest
//! boundary (PRD resolved q47) leaves it alone, so `build` never writes it, so
//! the artifact never carries it, so every worker's `bun install` at materialise
//! (`docs/distributed.md` §4 step 4) resolves against the public registry the
//! corporate network blocks. What this file pins is that the emitted
//! configuration does **not** have that shape:
//!
//! * both files are ordinary members of the emitted file list, so they are in
//!   `ARTIFACT_FILES` — which is the list `src/mesh.ts` packs a tarball from —
//!   and under `ARTIFACT_HASH`, so a changed registry is a changed artifact that
//!   reaches every worker through the join handshake with no new protocol;
//! * the credential joins the environment manifest of the hub **and** of every
//!   placement, because every one of those processes installs (§9.1's rule
//!   reading on one more reference site, exactly as resolved q58 extended it to
//!   provider connections);
//! * and the token stays a *reference* through all of it, so nothing secret is
//!   in the tarball and a rotation does not move the hash.
//!
//! The counterpart is here too, because it is the half that catches a leak: a
//! target that declares no registry emits neither file and demands no variable.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Authored, emit, resolve_with_target};

/// A composition with something to place, so there is a placement manifest for
/// the credential to be absent from or present on.
const SPEC: &str = r#"version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

agent.builder:
  model: model.smart
  prompt: Build the requested target and report what the log says.
  input:
    target: { type: string }
  output:
    verdict: { type: string }

flow.release:
  inputs:
    target: { type: string }
  outputs: {}
  nodes:
    build:
      agent: agent.builder
      input:
        target: "input.target"
  edges:
    - { from: start, to: build }
    - { from: build, to: end }
"#;

/// A deploy layer with a mirror, a credential, and one scope of its own.
const WITH_REGISTRY: &str = r#"version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  builders:
    members: [agent.builder]

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
      token: ${NPM_CORP_TOKEN}
"#;

/// The same deployment with no mirror at all.
const WITHOUT_REGISTRY: &str = r#"version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  builders:
    members: [agent.builder]
"#;

/// The two paths a declared registry adds, in the order a sorted tree holds
/// them.
const INSTALLER_FILES: [&str; 2] = [".npmrc", "bunfig.toml"];

fn scratch(name: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("package-registry")
        .join(format!("{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("deploy")).expect("can create the project");
    directory
}

/// Resolve `SPEC` under `--target mesh` with this deploy layer, insisting the
/// composition is one `build` would accept.
fn built(name: &str, deploy: &str) -> compose_core::GeneratedProject {
    let directory = scratch(name);
    fs::write(directory.join("main.yml"), SPEC).expect("can write the spec");
    fs::write(directory.join("deploy/mesh.yml"), deploy).expect("can write the deploy file");
    let resolution = resolve_with_target(directory.join("main.yml"), "mesh");
    let ir = resolution.ir.expect("the fixture resolves");
    let mut diagnostics = resolution.diagnostics;
    diagnostics.extend(compose_core::check(&ir));
    assert!(
        diagnostics.is_empty(),
        "the fixture no longer validates, so this file measures the wrong thing: {diagnostics:#?}"
    );
    let project = emit(&ir, &Authored::none());
    let _ = fs::remove_dir_all(&directory);
    project
}

fn contents<'a>(project: &'a compose_core::GeneratedProject, path: &str) -> &'a str {
    project
        .file(path)
        .unwrap_or_else(|| panic!("this build emits `{path}`"))
        .contents
        .as_str()
}

/// The installer configuration is in the artifact, on the list the hub packs a
/// tarball from — so a worker materialises it with no change to the protocol.
#[test]
fn the_installer_configuration_is_a_member_of_the_artifact() {
    let project = built("member", WITH_REGISTRY);
    let paths: Vec<&str> = project.paths().collect();
    for path in INSTALLER_FILES {
        assert!(
            paths.contains(&path),
            "`{path}` is not in the tree `build` writes, so it is not in the tarball a worker \
             fetches (docs/distributed.md §3.5, §4): {paths:?}"
        );
    }

    // `src/mesh.ts` packs the artifact route's tarball by walking
    // `ARTIFACT_FILES`, so membership in that list is what "it ships" means.
    let artifact = contents(&project, "src/artifact.ts");
    for path in INSTALLER_FILES {
        assert!(
            artifact.contains(&format!("  \"{path}\",")),
            "`ARTIFACT_FILES` does not list `{path}`, and it is the list the hub serves the \
             tarball from:\n{artifact}"
        );
    }
}

/// …and under the hash, so a changed registry reaches every worker through the
/// join handshake — while a rotated **token** does not, because what is in the
/// bytes is the variable's name.
#[test]
fn the_registry_is_hashed_but_the_credential_it_names_is_not() {
    let hash = |project: &compose_core::GeneratedProject| {
        contents(project, "src/artifact.ts")
            .lines()
            .find(|line| line.starts_with("export const ARTIFACT_HASH"))
            .expect("the artifact module declares its hash")
            .to_string()
    };

    let declared = built("hash-declared", WITH_REGISTRY);
    let none = built("hash-none", WITHOUT_REGISTRY);
    assert_ne!(
        hash(&declared),
        hash(&none),
        "declaring a mirror did not change the artifact, so a worker holding the old one would \
         never be sent the new configuration (PRD resolved q40)"
    );

    let moved = built(
        "hash-moved",
        &WITH_REGISTRY.replace("npm-group/", "npm-group-2/"),
    );
    assert_ne!(
        hash(&declared),
        hash(&moved),
        "moving the registry did not change the artifact"
    );

    let rotated = built(
        "hash-rotated",
        &WITH_REGISTRY.replace("${NPM_MIRROR_TOKEN}", "${NPM_MIRROR_TOKEN_2024}"),
    );
    assert_ne!(
        hash(&declared),
        hash(&rotated),
        "renaming the variable is a change to the emitted bytes and must be a change to the hash"
    );
    // The value behind the name is never in the bytes at all — see
    // `crates/agent-compose/tests/build_cli.rs::build_writes_the_registry_reference_rather_than_the_token_it_resolves_to`,
    // which runs a real build with the value in its environment.
    for path in INSTALLER_FILES {
        let emitted = contents(&declared, path);
        assert!(
            emitted.contains("NPM_MIRROR_TOKEN"),
            "`{path}` does not name the variable: {emitted}"
        );
    }
}

/// The credential is on the hub's manifest **and** on every placement's,
/// because every one of those processes runs an install.
#[test]
fn the_credential_belongs_to_every_process_that_installs() {
    let project = built("manifest", WITH_REGISTRY);
    let environment = contents(&project, "src/env.ts");
    let deployment = contents(&project, "src/deployment.ts");
    let manifest = contents(&project, "manifest.json");

    for variable in ["NPM_MIRROR_TOKEN", "NPM_CORP_TOKEN"] {
        assert!(
            environment.contains(variable),
            "`src/env.ts` does not name `{variable}`, so the hub — which installs the project it \
             serves — would start clean and meet a `401` from the mirror instead (PRD resolved \
             q15, q41): {environment}"
        );
        assert!(
            deployment.contains(variable),
            "no placement's manifest names `{variable}`, and a worker runs `bun install` over \
             every artifact it materialises (docs/distributed.md §4 step 4): {deployment}"
        );
        assert!(
            manifest.contains(variable),
            "`manifest.json` is what `agent-compose worker` reads its `env_ok` report out of, \
             and it does not name `{variable}`: {manifest}"
        );
    }
    assert!(
        environment.contains("deploy.package_registry.token")
            && environment.contains("deploy.package_registry.scopes.@corp.token"),
        "the variables reach the manifest without the surfaces that wrote them: {environment}"
    );
    // The placement half, stated against the block that holds it rather than
    // against the whole module: `src/deployment.ts` declares each placement's
    // list before the hub's own, so a variable that only reached the hub would
    // satisfy a whole-file `contains` while the worker's report went without it.
    let placements = deployment
        .split_once("export const hubEnvironment")
        .expect("the module declares the hub's list after the placements")
        .0;
    for variable in ["NPM_MIRROR_TOKEN", "NPM_CORP_TOKEN"] {
        assert!(
            placements.contains(variable),
            "`{variable}` is on the hub's list and on no placement's: {placements}"
        );
    }
}

/// The counterpart, which is the half that catches a leak: no registry, no
/// files, and no variable anybody is asked for.
#[test]
fn a_target_with_no_registry_carries_no_installer_configuration() {
    let project = built("absent", WITHOUT_REGISTRY);
    let paths: Vec<&str> = project.paths().collect();
    for path in INSTALLER_FILES {
        assert!(
            !paths.contains(&path),
            "`{path}` was emitted for a target that declares no `package_registry:`: {paths:?}"
        );
    }
    let artifact = contents(&project, "src/artifact.ts");
    for path in INSTALLER_FILES {
        assert!(
            !artifact.contains(&format!("  \"{path}\",")),
            "`ARTIFACT_FILES` lists `{path}` for a build that never wrote it:\n{artifact}"
        );
    }
    for module in ["src/env.ts", "src/deployment.ts", "manifest.json"] {
        let emitted = contents(&project, module);
        assert!(
            !emitted.contains("package_registry"),
            "`{module}` carries a registry surface for a target with none: {emitted}"
        );
    }
}
