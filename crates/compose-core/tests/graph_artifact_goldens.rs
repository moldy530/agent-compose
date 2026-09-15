//! Same spec in, byte-identical artifact out (PRD §9.15, resolved q56).
//!
//! The same discipline `tests/generated_project_goldens.rs` applies to the
//! emitted TypeScript, over the two things `visualize` emits: the **graph
//! document**, which is a versioned public surface (`docs/graph.md`), and the
//! **page**, which is that document inside the embedded template.
//!
//! Both are committed, and both are worth committing for the same two reasons a
//! codegen golden is:
//!
//! * **A regression fence.** A change to the derivation shows up as a diff over
//!   a real composition rather than as a passing unit test about a rule nobody
//!   reads. The document golden is where that diff is legible — `"covers":
//!   ["duplicate"]` disappearing is one line.
//! * **A review surface.** The page golden is the only place the template is
//!   read as HTML rather than as a string embedded in a Rust module, and a
//!   template edit is reviewed there.
//!
//! # Regenerating
//!
//! ```sh
//! UPDATE_GOLDENS=1 cargo test -p compose-core --test graph_artifact_goldens
//! ```
//!
//! Then read the diff. A template edit rewrites the page golden and touches
//! nothing else; a change to the document rewrites both, which is the review
//! the version number in `docs/graph.md` §9.3 is supposed to get.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::graph::{self, GraphDocument};
use compose_core::{Ir, resolve};

/// One golden artifact: the project it is rendered from, and the file names it
/// is committed under.
struct Golden {
    /// The example project, relative to the repository root.
    project: &'static str,
    /// The committed document, under `tests/graph-goldens/`.
    document: &'static str,
    /// The committed page, or `None` where only the document is committed.
    page: Option<&'static str>,
}

/// The corpus: three documents, and one page.
///
/// Both worked examples, because the two compositions exercise disjoint halves
/// of the format — `triage-fanout` has the routed `map`, the `human` wait, the
/// store and the subgraph, and `review-loop` has the bounded cycle, the `else:`
/// escape and a failover **route** rather than a direct model.
///
/// And `omitted-graph-keys`, which is a fixture rather than an example and is
/// the one committed here for what it does *not* declare: a homogeneous
/// fan-out, a model with no `settings:`, an agent with no tools and no
/// `input:`, inline blocks with nothing but their command and their url, and an
/// unbounded wait. Every one of those is a key this format **omits** rather
/// than writes empty (`docs/graph.md` §9.1), and the two examples happen to
/// declare every one of them — so without this the presence rules would be
/// pinned in one direction only.
///
/// One page, because the page is the document plus a template that does not
/// vary: a second copy of a 60-kilobyte template would double what a template
/// edit rewrites and prove nothing the first does not.
const GOLDENS: &[Golden] = &[
    Golden {
        project: "examples/triage-fanout",
        document: "triage-fanout.json",
        page: Some("triage-fanout.html"),
    },
    Golden {
        project: "examples/review-loop",
        document: "review-loop.json",
        page: None,
    },
    Golden {
        project: "crates/compose-core/tests/projects/omitted-graph-keys",
        document: "omitted-graph-keys.json",
        page: None,
    },
    // The `coder:` kind, and the one record whose most important field is in no
    // composition: `tools_enforced` is a fact about the *harness*, so a diff is
    // the only place a change to which harness enforces what is reviewable
    // (`docs/graph.md` §5.8, PRD resolved q57 ruling c).
    Golden {
        project: "examples/patch-pipeline",
        document: "patch-pipeline.json",
        page: None,
    },
];

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

fn goldens_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/graph-goldens")
}

fn updating() -> bool {
    std::env::var_os("UPDATE_GOLDENS").is_some_and(|value| !value.is_empty())
}

/// The artifact one golden is rendered from, insisting the composition is
/// valid.
///
/// `visualize` emits only on a clean report, so a golden rendered from a
/// composition the validator rejects would be a page the command never writes.
fn artifact(project: &str) -> Ir {
    let resolution = resolve(repository().join(project).join("main.yml"));
    assert!(
        resolution.diagnostics.is_empty(),
        "`{project}` does not resolve: {:#?}",
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "`{project}` does not validate: {diagnostics:#?}"
    );
    ir
}

fn document(project: &str) -> GraphDocument {
    graph::graph(&artifact(project))
}

