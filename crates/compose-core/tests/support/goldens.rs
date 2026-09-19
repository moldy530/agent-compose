//! The golden corpus, shared by the tests that read it.
//!
//! Two integration binaries need the same three facts about each golden — where
//! it is committed, what it was built from, and for which target — and a table
//! declared twice is a table that drifts. Files under `tests/` subdirectories are
//! not test binaries, so this one is included by both with `#[path]`.

#![allow(dead_code, reason = "each binary uses the part of the table it needs")]

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{GeneratedProject, emit, resolve_with_target};

/// One golden project.
pub struct Golden {
    /// Its directory under `tests/goldens/`.
    pub directory: &'static str,
    /// The example project it is built from, relative to the repository root.
    pub project: &'static str,
    /// The deploy target it is resolved for.
    pub target: &'static str,
}

/// The corpus: the two worked examples, one of them under both targets, and a
/// shape fixture.
///
/// `examples/triage-fanout` appears twice because only the deploy layer forks per
/// target (PRD 5.8's per-target invariant), and the environment references a
/// project needs are the visible consequence — `staging` binds two storage
/// backends whose `${ENV}` references `local` never sees.
///
/// `every-schema-form` is not an example and is not written to be read as one.
/// Between them the two worked projects reach about half of grammar 3's
/// vocabulary; the fixture reaches the rest — every `format:`, every reduce
/// policy, `unique_items`, `pattern`, the exclusive numeric bounds, an empty
/// field map, and a union in the property position rather than as an array's
/// `items:`. Each of those is a spelling this compiler could get wrong in a way
/// only `tsc` or a real parse would notice.
///
/// `patch-pipeline` is the fifth, and the coding-harness one. A `coder:` node is
/// the one kind whose emitted material is spread over three files that have to
/// agree — the config map in `src/graph.ts`, the driver in `src/harness.ts`, and
/// the pins in `package.json` — so it is the kind a golden earns most. It binds
/// **both** v1 harnesses in one composition on purpose: "emit only the drivers a
/// composition uses" is a claim about a *set*, and a golden with one harness in
/// it would read identically whether the rule were per-harness or all-or-nothing.
///
/// `coder-fanout` is the sixth, and it is `patch-pipeline`'s other half: that
/// one binds both harnesses in one composition, and this one binds one harness
/// in both spellings of `workspace:` (PRD resolved q61). A `map` dispatches a
/// flow whose coder node reads the item's own checkout out of its input scope,
/// and a second coder node takes `workspace: fresh` under a `retry:`. Each
/// lowers to a different `HarnessBinding.workspace` — an expression, and the
/// keyword — and the pair is what makes "the emitted binding carries the key as
/// written" a claim about a *set* rather than about one spelling.
///
/// `placed-nodes` is the fourth composition and the mesh one. `triage-fanout`'s
/// `staging` target already places a component, so a mesh reaches the corpus
/// either way; what this adds is the **three shapes a placed component is
/// reached by**, in one project — an `agent:` node, a `function:` node over a
/// placed tool, and a `map` whose dispatch target is placed
/// (`docs/distributed.md` §7, grammar §14.1). Each lowers to a different call
/// site, and a golden is where a change to any of them is reviewed as a diff
/// rather than discovered by a worker.
pub const GOLDENS: &[Golden] = &[
    Golden {
        directory: "every-schema-form",
        project: "crates/compose-core/tests/projects/every-schema-form",
        target: "local",
    },
    Golden {
        directory: "review-loop",
        project: "examples/review-loop",
        target: "local",
    },
    Golden {
        directory: "triage-fanout",
        project: "examples/triage-fanout",
        target: "local",
    },
    Golden {
        directory: "triage-fanout-staging",
        project: "examples/triage-fanout",
        target: "staging",
    },
    Golden {
        directory: "patch-pipeline",
        project: "examples/patch-pipeline",
        target: "local",
    },
    Golden {
        directory: "coder-fanout",
        project: "crates/compose-core/tests/projects/coder-fanout",
        target: "local",
    },
    Golden {
        directory: "placed-nodes",
        project: "crates/agent-compose/tests/projects/execution/placed-nodes",
        target: "mesh",
    },
];

/// The repository root.
pub fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// Where the committed goldens live.
pub fn goldens_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens")
}

/// The golden named by this directory.
pub fn golden(directory: &str) -> &'static Golden {
    GOLDENS
        .iter()
        .find(|golden| golden.directory == directory)
        .unwrap_or_else(|| panic!("`{directory}` is not a golden project"))
}

/// The artifact one golden is emitted from, insisting the composition is valid.
///
/// A golden built from a composition the validator rejects would be a project
/// `build` refuses to produce, which is the one thing a golden must never be.
pub fn artifact(golden: &Golden) -> compose_core::Ir {
    let entrypoint = repository().join(golden.project).join("main.yml");
    let resolution = resolve_with_target(&entrypoint, golden.target);
    assert!(
        resolution.diagnostics.is_empty(),
        "`{}` does not resolve under `{}`: {:#?}",
        golden.project,
        golden.target,
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "`{}` does not validate under `{}`: {:#?}",
        golden.project,
        golden.target,
        diagnostics
    );
    ir
}

/// The project one golden is emitted as.
///
/// The authored files a `module:` binding names are read out of the golden's own
/// **source project** — the checked-in tree beside its `main.yml`, exactly where
/// `agent-compose build` reads them from. That is what makes a module golden a
/// real test of the contract rather than of a fixture: the committed `.ts` is
/// the author's, `tsc` runs over the copy this carries into the golden
/// directory, and a schema change breaks the type check (PRD resolved q48, q49).
pub fn emitted(golden: &Golden) -> GeneratedProject {
    let ir = artifact(golden);
    let root = repository().join(golden.project);
    let authored = compose_core::Authored::read(&ir, &root).unwrap_or_else(|error| {
        panic!(
            "`{}` references a module that cannot be read: {error}",
            golden.project
        )
    });
    emit(&ir, &authored)
}

/// Every file under `root`, as `/`-separated relative paths, sorted.
///
/// `node_modules/` is skipped: a local install inside a golden directory is not
/// a file the emitter forgot to write.
pub fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut queue = vec![root.to_path_buf()];
    while let Some(directory) = queue.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "node_modules") {
                    continue;
                }
                queue.push(path);
                continue;
            }
            found.push(
                path.strip_prefix(root)
                    .expect("the walk started at the root")
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    found.sort();
    found
}
