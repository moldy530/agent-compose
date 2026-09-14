//! `docs/graph.md` against the document the compiler actually writes.
//!
//! The same bind `tests/trace_format_inventory.rs` puts on `docs/trace.md`, over
//! the surface PRD resolved q56 adds: a graph document is what a consumer pins
//! `graph_version` on, so the account of it may not rot behind the code. A
//! derivative that drifts silently is the whole failure class — the document
//! keeps saying what the format used to be, and every reader who believed it is
//! wrong in a way nothing tells them about.
//!
//! # The binds
//!
//! **(a) The envelope, both ways round.** The top-level keys the compiler writes
//! and the fields §2's table names are the **same set**. A key the compiler adds
//! and nobody documents is a field nobody knows to read; a field the document
//! names and the compiler stopped writing is worse, because a reader who follows
//! the documented shape concludes an absent key is a statement about their
//! composition.
//!
//! **(b) Every record type is specified somewhere.** Every `pub struct` of
//! `src/graph/document.rs` is named — as a bare backticked type name — in the
//! section that specifies it. A record type reachable from the envelope and
//! written down nowhere is a whole object a reader meets with no account of it.
//!
//! **(c) Every closed vocabulary is written out.** §9.1 makes a reader entitled
//! to the members of each enumeration, and §9.3 makes adding one a version bump
//! — both of which are promises about a list that has to be *in* the document.
//! The members are read out of the compiler: each vocabulary's `ALL` is written
//! by the declaration of the enumeration itself (see the `vocabulary!` macro in
//! `src/graph/document.rs`), so a member cannot be missing from the list this
//! runs over. A list kept by hand beside the enumeration would have made this
//! test pass on exactly the change it exists to catch — a new member, documented
//! nowhere, and §9.3's bump never asked for.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::graph::{EdgeClass, NodeKind, PolicyLevel, SchemaSource, ToolSource};
use compose_core::resolve;

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

fn specification() -> String {
    fs::read_to_string(repository().join("docs/graph.md"))
        .expect("`docs/graph.md` is readable — the graph document's normative account")
}

/// The graph document of a worked example, as JSON.
fn document() -> serde_json::Value {
    let resolution = resolve(repository().join("examples/triage-fanout/main.yml"));
    let ir = resolution.ir.expect("the example resolves");
    let document = compose_core::graph(&ir);
    serde_json::from_str(&document.to_json().expect("it serializes")).expect("it is JSON")
}

/// The fields one section's tables name, read out of the first column.
///
/// A table row is `| `name` | type | presence | meaning |`, which is the shape
/// every field table in that document has — the same layout `docs/trace.md`
/// fixes and the same reason: a field explained only in the surrounding prose is
/// an undocumented field, and the fix is a row.
fn documented_fields(specification: &str, heading: &str) -> BTreeSet<String> {
    let from = specification
        .find(heading)
        .unwrap_or_else(|| panic!("`docs/graph.md` has a `{heading}` section"));
    let rest = &specification[from + heading.len()..];
    let section = rest.find("\n## ").map_or(rest, |at| &rest[..at]);
    section
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split_once('`'))
        .map(|(name, _)| name.to_string())
        .collect()
}

/// (a) The envelope's fields and the document's top-level keys are one set.
#[test]
fn the_envelope_and_the_document_name_the_same_top_level_fields() {
    let documented = documented_fields(&specification(), "\n## 2. The envelope\n");
    let written: BTreeSet<String> = document()
        .as_object()
        .expect("the document is one object")
        .keys()
        .cloned()
        .collect();
    assert!(
        written.len() >= 5,
        "the document still has its keys, found {written:?}"
    );
    assert_eq!(
        documented, written,
        "`docs/graph.md` §2 and the emitted document name different top-level fields. Add the row, \
         or remove it — §9.3 makes adding and removing a key a version bump either way"
    );
}

/// (a) …and `graph_version` is the first of them.
///
/// §2 promises a consumer can dispatch on the version before reading anything
/// else, which is a promise about the *order* of the keys and not only about
/// their presence.
#[test]
fn the_version_is_the_first_key_the_document_writes() {
    let resolution = resolve(repository().join("examples/review-loop/main.yml"));
    let ir = resolution.ir.expect("the example resolves");
    let json = compose_core::graph(&ir)
        .to_json()
        .expect("the document serializes");
    assert!(
        json.starts_with("{\n  \"graph_version\": "),
        "the document opens with its version: {}",
        &json[..json.len().min(80)]
    );
}

