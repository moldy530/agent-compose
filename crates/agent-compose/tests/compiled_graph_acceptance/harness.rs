//! What an M1 acceptance test does: build a fixture project, run it against the
//! mock provider, and read the transcript back.
//!
//! # The shape of an acceptance test
//!
//! ```text
//! let provider = MockProvider::start();          // no API keys, no network
//! provider.enqueue(Script::new(model, Outcome::structured(json!({ … }))));
//! let run = harness::run("agent-anthropic", "flow.review", &[("goal", "…")], &provider);
//! assert_eq!(run.outputs()["verdict"], "approve");   // what the graph produced
//! assert_eq!(provider.requests()[0].tools, ["…"]);   // what the graph sent
//! ```
//!
//! Both halves matter. The output says the graph *ran*; the transcript says it
//! ran **correctly** — the request a compiled graph sends is the thing codegen
//! can get wrong in ways an output assertion cannot see, and the mock refuses
//! anything a real provider would refuse (see `crates/mock-provider`).
//!
//! # These commands do not exist yet
//!
//! `build`, `run`, and `serve` are M1's own deliverables (PRD §7 M1). Every
//! helper below shells out to the **real** CLI, so today they fail with clap's
//! "unrecognized subcommand" and the tests that call them are `#[ignore]`d with
//! the reason naming what must land first. That is deliberate: un-ignoring is
//! the definition of done, and a harness of stubs would let a test pass against
//! a stub.
//!
//! # Interface assumptions
//!
//! The PRD names the commands but not their flags, so this file fixes them, and
//! the codegen PRs either match it or change it here in the same PR:
//!
//! | call | command |
//! |---|---|
//! | [`build`] | `agent-compose build <entrypoint> --target <name> --out <dir>` |
//! | [`run`] | `agent-compose run <entrypoint> <flow> --input k=v … [--session <key>]` |
//! | [`serve`] | `agent-compose serve <entrypoint> --port 0`, announcing its address on stdout |
//!
//! [`run`] is assumed to print the flow's outputs as one JSON object on stdout
//! (PRD 5.11's `run` is a CLI verb over a flow's declared output schema, so a
//! JSON object is the only shape that survives the schema's own types), and
//! `serve` to print its bound address the way `mock-provider` does, because a
//! test that has to guess a port cannot run in parallel with another one.

#![allow(
    dead_code,
    reason = "each helper is used by the tests of one M1 bullet, \
    and a bullet whose tests are all still `#[ignore]`d leaves its helper unused \
    from the compiler's point of view until that bullet lands"
)]

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use mock_provider::{Client, MockProvider};
use serde_json::Value;

/// Every acceptance fixture project, by directory name.
///
/// These are the inputs the whole suite is written against, and they are checked
/// **today**: `the_acceptance_fixtures_validate_clean` runs the real `validate`
/// over each one, so an ignored test is never waiting on a project that stopped
/// being a valid composition.
pub const FIXTURES: &[&str] = &[
    "agent-anthropic",
    "agent-openai",
    "bounded-cycle",
    "fanout",
    "http-trigger",
    "model-failover",
    "stores",
];

/// The `${MOCK_BASE_URL}` every fixture's providers resolve at process start.
pub const BASE_URL: &str = "MOCK_BASE_URL";
/// The `${MOCK_API_KEY}` they send. Any non-empty value: the harness needs no
/// API keys, but a request still has to carry the header a client sends.
pub const API_KEY: &str = "MOCK_API_KEY";
/// The one non-provider env ref a fixture declares (`http-trigger`'s `escalate`
/// node), supplied so a run is not stopped by an unrelated presence check.
pub const OPS_BIN: &str = "OPS_BIN";

/// The compiler under test.
fn agent_compose() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agent-compose"))
}

/// The entrypoint of one acceptance fixture.
pub fn fixture(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/projects/m1")
        .join(name)
        .join("main.yml");
    assert!(
        path.is_file(),
        "the acceptance fixture `{name}` is missing: {}",
        path.display()
    );
    path
}