/// The committed bytes are what the compiler answers, for both artifacts.
#[test]
fn every_example_renders_the_committed_artifact() {
    for golden in GOLDENS {
        let document = document(golden.project);
        let mut emitted: Vec<(&str, String)> = vec![(
            golden.document,
            document.to_json().expect("the document serializes"),
        )];
        if let Some(page) = golden.page {
            emitted.push((
                page,
                graph::render(&document).expect("the document renders"),
            ));
        }

        for (name, contents) in emitted {
            let path = goldens_root().join(name);
            if updating() {
                fs::create_dir_all(goldens_root()).expect("the golden directory is writable");
                fs::write(&path, &contents).expect("the golden file is writable");
                continue;
            }
            let committed = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!(
                    "cannot read {}: {error} — regenerate with `UPDATE_GOLDENS=1 cargo test -p \
                     compose-core --test graph_artifact_goldens`",
                    path.display()
                )
            });
            assert_eq!(
                committed, contents,
                "`{name}` is not what the compiler emits; regenerate with `UPDATE_GOLDENS=1 cargo \
                 test -p compose-core --test graph_artifact_goldens` and review the diff"
            );
        }
    }
}

/// The property that makes a golden diff mean anything: identical input renders
/// identically (PRD 5.12).
///
/// From two **separate** resolutions rather than one artifact rendered twice,
/// which is what catches a derivation that had picked up an iteration order
/// from a hash map somewhere behind the IR.
#[test]
fn rendering_the_same_composition_twice_answers_the_same_bytes() {
    for golden in GOLDENS {
        let once = document(golden.project);
        let twice = document(golden.project);
        assert_eq!(
            once.to_json().expect("it serializes"),
            twice.to_json().expect("it serializes"),
            "`{}` produced a different document the second time",
            golden.project
        );
        assert_eq!(
            graph::render(&once).expect("it renders"),
            graph::render(&twice).expect("it renders"),
            "`{}` produced a different page the second time",
            golden.project
        );
    }
}

/// Every committed file is one the corpus knows about.
///
/// The direction that catches a golden left behind by a renamed example: an
/// orphan would keep passing the test above, which starts from the table rather
/// than from the directory.
#[test]
fn no_golden_file_is_an_orphan() {
    if updating() {
        return;
    }
    let known: BTreeSet<&str> = GOLDENS
        .iter()
        .flat_map(|golden| [Some(golden.document), golden.page])
        .flatten()
        .collect();
    let mut found: Vec<String> = fs::read_dir(goldens_root())
        .expect("the golden corpus exists")
        .map(|entry| {
            entry
                .expect("a readable directory entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    found.sort();
    for name in &found {
        assert!(
            known.contains(name.as_str()),
            "`tests/graph-goldens/{name}` belongs to no golden in the corpus table"
        );
    }
    assert_eq!(found.len(), known.len(), "found {found:?}");
}

/// The committed document is the one the committed page carries.
///
/// Two files rendered from one composition can only disagree if the renderer
/// stopped embedding what `--format json` prints — which is the promise PRD
/// resolved q56 makes about the two surfaces ("`--format json` prints exactly
/// that document"), and the reason `graph_version` covers both.
#[test]
fn the_committed_page_carries_the_committed_document() {
    if updating() {
        return;
    }
    for golden in GOLDENS {
        let Some(name) = golden.page else { continue };
        let page = fs::read_to_string(goldens_root().join(name)).expect("the page is committed");
        let committed =
            fs::read_to_string(goldens_root().join(golden.document)).expect("the document too");
        // The page carries the document with `<` escaped and no trailing
        // newline; `to_json` adds one.
        let embedded = committed.trim_end().replace('<', "\\u003c");
        assert!(
            page.contains(&embedded),
            "`{name}` does not carry `{}`",
            golden.document
        );
    }
}

/// Every emitted byte is text a diff tool will print.
///
/// The other half of "goldens are reviewed in PRs like any other code"
/// (CLAUDE.md): git calls a file binary the moment it finds a NUL in the first
/// few kilobytes, and one control character carried out of a `prompt:` would
/// turn a whole page into `Binary files differ`.
#[test]
fn every_rendered_byte_is_text_a_diff_will_print() {
    for golden in GOLDENS {
        let document = document(golden.project);
        for (name, contents) in [
            (golden.document, document.to_json().expect("it serializes")),
            (
                golden.page.unwrap_or("the page"),
                graph::render(&document).expect("it renders"),
            ),
        ] {
            let found = contents.chars().enumerate().find(|(_, character)| {
                character.is_control() && !matches!(character, '\t' | '\n' | '\r')
            });
            assert!(
                found.is_none(),
                "`{name}` carries U+{:04X} at character {}",
                found.expect("a control character").1 as u32,
                found.expect("a control character").0,
            );
        }
    }
}
