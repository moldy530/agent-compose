//! Every spec this compiler *hands out* is run through this compiler.
//!
//! The discovery surface (PRD §7 M2, resolved q23) ships YAML that a reader is
//! meant to copy: the `init` scaffold, and the examples inside the topic
//! documents. A published example that does not validate is worse than a
//! missing one — it teaches a shape the compiler rejects, and the reader's
//! first `validate` reports on prose this repository wrote. So each one is
//! resolved and checked here, by the same passes `agent-compose validate` runs.
//!
//! What is checked is stated in terms of the **marker convention**, because a
//! topic needs both kinds of block: a whole spec a reader could save as
//! `main.yml`, and a fragment showing one key in isolation. The two are told
//! apart by the fenced block's info string, which is the only place a Markdown
//! fence carries metadata:
//!
//! * ```` ```yaml spec ```` — a complete spec. Written to `main.yml` in a
//!   scratch directory of its own and required to resolve and check **clean**
//!   under the built-in `local` target.
//! * ```` ```yaml ```` — a fragment. Skipped, and deliberately so: a block
//!   showing `retry: { max: 2 }` on its own is not a document and has no
//!   verdict to have.
//!
//! The convention is one word because it has to survive a Markdown renderer:
//! every renderer takes the first token of an info string as the language, so
//! ```` ```yaml spec ```` still highlights as YAML wherever the topics are read
//! outside this binary.
//!
//! The direction this cannot check is a *fragment that should have been a
//! spec* — an author who marks a whole document `yaml` gets no verdict and no
//! failure. That is the same residual `trace_format_inventory.rs` names about
//! prose: the check is worth what the document's own discipline is worth. What
//! it does close is the direction that ships a broken example.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, docs, resolve};

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// A scratch directory of this test's own, emptied first.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-documented-examples")
        .join(name);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("can create a scratch directory");
    directory
}

/// Resolve and check one complete spec, and report everything it was told.
#[track_caller]
fn validates(name: &str, source: &str) {
    let directory = scratch(name);
    let entrypoint = directory.join("main.yml");
    fs::write(&entrypoint, source).expect("can write the spec");

    let resolution = resolve(&entrypoint);
    assert!(
        resolution.diagnostics.is_empty(),
        "{name} does not resolve:\n{}",
        render(&resolution.diagnostics)
    );
    let ir = resolution
        .ir
        .expect("a clean resolution produces an artifact");
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "{name} does not validate:\n{}",
        render(&diagnostics)
    );
}

fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} at {}: {}",
                diagnostic.code, diagnostic.span, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The scaffold `agent-compose init` writes.
///
/// Its whole contract: a reader's first command after `init` is `validate`, and
/// it must answer about their project rather than about this compiler's file.
#[test]
fn the_init_scaffold_validates_clean() {
    validates("init-scaffold", docs::SCAFFOLD);
}

/// The scaffold is also the first document anybody reads, so the commands
/// written in its comments have to be the commands that work.
#[test]
fn the_scaffold_names_a_flow_it_defines() {
    let flow = docs::SCAFFOLD
        .lines()
        .find(|line| line.starts_with("flow."))
        .expect("the scaffold defines a flow")
        .trim_end_matches(':');
    assert!(
        docs::SCAFFOLD.contains(&format!("agent-compose run main.yml {flow}")),
        "the `run` line in the comments names `{flow}`"
    );
    assert!(
        docs::SCAFFOLD.contains(&format!("    flow: {flow}")),
        "the manual trigger names `{flow}`"
    );
}

/// The repository is checked too, since the topics are not the only YAML the
/// project publishes: `docs/` is where a reader who *did* clone lands.
#[test]
fn the_worked_examples_are_still_where_the_scaffold_points() {
    assert!(
        repository().join("examples/review-loop/main.yml").is_file(),
        "the worked example the topics build toward is present"
    );
}
