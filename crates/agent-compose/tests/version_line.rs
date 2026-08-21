//! What `agent-compose --version` answers.
//!
//! The line is a **support surface**, not decoration. A released binary is
//! downloaded rather than built (PRD §7 M2), the `unsupported-version`
//! explanation sends a reader here to say which build they are holding, and the
//! release pipeline's smoke test asks this question of every artifact it
//! packages — so its shape is pinned by tests rather than by habit.
//!
//! **Shape, never the sha.** The commit is whatever the build environment knew,
//! and the three answers it can give — a sha from `GIT_SHA`, a sha from the
//! repository, `unknown` from a source tarball with no `.git` beside it — are
//! all correct. A test that pinned one of them would be a test that fails in a
//! release container, or in a checkout, or in a vendored build. So what is held
//! here is everything *around* the sha: the program name, the crate version
//! exactly, the target triple's architecture, the punctuation between them, and
//! that the sha is a commit name or the word that says there wasn't one.

use std::process::Output;

use assert_cmd::Command;

/// Ask the binary for its version, through the flag under test.
fn version(flag: &str) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .env("NO_COLOR", "1")
        .arg(flag)
        .output()
        .expect("the command runs")
}

#[track_caller]
fn line(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(0),
        "`--version` is a question with an answer, not a report"
    );
    assert_eq!(
        std::str::from_utf8(&output.stderr).expect("stderr is UTF-8"),
        "",
        "the answer goes to stdout alone"
    );
    let printed = std::str::from_utf8(&output.stdout).expect("stdout is UTF-8");
    let (first, rest) = printed
        .split_once('\n')
        .expect("the version line ends with a newline");
    assert_eq!(rest, "", "the version is one line: {printed:?}");
    first.to_string()
}

/// The three facts, in the order and the punctuation the line commits to:
/// `agent-compose <version> (<sha>, <target>)`.
///
/// The crate version is held **exactly** — it is the number a release tag is
/// checked against by the release workflow's guard, so a version line that
/// disagreed with `Cargo.toml` would make that guard check the wrong thing.
#[test]
fn the_version_line_names_the_crate_version_the_commit_and_the_target() {
    let printed = line(&version("--version"));

    let rest = printed
        .strip_prefix("agent-compose ")
        .unwrap_or_else(|| panic!("the line opens with the program name: {printed:?}"));
    let (crate_version, build) = rest
        .split_once(' ')
        .unwrap_or_else(|| panic!("the version is followed by the build it came from: {rest:?}"));
    assert_eq!(
        crate_version,
        env!("CARGO_PKG_VERSION"),
        "the line names this crate's version: {printed:?}"
    );

    let build = build
        .strip_prefix('(')
        .and_then(|held| held.strip_suffix(')'))
        .unwrap_or_else(|| panic!("the build is parenthesized: {printed:?}"));
    let (sha, target) = build
        .split_once(", ")
        .unwrap_or_else(|| panic!("the build is the commit, then the target: {build:?}"));

    assert!(
        sha == "unknown"
            || (!sha.is_empty()
                && sha.len() <= 7
                && sha
                    .chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())),
        "the commit is a short lowercase sha, or the word for not having one: {sha:?}"
    );

    // The tests and the binary under test are built for the same target, so the
    // architecture is one the test process can name. The rest of the triple —
    // vendor, system, ABI — is not: `macos` is spelled `darwin` in a triple, and
    // a `musl` build and a `gnu` build of one host are the same
    // `std::env::consts`.
    assert!(
        target.starts_with(std::env::consts::ARCH),
        "the target triple opens with the architecture ({}): {target:?}",
        std::env::consts::ARCH
    );
    assert!(
        !target.contains(char::is_whitespace) && target.matches('-').count() >= 2,
        "the target is a triple: {target:?}"
    );
}

/// `-V` and `--version` are the same question.
///
/// Both are clap's, and a `version` set to a composed string rather than left to
/// the default is exactly the kind of change that could reach one and not the
/// other.
#[test]
fn the_short_flag_answers_with_the_same_line() {
    assert_eq!(line(&version("-V")), line(&version("--version")));
}
