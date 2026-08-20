//! The discovery verbs, end to end through the real binary.
//!
//! `schema`, `init`, `explain`, `docs` and `skill` are the surface PRD §7 M2
//! calls progressive discovery: what a coding agent holding nothing but a
//! released binary can ask (resolved q23). Their contract is small and entirely
//! observable from outside — which stream a document goes to, what a name
//! nobody defines does, and which file a write lands in — so it is pinned here
//! rather than in a unit test that could agree with itself about a verb the
//! command line never routes to.
//!
//! Every document these verbs print is embedded, and
//! `tests/discovery_surface_inventory.rs` is what holds the documents to the
//! compiler they describe. This file holds the *command*.

use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

/// The repository root, which is where the published schema is committed.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// Run the binary in `directory` with `arguments`.
fn run(directory: &Path, arguments: &[&str]) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .args(arguments)
        .output()
        .expect("the command runs")
}

#[track_caller]
fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("stdout is UTF-8")
}

#[track_caller]
fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).expect("stderr is UTF-8")
}

#[track_caller]
fn code(output: &Output) -> i32 {
    output.status.code().expect("the command was not signalled")
}

/// A scratch directory of this test's own, emptied first so a rerun starts
/// where the first run did.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-discovery-cli")
        .join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("can create a scratch directory");
    directory
}

/// `schema` writes the committed document to **stdout**, byte for byte, and
/// exits clean.
///
/// Stdout because the document is the answer — `agent-compose schema >
/// schema.json` is the whole point of the verb, and a byte of prose on that
/// stream would corrupt the file it redirects into.
#[test]
fn schema_writes_the_published_document_to_stdout() {
    let committed = std::fs::read_to_string(repo_root().join("schemas/agent-compose.schema.json"))
        .expect("the published schema is readable");
    let output = run(&repo_root(), &["schema"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), committed);
    assert_eq!(stderr(&output), "");
}

/// The redirect the verb exists for, performed: what lands on disk parses as
/// the schema it claims to be.
#[test]
fn a_redirected_schema_is_a_usable_file() {
    let directory = scratch("schema-redirect");
    let output = run(&directory, &["schema"]);
    let written = directory.join("schema.json");
    std::fs::write(&written, stdout(&output)).expect("can write the redirect");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&written).expect("readable"))
            .expect("the redirected document is JSON");
    assert_eq!(
        parsed["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "the redirect carries the dialect an editor reads"
    );
}
