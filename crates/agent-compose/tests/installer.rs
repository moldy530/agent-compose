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
///
/// It carries a `sysctl` too. macOS decides "is this process being translated
/// by Rosetta?" with `sysctl.proc_translated`, so a machine that answers `uname`
/// but not that one is only half a machine — and a fixture that let the *host's*
/// `sysctl` answer would behave differently on somebody's Mac than in CI. The
/// answer here is a native process's, `0`; [`rosetta`] and [`intel_mac`]
/// overwrite it with the other two.
fn machine(purpose: &str, system: &str, machine: &str) -> PathBuf {
    let directory = scratch(purpose);
    write_executable(
        &directory.join("uname"),
        &format!(
            "#!/bin/sh\ncase \"$1\" in\n  -s) echo {system} ;;\n  -m) echo {machine} ;;\n  \
             *) echo \"the fixture uname was asked for $1\" >&2; exit 1 ;;\nesac\n"
        ),
    );
    write_executable(
        &directory.join("sysctl"),
        "#!/bin/sh\ncase \"$*\" in\n  *sysctl.proc_translated) echo 0 ;;\n  \
         *) echo \"the fixture sysctl was asked for $*\" >&2; exit 1 ;;\nesac\n",
    );
    directory
}

/// A shell running **under Rosetta**: `uname -m` says `x86_64`, because x86_64
/// is what is being emulated, and `sysctl.proc_translated` says `1`, because
/// the machine underneath it is arm64.
fn rosetta(purpose: &str) -> PathBuf {
    let directory = machine(purpose, "Darwin", "x86_64");
    write_executable(
        &directory.join("sysctl"),
        "#!/bin/sh\ncase \"$*\" in\n  *sysctl.proc_translated) echo 1 ;;\n  \
         *) echo \"the fixture sysctl was asked for $*\" >&2; exit 1 ;;\nesac\n",
    );
    directory
}

/// An Intel Mac: x86_64 all the way down, and **no `proc_translated` oid at
/// all**. That is an error rather than a `0`, and the installer has to read the
/// error as "not translated" rather than as an answer it can compare.
fn intel_mac(purpose: &str) -> PathBuf {
    let directory = machine(purpose, "Darwin", "x86_64");
    write_executable(
        &directory.join("sysctl"),
        "#!/bin/sh\necho \"sysctl: unknown oid '$2'\" >&2\nexit 1\n",
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

/// A GitHub that serves one release, as a `curl` to put on `PATH` ahead of the
/// real one.
///
/// It answers **two exact URLs** and refuses everything else with `curl`'s own
/// "the transfer failed" status — the release's `latest` document, and an asset
/// under `releases/download/<tag>/<name>`. Both shapes are GitHub's, written
/// here rather than read out of the script, so an installer that asked for the
/// wrong URL gets a refusal naming what it asked for.
///
/// The served directory holds `latest.json` and a `v<version>/` of assets.
fn github(purpose: &str) -> PathBuf {
    let served = scratch(purpose);
    let assets = release(&format!("{purpose}-assets"), &[RELEASED]);
    fs::rename(&assets, served.join(format!("v{RELEASED}"))).expect("the assets are movable");

    // The order is the API's own: `tag_name` before `body`. The body carries the
    // same words on purpose — a release note may quote them, and the reader must
    // still take the field.
    fs::write(
        served.join("latest.json"),
        format!(
            "{{\n  \"html_url\": \"https://github.com/moldy530/agent-compose/releases/tag/\
             v{RELEASED}\",\n  \"tag_name\": \"v{RELEASED}\",\n  \"draft\": false,\n  \
             \"body\": \"the installer reads \\\"tag_name\\\": \\\"v9.9.9\\\" out of this \
             document\"\n}}\n"
        ),
    )
    .expect("the release document is writable");

    write_executable(
        &served.join("curl"),
        r#"#!/bin/sh
# A GitHub that serves one release. Reads the argument shapes the installer
# uses: `-fsSL <url>` to standard output, and `-fsSL --retry N -o <path> <url>`.
destination=""
url=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) destination="$2"; shift 2 ;;
    --retry) shift 2 ;;
    -*) shift ;;
    *) url="$1"; shift ;;
  esac
