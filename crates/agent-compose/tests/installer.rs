//! `install.sh`, run.
//!
//! PRD §7 M2's exit criterion opens with "a user downloads a released binary",
//! and the script at the repository root is that sentence's implementation. It
//! is exercised for real by the release pipeline's dry run, against artifacts
//! built minutes earlier — but that gate lives in a workflow, so nothing here
//! would catch a broken installer until CI ran, and nothing at all would catch
//! the installer and the workflows disagreeing about what an artifact is
//! *called*. This file closes both gaps with the shell script itself under test.
//!
//! **A release, faked precisely.** Each test builds a directory that looks like
//! one — four `agent-compose-<version>-<target>.tar.gz` archives and a
//! `SHA256SUMS` over them — and points the script at it with
//! `AGENT_COMPOSE_ARTIFACT_DIR`, the local-artifact mode the dry-run gate uses.
//! The binary inside each archive is a shell script that names the target it
//! stands for, so "which artifact did the installer choose?" is answered by
//! what the installed file *says* rather than by which file was copied.
//!
//! **A machine, faked precisely.** The script asks `uname` what it is running
//! on. Prepending a `uname` of our own to `PATH` is what lets one Linux x86_64
//! test machine answer as all six machines a release supports — which is the
//! only way the arm64/aarch64 and amd64/x86_64 spellings, and the refusal for a
//! machine no release covers, are checkable at all outside a fleet.

#![cfg(unix)]

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

/// The four artifacts a release publishes, and the four the installer knows how
/// to ask for. Both sides are held to this list.
const TARGETS: [&str; 4] = [
    "aarch64-apple-darwin",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-musl",
];

/// The version the fixture release carries. Deliberately not this crate's: what
/// the installer does is decided by the names in the directory it was pointed
/// at, and a fixture wearing the real version could hide a test that quietly
/// depends on the build under it.
const RELEASED: &str = "1.2.3";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// A directory the test writes into.
fn scratch(purpose: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "agent-compose-installer-{purpose}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("a scratch directory");
    path
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("a writable scratch file");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("a settable mode");
}

