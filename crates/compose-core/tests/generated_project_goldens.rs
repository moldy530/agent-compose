//! The golden-file codegen corpus: same DSL in, byte-identical TypeScript out
//! (PRD 5.12, CLAUDE.md's *Golden-file codegen tests*).
//!
//! # What a golden is for
//!
//! Two different things, and both matter:
//!
//! * **A regression fence.** Every emitted byte is committed, so a change to the
//!   emitter shows up as a reviewable diff over real projects rather than as a
//!   passing unit test about a rule nobody reads.
//! * **A review surface.** CLAUDE.md asks for goldens to be "reviewed in PRs like
//!   any other code". They are the only place the generated TypeScript is legible
//!   as TypeScript instead of as string literals inside a Rust module.
//!
//! # Regenerating
//!
//! ```sh
//! UPDATE_GOLDENS=1 cargo test -p compose-core --test generated_project_goldens
//! ```
//!
//! Then read the diff. A compiler release that repins its LangGraph version
//! (PRD 5.12) rewrites every header and so touches every file: that is the review
//! the pin is supposed to get, not noise to suppress.

#[path = "support/goldens.rs"]
mod goldens;

use std::collections::BTreeSet;
use std::fs;

use goldens::{GOLDENS, emitted, files_under, goldens_root};

fn updating() -> bool {
    std::env::var_os("UPDATE_GOLDENS").is_some_and(|value| !value.is_empty())
}

/// Whether a test that *reads* the committed corpus should run.
///
/// Under `UPDATE_GOLDENS` one test is rewriting those directories while the
/// others would be reading them, on cargo's parallel threads: every read is a
/// race against a `remove_dir_all`. Regeneration is a maintenance action rather
/// than a test run, so the readers stand down and the very next `cargo test`
/// — the one whose diff is the point — runs all of them.
fn regenerating() -> bool {
    if updating() {
        eprintln!("note: UPDATE_GOLDENS is set; this test reads the corpus and is standing down");
        return true;
    }
    false
}

/// The committed project is what the compiler emits, byte for byte.
#[test]
fn every_example_emits_the_committed_project() {
    for golden in GOLDENS {
        let project = emitted(golden);
        let root = goldens_root().join(golden.directory);

        if updating() {
            let _ = fs::remove_dir_all(&root);
            for file in project.files() {
                let path = root.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
                fs::create_dir_all(path.parent().expect("a generated path has a parent"))
                    .expect("the golden directory is writable");
                fs::write(&path, &file.contents).expect("the golden file is writable");
            }
            continue;
        }

        assert!(
            root.is_dir(),
            "the golden project `{}` is not committed; regenerate with \
             `UPDATE_GOLDENS=1 cargo test -p compose-core --test generated_project_goldens`",
            golden.directory
        );

        let committed: BTreeSet<String> = files_under(&root).into_iter().collect();
        let generated: BTreeSet<String> = project.paths().map(str::to_string).collect();
        assert_eq!(
            generated, committed,
            "`{}` emits a different file set than is committed",
            golden.directory
        );

        for file in project.files() {
            let path = root.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
            let bytes = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            assert_eq!(
                bytes, file.contents,
                "`{}/{}` is not what the compiler emits; regenerate with \
                 `UPDATE_GOLDENS=1 cargo test -p compose-core --test generated_project_goldens` \
                 and review the diff",
                golden.directory, file.path
            );
        }
    }
}

/// The property that makes a golden diff mean anything: identical input
/// regenerates identically (PRD 5.12).
///
/// Emitting twice from one artifact would be the weaker half. This emits from two
/// *separate* resolutions of the same files, which is what catches an emitter
/// that had picked up an iteration order from a hash map somewhere behind the IR.
#[test]
fn emitting_the_same_composition_twice_answers_the_same_bytes() {
    for golden in GOLDENS {
        let once = emitted(golden);
        let twice = emitted(golden);
        assert_eq!(
            once.paths().collect::<Vec<_>>(),
            twice.paths().collect::<Vec<_>>(),
            "`{}` emitted a different file set the second time",
            golden.directory
        );
        for (left, right) in once.files().iter().zip(twice.files()) {
            assert_eq!(
                left.contents, right.contents,
                "`{}/{}` differs between two emissions of the same composition",
                golden.directory, left.path
            );
        }
    }
}