done

serve() {
  if [ -n "$destination" ]; then cp "$1" "$destination"; else cat "$1"; fi
}

case "$url" in
  https://api.github.com/repos/moldy530/agent-compose/releases/latest)
    serve "$FAKE_GITHUB/latest.json"
    ;;
  https://github.com/moldy530/agent-compose/releases/download/*/*)
    name="${url##*/}"
    tag="${url%/*}"
    tag="${tag##*/}"
    if [ ! -f "$FAKE_GITHUB/$tag/$name" ]; then
      echo "no such asset: $url" >&2
      exit 22
    fi
    serve "$FAKE_GITHUB/$tag/$name"
    ;;
  *)
    echo "not a url this release serves: $url" >&2
    exit 22
    ;;
esac
"#,
    );
    served
}

/// Run `install.sh` against the GitHub `serving`, with no artifact directory —
/// the path every user who runs the one-liner takes.
fn install_from_github(
    serving: &Path,
    into: &Path,
    path_first: &Path,
    arguments: &[&str],
) -> Output {
    let inherited = std::env::var("PATH").unwrap_or_default();
    Command::new("sh")
        .arg(repo_root().join("install.sh"))
        .args(arguments)
        .env(
            "PATH",
            format!(
                "{}:{}:{inherited}",
                serving.to_str().expect("a UTF-8 path"),
                path_first.to_str().expect("a UTF-8 path")
            ),
        )
        .env("FAKE_GITHUB", serving)
        .env_remove("AGENT_COMPOSE_ARTIFACT_DIR")
        .env_remove("AGENT_COMPOSE_VERSION")
        .env("AGENT_COMPOSE_INSTALL", into)
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

/// A shell under Rosetta installs the binary for the machine underneath it.
///
/// `uname -m` answers with the architecture the process is *running as*, which
/// under translation is not the architecture of the machine: a `sh` translated
/// on Apple silicon says `x86_64`. Taking that answer installs the Intel build
/// on an arm64 Mac, where it works — slowly, under Rosetta, for as long as
/// nobody looks. The other two Darwin answers are here beside it, because the
/// oid's *absence* on an Intel Mac is a different thing from a `0` and must not
/// be read as a translated process.
#[test]
fn a_translated_shell_installs_the_binary_for_the_machine_underneath() {
    let artifacts = release("rosetta", &[RELEASED]);

    let into = scratch("rosetta-into");
    let output = install(&artifacts, &into, &rosetta("uname-rosetta"), &[]);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stdout(&output).contains(&format!(
            "agent-compose {RELEASED} (fixture, aarch64-apple-darwin)"
        )),
        "a translated shell should be handed the native arm64 build: {}",
        stdout(&output)
    );

    let into = scratch("intel-into");
    let output = install(&artifacts, &into, &intel_mac("uname-intel"), &[]);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(
        stdout(&output).contains(&format!(
            "agent-compose {RELEASED} (fixture, x86_64-apple-darwin)"
        )),
        "a Mac whose kernel has no `proc_translated` at all is not a translated \
         shell, and should be handed the x86_64 build: {}",
        stdout(&output)
    );
}

