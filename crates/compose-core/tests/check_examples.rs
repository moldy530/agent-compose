//! The positive corpus for the data checks: both example projects check clean.
//!
//! The examples are grammar-conformant by construction — `docs/grammar.md`
//! points at them for every construct it defines — so a check firing on one is
//! evidence about the *check*, not about the example.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("manifest dir has a grandparent")
        .to_path_buf()
}

fn render(diagnostics: &[compose_core::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            let mut rendered = format!(
                "  {} [{}] {}",
                diagnostic.span, diagnostic.code, diagnostic.message
            );
            for label in &diagnostic.labels {
                rendered.push_str(&format!("\n      {} {}", label.span, label.message));
            }
            if let Some(help) = &diagnostic.help {
                rendered.push_str(&format!("\n      help: {help}"));
            }
            rendered
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_example_checks_clean() {
    for (project, target) in [
        ("examples/review-loop", "local"),
        ("examples/triage-fanout", "local"),
        ("examples/triage-fanout", "staging"),
    ] {
        let entrypoint = repo_root().join(project).join("main.yml");
        let resolution = compose_core::resolve_with_target(&entrypoint, target);
        let ir = resolution.ir.expect("the example resolves");
        let diagnostics = compose_core::check(&ir);
        assert!(
            diagnostics.is_empty(),
            "{project} --target {target} produced {} diagnostic(s):\n{}",
            diagnostics.len(),
            render(&diagnostics)
        );
    }
}