/// Every golden carries the marker that says it is not source (PRD §8).
#[test]
fn every_golden_file_says_it_is_generated() {
    if regenerating() {
        return;
    }
    for golden in GOLDENS {
        let root = goldens_root().join(golden.directory);
        assert!(root.is_dir(), "`{}` is not committed", golden.directory);
        for path in files_under(&root) {
            let text = fs::read_to_string(root.join(&path)).expect("a golden file is readable");
            assert!(
                text.contains("generated by agent-compose"),
                "`{}/{path}` carries no generated-file header",
                golden.directory
            );
        }
    }
}

/// Every emitted byte is text a diff tool will show.
///
/// This is the other half of "goldens are reviewed in PRs like any other code"
/// (CLAUDE.md): a committed file is only a review surface while `git diff` is
/// willing to print it, and git calls a file binary the moment it finds a NUL in
/// the first few kilobytes — one control character inside one string literal is
/// enough to turn a whole emitted module into `Binary files differ`. TypeScript
/// admits those characters in source, so nothing else would catch it; a codepoint
/// that needs to *be* in a string belongs there as an escape.
///
/// Tab, newline and carriage return are the three a text file legitimately holds.
/// It emits rather than reads, so a regression fails here on the run that
/// introduced it instead of on the run that regenerated the corpus.
#[test]
fn every_emitted_file_is_text_a_diff_will_print() {
    for golden in GOLDENS {
        for file in emitted(golden).files() {
            let found = file.contents.chars().enumerate().find(|(_, character)| {
                character.is_control() && !matches!(character, '\t' | '\n' | '\r')
            });
            assert!(
                found.is_none(),
                "`{}/{}` carries U+{:04X} at character {}: write it as an escape, \
                 or a diff of this file reads `Binary files differ`",
                golden.directory,
                file.path,
                found.expect("a control character").1 as u32,
                found.expect("a control character").0,
            );
        }
    }
}

/// The corpus reaches both targets of the example that has two.
///
/// A golden set that only ever emitted `local` would not notice a deploy layer
/// the emitter had stopped reading — and the deploy layer is where a target's
/// environment references come from (PRD 5.8, 5.9).
#[test]
fn the_corpus_covers_more_than_one_target() {
    let targets: BTreeSet<&str> = GOLDENS.iter().map(|golden| golden.target).collect();
    assert!(targets.len() > 1, "every golden is built for one target");

    let local = emitted(goldens::golden("triage-fanout"));
    let staging = emitted(goldens::golden("triage-fanout-staging"));
    let local_env = &local.file("src/env.ts").expect("an env module").contents;
    let staging_env = &staging.file("src/env.ts").expect("an env module").contents;
    assert!(
        !local_env.contains("CHROMA_URL"),
        "`local` substitutes local disk for every store, so it binds no backend"
    );
    assert!(
        staging_env.contains("CHROMA_URL") && staging_env.contains("REDIS_URL"),
        "the deploy layer's own environment references are missing from the target that has them"
    );
}

/// Every committed golden directory is one the corpus knows about.
///
/// The direction that catches a golden left behind by a renamed example: an
/// orphan would keep passing every test above, because every one of them starts
/// from the table rather than from the directory.
#[test]
fn no_golden_directory_is_an_orphan() {
    if regenerating() {
        return;
    }
    let known: BTreeSet<&str> = GOLDENS.iter().map(|golden| golden.directory).collect();
    let mut found: Vec<String> = Vec::new();
    for entry in fs::read_dir(goldens_root()).expect("the golden corpus exists") {
        let entry = entry.expect("a readable directory entry");
        if entry.path().is_dir() {
            found.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    found.sort();
    for directory in &found {
        assert!(
            known.contains(directory.as_str()),
            "`tests/goldens/{directory}` belongs to no golden in the corpus table"
        );
    }
    assert_eq!(found.len(), GOLDENS.len(), "found {found:?}");
}