/// (b) Every record type is named in the document that specifies it.
#[test]
fn every_record_type_is_specified() {
    let specification = specification();
    let source = fs::read_to_string(repository().join("crates/compose-core/src/graph/document.rs"))
        .expect("the record types are readable");

    let mut found = 0;
    for line in source.lines() {
        let Some(rest) = line.strip_prefix("pub struct ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|held| held.is_alphanumeric())
            .collect();
        assert!(!name.is_empty(), "a declaration with no name: {line}");
        assert!(
            specification.contains(&format!("`{name}`")),
            "`docs/graph.md` does not specify `{name}`. Every record type gets a section that \
             names it and a table for its fields (§2's own layout)"
        );
        found += 1;
    }
    assert!(
        found >= 20,
        "the scan still finds the record types, found {found}"
    );
}

/// (c) Every member of every closed vocabulary is written out.
///
/// Read out of the enumerations rather than listed here: every `ALL` below is
/// written by the same declaration as the enumeration it belongs to, so a
/// variant added to the compiler is a variant this test demands `docs/graph.md`
/// name — and somebody has to decide whether §9.3's bump is owed before it
/// ships. The page's own side of that bind is in `src/graph/document.rs`, where
/// `NodeKind::ALL` is held to the template's `KIND` table and `EdgeClass::ALL` to
/// its `EDGE` table.
#[test]
fn every_closed_vocabulary_is_written_out() {
    let specification = specification();
    let vocabularies: Vec<(&str, Vec<String>)> = vec![
        ("GraphNode.kind", members(NodeKind::ALL)),
        ("GraphEdge.class", members(EdgeClass::ALL)),
        ("SchemaView.source", members(SchemaSource::ALL)),
        ("RetryView.level", members(PolicyLevel::ALL)),
        ("ToolView.source", members(ToolSource::ALL)),
    ];

    for (vocabulary, members) in vocabularies {
        assert!(
            specification.contains(vocabulary),
            "`docs/graph.md` §9.1 does not name the `{vocabulary}` vocabulary"
        );
        for held in members {
            // Either spelling the document uses: a quoted member in a type
            // column, or a bare one in a table of its own.
            assert!(
                specification.contains(&format!("`\"{held}\"`"))
                    || specification.contains(&format!("`{held}`")),
                "`docs/graph.md` does not write out `{held}`, a member of `{vocabulary}`. §9.1 \
                 makes a reader entitled to the members and §9.3 makes adding one a version bump"
            );
        }
    }
}

/// The serde names of one vocabulary's members — what the document actually
/// carries.
fn members<T: serde::Serialize>(all: &[T]) -> Vec<String> {
    all.iter()
        .map(|value| {
            serde_json::to_value(value)
                .expect("a unit variant serializes")
                .as_str()
                .expect("as a string")
                .to_string()
        })
        .collect()
}

/// The document's own version and the constant the compiler writes agree.
#[test]
fn the_documented_version_is_the_one_the_compiler_writes() {
    let specification = specification();
    let expected = format!("**Graph version:** `{}`", compose_core::GRAPH_VERSION);
    assert!(
        specification.contains(&expected),
        "`docs/graph.md`'s header does not read `{expected}`"
    );
    assert!(
        specification.contains(&format!(
            "`{}` is this document",
            compose_core::GRAPH_VERSION
        )),
        "…nor does §2's row for `graph_version`"
    );
}

/// §9.4 names the tests that hold this document to the code, and they exist.
///
/// A stability section that pointed at a test nobody wrote would be the same
/// failure one layer up: the promise is checkable only if the check is there.
#[test]
fn the_stability_section_names_tests_that_exist() {
    let specification = specification();
    let mut named = 0;
    for line in specification.lines() {
        for at in line.match_indices("`crates/").map(|(at, _)| at) {
            let path = line[at + 1..]
                .split('`')
                .next()
                .expect("a closing backtick");
            if !path.ends_with(".rs") {
                continue;
            }
            assert!(
                repository().join(path).is_file(),
                "`docs/graph.md` names `{path}`, which is not a file"
            );
            named += 1;
        }
    }
    assert!(
        named >= 4,
        "§9.4 still names the tests that hold the two together, found {named}"
    );
}
