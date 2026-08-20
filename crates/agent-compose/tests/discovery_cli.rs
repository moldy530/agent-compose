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

/// The loop's first two steps, back to back, through the real binary: `init`
/// then `validate`.
///
/// This is the one property the scaffold has, and it is worth running through
/// the command rather than over the constant — an agent's very first
/// `validate` must not be a report about this compiler's own file.
#[test]
fn a_scaffolded_project_validates_clean() {
    let directory = scratch("init-then-validate");
    let initialized = run(&directory, &["init"]);
    assert_eq!(code(&initialized), 0, "stderr: {}", stderr(&initialized));
    assert_eq!(stdout(&initialized), "", "the file is the answer");
    assert!(
        stderr(&initialized).contains("agent-compose validate"),
        "the next step is named: {}",
        stderr(&initialized)
    );

    let validated = run(&directory, &["validate", "main.yml"]);
    assert_eq!(
        code(&validated),
        0,
        "the scaffold validates: {}",
        stderr(&validated)
    );
    assert_eq!(stderr(&validated), "`main.yml` is valid (target `local`)\n");
}

/// A named directory that does not exist is created; an argument is not a
/// second way to spell the current directory.
#[test]
fn init_creates_the_directory_it_is_named() {
    let directory = scratch("init-names-a-child");
    let output = run(&directory, &["init", "summarizer"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let scaffold = directory.join("summarizer/main.yml");
    assert!(scaffold.is_file(), "the scaffold landed under the name");
    assert_eq!(
        std::fs::read_to_string(&scaffold).expect("readable"),
        compose_core::docs::SCAFFOLD
    );
}

/// A directory holding anything at all is refused, with the exit code that
/// means *the answer is no* rather than the one that means *the command could
/// not run*, and with the remedy in the message (PRD G3).
#[test]
fn init_refuses_a_directory_that_holds_anything() {
    let directory = scratch("init-refuses-occupied");
    std::fs::write(directory.join("notes.md"), "mine\n").expect("can write");
    let output = run(&directory, &["init"]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    let reported = stderr(&output);
    assert!(
        reported.contains("`notes.md`"),
        "names what it found: {reported}"
    );
    assert!(
        reported.contains("agent-compose init <name>"),
        "names the remedy: {reported}"
    );
    assert!(
        !directory.join("main.yml").exists(),
        "a refusal writes nothing"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("notes.md")).expect("readable"),
        "mine\n",
        "and touches nothing"
    );
}