/// The environment a run needs to reach the mock instead of a real provider.
///
/// This is the whole redirection mechanism, and it is a *spec-level* one: every
/// fixture declares `base_url: ${MOCK_BASE_URL}` on its providers, env refs
/// survive unresolved into the IR, and resolution happens at process start
/// (PRD 5.9). Nothing about the composition differs between a scripted run and a
/// real one.
pub fn environment(provider: &MockProvider) -> Vec<(String, String)> {
    vec![
        (BASE_URL.to_string(), provider.base_url()),
        (API_KEY.to_string(), "mock-provider-key".to_string()),
        (OPS_BIN.to_string(), "/bin".to_string()),
    ]
}

/// A directory that removes itself.
pub struct Scratch(PathBuf);

impl Scratch {
    fn new(purpose: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "agent-compose-m1-{purpose}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        Self(path)
    }

    /// Where it is.
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// What `validate` said about a project.
pub fn validate(name: &str, target: &str) -> Output {
    agent_compose()
        .arg("validate")
        .arg(fixture(name))
        .args(["--target", target])
        .env("NO_COLOR", "1")
        .output()
        .expect("the command runs")
}

/// A built project: the generated TypeScript, and what the command said.
pub struct Built {
    pub scratch: Scratch,
    pub output: Output,
}

impl Built {
    /// The generated project's root.
    pub fn root(&self) -> &Path {
        self.scratch.path()
    }

    /// Every generated file, as repository-relative paths under the output
    /// directory, sorted — the shape a golden-file comparison reads.
    pub fn files(&self) -> Vec<String> {
        let mut found = Vec::new();
        let mut queue = vec![self.root().to_path_buf()];
        while let Some(directory) = queue.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    queue.push(path);
                    continue;
                }
                if let Ok(relative) = path.strip_prefix(self.root()) {
                    found.push(relative.display().to_string());
                }
            }
        }
        found.sort();
        found
    }

    /// The bytes of one generated file.
    pub fn read(&self, relative: &str) -> String {
        std::fs::read_to_string(self.root().join(relative)).unwrap_or_else(|error| {
            panic!(
                "the build did not write `{relative}` ({error}); it wrote {:?}",
                self.files()
            )
        })
    }

    /// Assert the build succeeded, with its own diagnostics if it did not.
    pub fn succeeded(&self) -> &Self {
        assert!(
            self.output.status.success(),
            "build failed ({:?})\nstdout: {}\nstderr: {}",
            self.output.status.code(),
            String::from_utf8_lossy(&self.output.stdout),
            String::from_utf8_lossy(&self.output.stderr),
        );
        self
    }
}

/// `agent-compose build <fixture> --target <target> --out <scratch>`.
pub fn build(name: &str, target: &str) -> Built {
    let scratch = Scratch::new(name);
    let output = agent_compose()
        .arg("build")
        .arg(fixture(name))
        .args(["--target", target])
        .arg("--out")
        .arg(scratch.path())
        .env("NO_COLOR", "1")
        .output()
        .expect("the command runs");
    Built { scratch, output }
}

/// What a run produced.
pub struct Run {
    pub output: Output,
}

impl Run {
    /// The flow's outputs, as the run printed them.
    pub fn outputs(&self) -> Value {
        let stdout = String::from_utf8_lossy(&self.output.stdout);
        serde_json::from_str(&stdout).unwrap_or_else(|error| {
            panic!(
                "a run prints the flow's outputs as one JSON object ({error})\nstdout: {stdout}\nstderr: {}",
                String::from_utf8_lossy(&self.output.stderr)
            )
        })
    }

    /// Everything the run said on stderr — where a failure explains itself.
    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    /// Assert the run succeeded.
    pub fn succeeded(&self) -> &Self {
        assert!(
            self.output.status.success(),
            "run failed ({:?})\nstdout: {}\nstderr: {}",
            self.output.status.code(),
            String::from_utf8_lossy(&self.output.stdout),
            self.stderr(),
        );
        self
    }

    /// Assert the run failed, and answer with what it said.
    pub fn failed(&self) -> String {
        assert!(
            !self.output.status.success(),
            "the run was expected to fail and did not\nstdout: {}",
            String::from_utf8_lossy(&self.output.stdout)
        );
        self.stderr()
    }
}

/// `agent-compose run <fixture> <flow> --input k=v …`, pointed at the mock.
pub fn run(name: &str, flow: &str, inputs: &[(&str, &str)], provider: &MockProvider) -> Run {
    run_with(name, flow, inputs, &environment(provider))
}