/// The install directory ends up holding the binary and nothing else.
///
/// The unpacked binary is staged **inside the install directory** and renamed
/// into place, because a temporary directory is routinely on another filesystem
/// — `/tmp` is a tmpfs on most Linux distributions — where `mv` cannot rename
/// and copies over the destination instead: not atomic, and a truncating write
/// onto a file that may be executing. Staging there makes the install directory
/// the place a failed install would leave litter, so that is what is checked:
/// an install, an install over it, and a refusal, and afterwards one file. The
/// refusal is also held to leaving the *previous* install intact — a checksum
/// that does not match is a reason to install nothing, not a reason to take
/// away the compiler somebody already had.
#[test]
fn an_install_leaves_the_directory_holding_the_binary_and_nothing_else() {
    let artifacts = release("staging", &[RELEASED]);
    let into = scratch("staging-into");
    for pass in ["first", "second"] {
        let output = install(
            &artifacts,
            &into,
            &machine(&format!("uname-staging-{pass}"), "Linux", "x86_64"),
            &[],
        );
        assert!(
            output.status.success(),
            "the {pass} install failed: {}{}",
            stdout(&output),
            stderr(&output)
        );
    }

    let corrupt = release("staging-corrupt", &[RELEASED]);
    fs::write(
        corrupt.join("SHA256SUMS"),
        format!(
            "{}  agent-compose-{RELEASED}-x86_64-unknown-linux-musl.tar.gz\n",
            "0".repeat(64)
        ),
    )
    .expect("the checksum file is writable");
    let refused = install(
        &corrupt,
        &into,
        &machine("uname-staging-refused", "Linux", "x86_64"),
        &[],
    );
    assert!(
        !refused.status.success(),
        "an archive the checksum did not vouch for was installed"
    );

    let left: BTreeSet<String> = fs::read_dir(&into)
        .expect("the install directory is readable")
        .map(|entry| {
            entry
                .expect("its entries are readable")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(
        left,
        BTreeSet::from(["agent-compose".to_string()]),
        "the install directory should hold the binary alone"
    );
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

/// The path the one-liner takes: ask GitHub for the latest release, download
/// the archive and the checksums it published, verify, install.
///
/// This is the branch every user runs and the one nothing else here reaches —
/// the dry-run gate installs from a local directory, because on a pull request
/// there is no release to download. What it pins is the two URL shapes, which
/// are GitHub's rather than ours: a wrong one is a `curl` failure at somebody
/// else's terminal, months after the change that caused it.
#[test]
fn the_latest_release_is_resolved_downloaded_and_verified() {
    let serving = github("github");
    let into = scratch("from-github");
    let output = install_from_github(
        &serving,
        &into,
        &machine("uname-github", "Linux", "x86_64"),
        &[],
    );
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    // The tag came out of the release document — including past a body that
    // quotes the same field name — and the asset URL was built from it.
    assert!(
        stdout(&output).contains(&format!(
            "downloading https://github.com/moldy530/agent-compose/releases/download/v{RELEASED}/\
             agent-compose-{RELEASED}-x86_64-unknown-linux-musl.tar.gz"
        )),
        "the installer asked for a different URL: {}",
        stdout(&output)
    );
    assert!(
        stdout(&output).contains(&format!(
            "agent-compose {RELEASED} (fixture, x86_64-unknown-linux-musl)"
        )),
        "the downloaded binary should be the one that ran: {}",
        stdout(&output)
    );
    assert!(into.join("agent-compose").is_file());

    // A release that is not there is a refusal naming the version, not a
    // half-install.
    let missing = scratch("from-github-missing");
    let refused = install_from_github(
        &serving,
        &missing,
        &machine("uname-github-missing", "Linux", "x86_64"),
        &["7.7.7"],
    );
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("v7.7.7"),
        "the refusal should name the release it could not get: {}",
        stderr(&refused)
    );
    assert!(!missing.join("agent-compose").exists());
}

/// A fetch that failed is reported as a fetch that failed.
///
/// Resolving "the latest release" reads a document from the releases API, and
/// every way that read can fail — an unreachable host, a proxy's error page,
/// the `404` GitHub answers for a repository that is private, renamed or gone —
/// has to arrive as *that*. Reading the download inside the same pipeline as
/// the parse loses it: a pipeline answers with its last command's status, so
/// `sed` finding no `tag_name` in an empty body succeeds, and the failure is
/// reported as "the API named no release" — which sends the reader off to check
/// a version number when what broke was the network. Both messages are pinned
/// here, against each other, because each is the other's wrong answer.
#[test]
fn a_release_document_that_could_not_be_fetched_names_the_fetch() {
    // `curl -f` exits 22 on a 404, which is what a private or renamed
    // repository answers.
    let refusing = scratch("curl-404");
    write_executable(
        &refusing.join("curl"),
        "#!/bin/sh\necho \"curl: (22) The requested URL returned error: 404\" >&2\nexit 22\n",
    );
    let into = scratch("unreachable-into");
    let output = install_from_github(
        &refusing,
        &into,
        &machine("uname-unreachable", "Linux", "x86_64"),
        &[],
    );
    assert!(
        !output.status.success(),
        "a release nobody could fetch is not an install"
    );
    assert!(
        stderr(&output).contains("could not reach the GitHub releases API"),
        "the refusal should name the fetch that failed: {}",
        stderr(&output)
    );
    assert!(
        !stderr(&output).contains("named no release"),
        "a failed download is not an empty release list: {}",
        stderr(&output)
    );
    assert!(!into.join("agent-compose").exists());

    // The other half: the API answered, and what it answered carried no
    // release. That is the message the case above must not take.
    let empty = scratch("curl-empty");
    write_executable(
        &empty.join("curl"),
        r#"#!/bin/sh
# A GitHub that answers, with a document naming no release.
destination=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) destination="$2"; shift 2 ;;
    --retry) shift 2 ;;
    *) shift ;;
  esac
done
if [ -n "$destination" ]; then : > "$destination"; else :; fi
"#,
    );
    let into = scratch("no-release-into");
    let output = install_from_github(
        &empty,
        &into,
        &machine("uname-no-release", "Linux", "x86_64"),
        &[],
    );
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("named no release"),
        "a document with no release in it should say so: {}",
        stderr(&output)
    );
    assert!(!into.join("agent-compose").exists());
}