/// Run a command in `directory` and insist it worked.
#[track_caller]
fn run(directory: &Path, program: &str, arguments: &[&str]) {
    let output = Command::new(program)
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap_or_else(|error| panic!("`{program}` runs: {error}"));
    assert!(
        output.status.success(),
        "`{program} {}` failed: {}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A directory that looks like a published release: one archive per target in
/// `versions`, and a `SHA256SUMS` over all of them.
fn release(purpose: &str, versions: &[&str]) -> PathBuf {
    let directory = scratch(purpose);
    for version in versions {
        for target in TARGETS {
            let stage = directory.join(format!("stage-{version}-{target}"));
            fs::create_dir_all(&stage).expect("a stage directory");
            write_executable(
                &stage.join("agent-compose"),
                &format!("#!/bin/sh\necho \"agent-compose {version} (fixture, {target})\"\n"),
            );
            let archive = format!("agent-compose-{version}-{target}.tar.gz");
            run(
                &directory,
                "tar",
                &[
                    "-czf",
                    &archive,
                    "-C",
                    stage.to_str().expect("a UTF-8 scratch path"),
                    "agent-compose",
                ],
            );
            fs::remove_dir_all(&stage).expect("the stage is removable");
        }
    }
    checksum(&directory);
    directory
}

/// Write the `SHA256SUMS` the release publishes, the way the release writes it.
fn checksum(directory: &Path) {
    run(
        directory,
        "sh",
        &[
            "-c",
            "if command -v sha256sum > /dev/null 2>&1; then \
               sha256sum agent-compose-*.tar.gz > SHA256SUMS; \
             else \
               shasum -a 256 agent-compose-*.tar.gz > SHA256SUMS; \
             fi",
        ],
    );
}

/// A `uname` that answers as `system`/`machine`, in a directory to put on
/// `PATH` ahead of the real one.
fn machine(purpose: &str, system: &str, machine: &str) -> PathBuf {
    let directory = scratch(purpose);
    write_executable(
        &directory.join("uname"),
        &format!(
            "#!/bin/sh\ncase \"$1\" in\n  -s) echo {system} ;;\n  -m) echo {machine} ;;\n  \
             *) echo \"the fixture uname was asked for $1\" >&2; exit 1 ;;\nesac\n"
        ),
    );
    directory
}

/// Run `install.sh` against `artifacts`, as `uname` on `path_first`, installing
/// into `into`.
fn install(artifacts: &Path, into: &Path, path_first: &Path, arguments: &[&str]) -> Output {
    let inherited = std::env::var("PATH").unwrap_or_default();
    Command::new("sh")
        .arg(repo_root().join("install.sh"))
        .args(arguments)
        .env(
            "PATH",
            format!("{}:{inherited}", path_first.to_str().expect("a UTF-8 path")),
        )
        .env("AGENT_COMPOSE_ARTIFACT_DIR", artifacts)
        .env("AGENT_COMPOSE_INSTALL", into)
        .env_remove("AGENT_COMPOSE_VERSION")
        .output()
        .expect("the installer runs")
}

#[track_caller]
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[track_caller]
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Every machine a release covers is handed the binary built for it.
///
/// Six machines and four artifacts, because two architectures answer to two
/// names each: `uname -m` says `arm64` on macOS and `aarch64` on Linux for the
/// same architecture, and `amd64` where some systems say `x86_64`. A user on
/// the wrong side of either spelling gets no binary at all, which is why the
/// mapping is a table here rather than a reading of the script.
#[test]
fn every_machine_a_release_covers_is_handed_its_own_binary() {
    let artifacts = release("machines", &[RELEASED]);
    for (system, reported, target) in [
        ("Linux", "x86_64", "x86_64-unknown-linux-musl"),
        ("Linux", "amd64", "x86_64-unknown-linux-musl"),
        ("Linux", "aarch64", "aarch64-unknown-linux-musl"),
        ("Linux", "arm64", "aarch64-unknown-linux-musl"),
        ("Darwin", "x86_64", "x86_64-apple-darwin"),
        ("Darwin", "arm64", "aarch64-apple-darwin"),
    ] {
        let into = scratch(&format!("into-{system}-{reported}"));
        let output = install(
            &artifacts,
            &into,
            &machine(&format!("uname-{system}-{reported}"), system, reported),
            &[],
        );
        assert!(
            output.status.success(),
            "{system}/{reported}: {}{}",
            stdout(&output),
            stderr(&output)
        );
        // The script runs what it installed, so its own output is the installed
        // binary's answer — which names the artifact it came out of.
        assert!(
            stdout(&output).contains(&format!("agent-compose {RELEASED} (fixture, {target})")),
            "{system}/{reported} should have installed the {target} build: {}",
            stdout(&output)
        );
        assert!(
            into.join("agent-compose").is_file(),
            "{system}/{reported} left nothing behind"
        );
    }
}

/// A machine no release covers is told so, by the name it gave.
#[test]
fn a_machine_no_release_covers_is_refused_by_the_name_it_gave() {
    let artifacts = release("unsupported", &[RELEASED]);
    for (system, reported, named) in [
        ("SunOS", "x86_64", "SunOS"),
        ("Linux", "riscv64", "riscv64"),
    ] {
        let into = scratch(&format!("unsupported-{system}-{reported}"));
        let output = install(
            &artifacts,
            &into,
            &machine(&format!("uname-bad-{system}-{reported}"), system, reported),
            &[],
        );
        assert!(
            !output.status.success(),
            "{system}/{reported} should not have installed anything"
        );
        assert!(
            stderr(&output).contains(named),
            "the refusal should name `{named}`: {}",
            stderr(&output)
        );
        assert!(
            !into.join("agent-compose").exists(),
            "{system}/{reported} left a binary behind"
        );
    }
}

/// Bytes the checksum does not vouch for are not installed.
///
/// Both ways a `SHA256SUMS` can fail to vouch, because they are different
/// failures with the same consequence: a line for this archive that disagrees,
/// and no line for this archive at all. The second is the one a naive
/// implementation gets wrong — a missing line reads as "nothing to check".
#[test]
fn a_checksum_that_does_not_vouch_for_the_archive_installs_nothing() {
    let target = "x86_64-unknown-linux-musl";
    for (purpose, line, expected) in [
        (
            "mismatch",
            format!(
                "{}  agent-compose-{RELEASED}-{target}.tar.gz\n",
                "0".repeat(64)
            ),
            "checksum mismatch",
        ),
        (
            "silent",
            format!("{}  some-other-artifact.tar.gz\n", "0".repeat(64)),
            "says nothing about",
        ),
    ] {
        let artifacts = release(purpose, &[RELEASED]);
        fs::write(artifacts.join("SHA256SUMS"), &line).expect("the checksum file is writable");
        let into = scratch(&format!("refused-{purpose}"));
        let output = install(
            &artifacts,
            &into,
            &machine(&format!("uname-{purpose}"), "Linux", "x86_64"),
            &[],
        );
        assert!(
            !output.status.success(),
            "{purpose}: an unverified archive was installed"
        );
        assert!(
            stderr(&output).contains(expected),
            "{purpose}: the refusal should say `{expected}`: {}",
            stderr(&output)
        );
        assert!(
            !into.join("agent-compose").exists(),
            "{purpose}: the installer refused and installed anyway"
        );
    }
}

/// The version asked for is the version installed, and an unasked-for choice is
/// refused rather than guessed.
///
/// A directory holding two releases is the case that decides it: with a version
/// named, that version; with none, the installer has two candidates and no way
/// to rank them, and picking either would install something nobody chose.
#[test]
fn the_version_asked_for_is_the_version_installed() {
    let artifacts = release("versions", &[RELEASED, "4.5.6"]);
    let target = "x86_64-unknown-linux-musl";

    for asked in [RELEASED.to_string(), format!("v{RELEASED}")] {
        let into = scratch("asked");
        let output = install(
            &artifacts,
            &into,
            &machine("uname-asked", "Linux", "x86_64"),
            &[&asked],
        );
        assert!(output.status.success(), "{}", stderr(&output));
        assert!(
            stdout(&output).contains(&format!("agent-compose {RELEASED} (fixture, {target})")),
            "`{asked}` should have installed {RELEASED}: {}",
            stdout(&output)
        );
    }

    let into = scratch("ambiguous");
    let output = install(
        &artifacts,
        &into,
        &machine("uname-ambiguous", "Linux", "x86_64"),
        &[],
    );
    assert!(
        !output.status.success(),
        "two releases in one directory is not a choice the installer may make"
    );
    assert!(
        stderr(&output).contains("Name the version to install"),
        "the refusal should say what to do about it: {}",
        stderr(&output)
    );
    assert!(!into.join("agent-compose").exists());
}

/// A version nobody built is refused, rather than half-installed.
#[test]
fn a_version_nobody_built_is_refused() {
    let artifacts = release("absent", &[RELEASED]);
    let into = scratch("absent-into");
    let output = install(
        &artifacts,
        &into,
        &machine("uname-absent", "Linux", "x86_64"),
        &["9.9.9"],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("agent-compose-9.9.9-x86_64-unknown-linux-musl.tar.gz"),
        "the refusal should name what it looked for: {}",
        stderr(&output)
    );
    assert!(!into.join("agent-compose").exists());
}

/// The workflows publish the artifacts the installer asks for.
///
/// The artifact name is this pipeline's one shared convention, and it is
/// written down in four places: the target lists both release workflows pass,
/// the packaging step that composes the file name, the installer that rebuilds
/// that name from `uname`, and the README's table. Nothing checks a workflow
/// until a tag is pushed, so a rename on one side and not the other would be
/// discovered by the first user to run the install line. Here it is a test.
#[test]
fn the_workflows_publish_the_artifacts_the_installer_asks_for() {
    let expected: BTreeSet<&str> = TARGETS.into_iter().collect();
    for workflow in ["release.yml", "release-dry-run.yml"] {
        let text = fs::read_to_string(repo_root().join(".github/workflows").join(workflow))
            .unwrap_or_else(|error| panic!("`{workflow}` is readable: {error}"));
        let named: BTreeSet<&str> = text
            .split("\"triple\": \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        assert_eq!(
            named, expected,
            "`{workflow}` builds a different set of targets than the installer knows about"
        );
    }

    let packaging = fs::read_to_string(repo_root().join(".github/workflows/release-artifacts.yml"))
        .expect("the callable workflow is readable");
    assert!(
        packaging.contains("archive=\"agent-compose-$VERSION-$TRIPLE.tar.gz\""),
        "the packaging step no longer composes the name the installer rebuilds"
    );

    let readme = fs::read_to_string(repo_root().join("README.md")).expect("the README is readable");
    for target in TARGETS {
        assert!(
            readme.contains(&format!("agent-compose-<version>-{target}.tar.gz")),
            "the README's table does not name the `{target}` archive"
        );
    }
}