/// The same, with the environment given explicitly — which is how the env-ref
/// presence check is tested: by leaving one out.
pub fn run_with(
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    environment: &[(String, String)],
) -> Run {
    let mut command = agent_compose();
    command.arg("run").arg(fixture(name)).arg(flow);
    for (field, value) in inputs {
        command.arg("--input").arg(format!("{field}={value}"));
    }
    seal(&mut command, environment);
    let output = command.output().expect("the command runs");
    Run { output }
}

/// The variables that survive [`seal`], because they are the machine and not
/// the composition: a generated project runs under `node` and an `exec` node
/// runs a command, so both need `PATH`, and `node`/`npm` read `HOME` for their
/// caches.
pub const MACHINE: &[&str] = &["PATH", "HOME"];

/// Give `command` the environment this call names, and nothing else.
///
/// Every helper that starts a process which **resolves env refs** goes through
/// here — `run_with` and [`serve`] — because both of the reasons are about that
/// resolution rather than about which command does it:
///
/// * the presence check of PRD 5.9 is only testable if a variable left out is
///   really absent;
/// * a developer machine that happens to export `ANTHROPIC_API_KEY`, or a
///   fixture that grows a ref this harness does not supply, must not make a run
///   behave one way locally and another in CI.
///
/// `validate` and `build` are left alone: env refs survive *unresolved* into the
/// IR and into generated code (PRD 5.9), so a compile-time command reads none of
/// them, and clearing the environment around a build that may yet shell out to
/// the Node toolchain would buy nothing for it.
pub fn seal(command: &mut Command, environment: &[(String, String)]) {
    command.env_clear();
    for passed_through in MACHINE {
        if let Ok(value) = std::env::var(passed_through) {
            command.env(passed_through, value);
        }
    }
    command.env("NO_COLOR", "1");
    for (name, value) in environment {
        command.env(name, value);
    }
}

/// A served project, killed when the test ends.
pub struct Served {
    child: Child,
    /// Where the generated app is listening.
    pub base_url: String,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `agent-compose serve <fixture> --port 0`, pointed at the mock.
///
/// Waits for the app to announce its address on stdout, the way
/// `mock-provider` does: a harness that slept instead would be a flaky test.
///
/// The environment is [`seal`]ed exactly as a `run`'s is: a served app resolves
/// the same env refs at process start, so the two halves of the harness have to
/// agree on what a fixture's refs may resolve from.
pub fn serve(name: &str, provider: &MockProvider) -> Served {
    let mut command = agent_compose();
    command
        .arg("serve")
        .arg(fixture(name))
        .args(["--port", "0"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    seal(&mut command, &environment(provider));
    let mut child = command.spawn().expect("the command runs");
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut line = String::new();
    BufReader::new(stdout)
        .read_line(&mut line)
        .expect("the generated app announces its address before serving");
    let announced: Value = serde_json::from_str(&line)
        .unwrap_or_else(|error| panic!("the readiness line is not JSON ({error}): {line:?}"));
    let base_url = announced["base_url"]
        .as_str()
        .unwrap_or_else(|| panic!("the readiness line names no base url: {announced}"))
        .to_string();
    Served { child, base_url }
}

/// The states a status report can be asserted about: an execution has either
/// finished, or stopped at an interrupt waiting for a resume.
const SETTLED: &[&str] = &["completed", "interrupted", "failed"];

/// The status route's report for `execution`, once it has stopped moving.
///
/// An `async` trigger answers with an execution id *before* the flow has run
/// (grammar 13.3), so the state a status assertion is about arrives some time
/// after the start route did. Polling is what makes such an assertion about the
/// graph rather than about scheduling luck — and a report that stays unsettled
/// is answered with the whole last body, so a run that ends somewhere the test
/// did not expect says where instead of timing out silently.
pub fn settled(app: &Client, execution: &str) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let last: Value = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        if last["status"]
            .as_str()
            .is_some_and(|status| SETTLED.contains(&status))
        {
            return last;
        }
        assert!(
            Instant::now() < deadline,
            "execution `{execution}` never reached one of {SETTLED:?}: {last}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}