/// Every target triple a workflow's matrix names.
fn triples(text: &str) -> BTreeSet<&str> {
    text.split("\"triple\": \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .collect()
}

/// What the dry run builds, and on which event.
///
/// A pull request builds the two Linux legs; a manual `workflow_dispatch`
/// rehearsal builds all four, as does a tag (PRD q24). The reason is money —
/// this repository is private, where a macOS runner minute bills at ten times a
/// Linux one — and the price of it is that a macOS-only break is caught at the
/// rehearsal or at the tag rather than on the pull request that caused it. Both
/// arms are pinned: one that quietly lost its macOS legs would make the
/// rehearsal pointless, and one that quietly gained them would put the bill
/// back on every push of every pull request.
#[test]
fn a_pull_request_builds_the_linux_legs_and_a_rehearsal_builds_all_four() {
    let text = fs::read_to_string(repo_root().join(".github/workflows/release-dry-run.yml"))
        .expect("the dry-run workflow is readable");
    let expression = text
        .split_once("github.event_name == 'workflow_dispatch'")
        .expect("the dry run picks its matrix by the event that started it")
        .1;
    // `… && '<rehearsal>' || '<pull request>'`: the quoted words of what
    // follows, in order, are the two arms.
    let arms: Vec<&str> = expression.split('\'').collect();
    let rehearsal = arms.get(1).expect("the `workflow_dispatch` arm");
    let pull_request = arms.get(3).expect("the pull-request arm");

    assert_eq!(
        triples(rehearsal),
        TARGETS.into_iter().collect::<BTreeSet<_>>(),
        "a rehearsal should build every leg a release does"
    );
    assert_eq!(
        triples(pull_request),
        BTreeSet::from(["aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl"]),
        "a pull request should build the two Linux legs and neither macOS one"
    );
}

/// The workflows publish the artifacts the installer asks for.
///
/// The artifact name is this pipeline's one shared convention, and it is
/// written down in four places: the target lists both release workflows pass,
/// the packaging step that composes the file name, the installer that rebuilds
/// that name from `uname`, and the README's table. Nothing checks a workflow
/// until a tag is pushed, so a rename on one side and not the other would be
/// discovered by the first user to run the install line. Here it is a test.
///
/// What is held is the *vocabulary*: every triple either workflow names is one
/// the installer asks for, and none is missing. Which of the dry run's two arms
/// builds which of them is a separate question, and
/// [`a_pull_request_builds_the_linux_legs_and_a_rehearsal_builds_all_four`] is
/// where it is answered.
#[test]
fn the_workflows_publish_the_artifacts_the_installer_asks_for() {
    let expected: BTreeSet<&str> = TARGETS.into_iter().collect();
    for workflow in ["release.yml", "release-dry-run.yml"] {
        let text = fs::read_to_string(repo_root().join(".github/workflows").join(workflow))
            .unwrap_or_else(|error| panic!("`{workflow}` is readable: {error}"));
        assert_eq!(
            triples(&text),
            expected,
            "`{workflow}` names a different set of targets than the installer knows about"
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
