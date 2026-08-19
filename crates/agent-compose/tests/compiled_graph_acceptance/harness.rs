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
//! # Two ways to run a compiled graph, and why there are two
//!
//! A criterion about **what a compiled graph does** and a criterion about **what
//! a command does** are different claims, and this harness keeps them apart:
//!
//! * [`invoke`] builds the fixture and runs its graph through **Node**, calling
//!   the emitted project's own `runFlow` — the invocation surface PRD 5.11's
//!   `start` is built on. Everything about node functions, routers, cycles and
//!   policy is decided this way, because that is where those criteria live: they
//!   are properties of the emitted TypeScript, and a CLI that has not been
//!   written yet is not what makes them true or false.
//! * [`run`] shells out to `agent-compose run`, which is its own M1 deliverable
//!   (PRD §7 M1's second bullet). The tests that use it are about **the command**
//!   — that it validates, builds, checks the environment, launches the emitted
//!   project and prints what it produced — which is what keeps "the CLI works" an
//!   honest claim rather than one the graph tests answer on its behalf.
//!
//! [`serve`] is the same shape: the real command, over the real emitted app.
//!
//! Every helper below reaches the **real** compiler and the **real** emitted
//! project — there are no stubs here, deliberately: un-ignoring is the
//! definition of done, and a harness of stubs would let a test pass against a
//! stub.
//!
//! # Interface assumptions
//!
//! The PRD names the commands but not their flags, so this file fixes them, and
//! the codegen PRs either match it or change it here in the same PR:
//!
//! | call | command |
//! |---|---|
//! | [`build`] | `agent-compose build <entrypoint> --target <name> --out <dir>` |
//! | [`invoke`] | `bun <driver> <project> <flow> <inputs.json> <trace.json> <session>`, over the emitted `runFlow` |
//! | [`run`] | `agent-compose run <entrypoint> <flow> --input k=v … [--session <key>] --out <dir>` |
//! | [`serve`] | `agent-compose serve <entrypoint> --port 0 --out <dir>`, announcing its address on stdout |
//!
//! `--out` is the one flag the original table did not name, and both verbs take
//! it for the reason [`scratch_project`] gives: a launch needs the pinned
//! dependency set to resolve and a store's data to be this test's own, and both
//! are properties of *where the project was built*.
//!
//! Both invocation forms print the flow's outputs as one JSON object on stdout
//! (PRD 5.11's `run` is a CLI verb over a flow's declared output schema, so a
//! JSON object is the only shape that survives the schema's own types), and
//! `serve` prints its bound address the way `mock-provider` does, because a test
//! that has to guess a port cannot run in parallel with another one.
//!
//! # The JavaScript toolchain
//!
//! [`invoke`] runs a compiled graph under **Bun**, which PRD §9.18 makes the
//! default runtime of every emitted project: an acceptance suite is the claim
//! that a compiled graph behaves, and it has to make that claim about the runtime
//! a reader is told to use. `compose-core`'s `tests/generated_code_gates.rs` is
//! where the Node fallback is checked — gate 13 runs a golden under it and gate 15
//! answers both shared corpora with it — on goldens rather than here, because the
//! fallback is a property of the emitted modules, not of any one composition.
//!
//! It needs the pinned dependency set installed, and reuses `compose-core`'s
//! committed toolchain fixture — the same `package.json` and `bun.lock` the gates
//! install, which `the_toolchain_fixture_pins_what_the_emitter_pins` holds to the
//! emitter's own pins — building each fixture into a directory beneath it, so
//! `node_modules` resolves by walking up. One install per test binary serves
//! every fixture, because the emitted dependency set is a compiler constant
//! rather than a per-project one. Both the install and the search that finds
//! `bun` are `compose-core`'s `tests/support/toolchain.rs`, included here by path
//! so the two suites cannot end up checking different toolchains.
//!
//! **In CI a missing toolchain fails; on a developer machine it skips**, which is
//! the rule that module states and for the same reason: CI is where "the suite is
//! green" has to mean "the compiled graph ran".

#![allow(
    dead_code,
    unused_imports,
    reason = "each helper is used by the tests of one M1 bullet, \
    and a bullet whose tests are all still `#[ignore]`d leaves its helper unused \
    from the compiler's point of view until that bullet lands — and this module is \
    included by more than one test target (`tests/trace_format_stability.rs` beside \
    the acceptance suite), each of which reaches for a different part of it"
)]

#[path = "../../../compose-core/tests/support/toolchain.rs"]
pub mod toolchain;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// `bun_command` beside `bun`: a test that needs the *runtime* and not the pinned
// install — the CEL conformance driver imports one dependency-free emitted module
// — takes the runtime directly rather than installing a dependency set it never
// resolves, and still gets the skip-locally, fail-in-CI rule `toolchain` states.
pub use toolchain::{bun, bun_command, installed};

use mock_provider::{Client, MockProvider};
use serde_json::Value;

/// Every acceptance fixture project, by directory name.
///
/// These are the inputs the whole suite is written against, and they are checked
/// **today**: `the_acceptance_fixtures_validate_clean` runs the real `validate`
/// over each one, so an ignored test is never waiting on a project that stopped
/// being a valid composition.
pub const FIXTURES: &[&str] = &[
    "activities",
    "agent-anthropic",
    "agent-openai",
    "bounded-cycle",
    "fanout",
    "flow-as-tool",
    "http-trigger",
    "model-failover",
    "provider-kinds",
    "stores",
];

/// The `${MOCK_BASE_URL}` every fixture's providers resolve at process start.
pub const BASE_URL: &str = "MOCK_BASE_URL";
/// The `${MOCK_API_KEY}` they send. Any non-empty value: the harness needs no
/// API keys, but a request still has to carry the header a client sends.
pub const API_KEY: &str = "MOCK_API_KEY";
/// The directory of ordinary commands an `exec:` fixture reaches
/// (`http-trigger`'s `escalate` node, `activities`' `false` and `sleep`),
/// supplied so a run is not stopped by an unrelated presence check.
pub const OPS_BIN: &str = "OPS_BIN";
/// The HTTP server an `http:` fixture node reaches (`activities`' `probe`). The
/// harness points it at the mock provider, whose control plane answers `GET
/// /_mock/state` with JSON — a real round trip over a server the test owns.
pub const OPS_URL: &str = "OPS_URL";

/// The compiler under test.
fn agent_compose() -> Command {
    Command::new(env!("CARGO_BIN_EXE_agent-compose"))
}

/// Where the acceptance fixture projects live.
///
/// Named for what they are — compositions this suite **executes** — rather than
/// for the milestone that introduced them (CLAUDE.md). The `one-*` projects
/// beside this directory are the other half of `tests/projects/`: compositions
/// the compiler is expected to *refuse*.
pub fn projects() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/execution")
}

/// The entrypoint of one acceptance fixture.
pub fn fixture(name: &str) -> PathBuf {
    let path = projects().join(name).join("main.yml");
    assert!(
        path.is_file(),
        "the acceptance fixture `{name}` is missing: {}",
        path.display()
    );
    path
}

/// The environment a run needs to reach the mock instead of a real provider.
///
/// For every **fixture** this is the whole redirection mechanism, and it is a
/// *spec-level* one: each declares `base_url: ${MOCK_BASE_URL}` on its
/// providers, env refs survive unresolved into the IR, and resolution happens at
/// process start (PRD 5.9). Nothing about the composition differs between a
/// scripted run and a real one.
///
/// A composition that does **not** parameterise its `base_url:` — which
/// `examples/review-loop` does not, being written to talk to Anthropic — is
/// redirected in the host preamble instead ([`invoke_hosted`]). That is a
/// harness affordance rather than something the spec offers, which is why the
/// fixtures are written rather than borrowed.
pub fn environment(provider: &MockProvider) -> Vec<(String, String)> {
    vec![
        (BASE_URL.to_string(), provider.base_url()),
        (API_KEY.to_string(), "mock-provider-key".to_string()),
        (OPS_BIN.to_string(), "/bin".to_string()),
        (OPS_URL.to_string(), provider.base_url()),
    ]
}

/// An executable script on `PATH`, for a composition that names a command.
///
/// A `tool.*` with an `exec:` binding names a real program, and a worked example
/// names the programs its author has — `repo-grep`, `run-checks`. A test that
/// wanted those to be absent would be testing the failure path; one that wants
/// the graph to *run* supplies them, prepends this directory to `PATH`, and
/// keeps the composition exactly as the example ships it.
///
/// # Panics
///
/// Panics when the scratch directory is not writable, which is a broken test
/// rather than a finding.
pub fn shim(directory: &Path, name: &str, body: &str) {
    let path = directory.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).expect("the scratch area is writable");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("the shim is made executable");
    }
}

/// A directory that removes itself.
pub struct Scratch(PathBuf);

impl Scratch {
    /// A fresh one, named for what it holds.
    pub fn new(purpose: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "agent-compose-acceptance-{purpose}-{}-{}",
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
    validate_entrypoint(&fixture(name), target)
}

/// The same, for any composition on disk.
pub fn validate_entrypoint(entrypoint: &Path, target: &str) -> Output {
    agent_compose()
        .arg("validate")
        .arg(entrypoint)
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

    /// The whole trace **document** the run wrote (`docs/trace.md`).
    ///
    /// Read from the file the command **names on stderr**, which is the whole
    /// point of it naming one: a trace grows with the run, so the terminal gets
    /// a summary and a reader — a person or this harness — gets the file.
    ///
    /// The file is the versioned envelope rather than a bare array: `entries`
    /// holds what [`Run::trace`] answers, and the keys around it are the version
    /// a reader pins and the run the entries belong to.
    pub fn trace_document(&self) -> Value {
        let path = self.trace_path();
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read the trace at `{path}`: {error}"));
        serde_json::from_str(&text).expect("the trace file holds one JSON document")
    }

    /// Where the run said it wrote its trace.
    pub fn trace_path(&self) -> String {
        let stderr = self.stderr();
        stderr
            .lines()
            .find_map(|line| line.strip_prefix("trace: "))
            .unwrap_or_else(|| panic!("the run names where it wrote its trace\nstderr: {stderr}"))
            .trim()
            .to_string()
    }

    /// Every routing decision the run recorded, in step order (PRD 5.3).
    pub fn trace(&self) -> Vec<Value> {
        let document = self.trace_document();
        document["entries"]
            .as_array()
            .unwrap_or_else(|| panic!("the trace document carries its entries: {document}"))
            .clone()
    }

    /// One node's trace entries, in step order.
    pub fn entries(&self, node: &str) -> Vec<Value> {
        self.trace()
            .into_iter()
            .filter(|entry| entry["node"] == node)
            .collect()
    }
}

/// The session identity every [`invoke`] supplies.
///
/// `runFlow` refuses a run whose flow reaches a `scope: session` store with no
/// session key (grammar 11.3), and one of the worked examples has such a store —
/// so the driver always supplies one. Which one it is says nothing: what a test
/// asserts about a session-scoped store is that a write outlives the execution.
pub const SESSION: &str = "acceptance-session";

/// The driver [`invoke`] runs: it imports the emitted project and calls its own
/// `runFlow`, which is the surface `run` and `serve` are built on.
fn driver() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compiled_graph_acceptance/invoke-flow.mjs")
}

/// The entrypoint of one worked example under `examples/`.
///
/// The examples are not fixtures — they are the documented projects, and
/// `compose-core`'s golden corpus is built from these very files — so running one
/// is the only way an acceptance test speaks about what a reader is shown.
pub fn example(name: &str) -> PathBuf {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .parent()
        .expect("the repository root")
        .join("examples")
        .join(name)
        .join("main.yml");
    assert!(
        path.is_file(),
        "the example `{name}` is missing: {}",
        path.display()
    );
    path
}

/// Build a fixture beneath the installed toolchain, so `node_modules` resolves.
pub fn build_under_toolchain(name: &str, purpose: &str) -> Option<(PathBuf, Output)> {
    build_entrypoint(&fixture(name), purpose)
}

/// A directory beneath the installed toolchain for one built project.
///
/// Everything that **runs** an emitted project builds into one of these, for two
/// reasons that both matter. `node_modules` resolves by walking up, so a project
/// built anywhere else could not import the pinned dependency set — and every
/// call gets its own directory, because cargo runs the tests of one binary on
/// parallel threads and two of them writing one project would race. It is also
/// where a store's data lands (`.agent-compose/`), so a fresh directory is a
/// fresh store: a `scope: global` store that carried a previous test's writes
/// would make an assertion about a `search` depend on test order.
pub fn scratch_project(purpose: &str) -> Option<PathBuf> {
    let root = installed()?;
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let out = root.join("projects").join(format!(
        "acceptance-{purpose}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&out);
    Some(out)
}

/// The same, for any composition on disk — a fixture, or a worked [`example`].
pub fn build_entrypoint(entrypoint: &Path, purpose: &str) -> Option<(PathBuf, Output)> {
    let out = scratch_project(purpose)?;
    let output = agent_compose()
        .arg("build")
        .arg(entrypoint)
        .args(["--target", "local"])
        .arg("--out")
        .arg(&out)
        .env("NO_COLOR", "1")
        .output()
        .expect("the command runs");
    Some((out, output))
}

/// Run one flow of a built fixture, through Node, against the mock provider.
///
/// This is the invocation half of every codegen criterion: the graph is the
/// emitted one, the provider is the scripted one, and what comes back is the
/// flow's own `outputs:` plus the routing trace the run recorded (PRD 5.3).
///
/// Answers `None` when Bun is absent and this is not CI, which is the same skip
/// `tests/generated_code_gates.rs` takes; a test that gets `None` has nothing to
/// assert and returns.
pub fn invoke(
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    provider: &MockProvider,
) -> Option<Invocation> {
    let object: Value = Value::Object(
        inputs
            .iter()
            .map(|(field, value)| ((*field).to_string(), Value::String((*value).to_string())))
            .collect(),
    );
    invoke_with(name, flow, &object, &environment(provider))
}

/// The same, with the inputs as JSON and the environment given explicitly.
pub fn invoke_with(
    name: &str,
    flow: &str,
    inputs: &Value,
    environment: &[(String, String)],
) -> Option<Invocation> {
    invoke_hosted(name, flow, inputs, environment, None)
}

/// The same again, with the **host preamble** — a module the driver imports
/// before the graph.
///
/// It is where a real host does whatever has to happen before the composition
/// loads. Two tests use it, for the two such things this suite has: registering
/// the implementation of a `function:` binding, which is the only way a
/// composition using grammar 6.1's escape hatch runs at all, and redirecting a
/// provider whose `base_url:` the composition does not parameterise. `None` is
/// what a test that wants the unregistered failure passes.
pub fn invoke_hosted(
    name: &str,
    flow: &str,
    inputs: &Value,
    environment: &[(String, String)],
    host: Option<&str>,
) -> Option<Invocation> {
    invoke_entrypoint(&fixture(name), name, flow, inputs, environment, host)
}

/// The same, for any composition on disk — a fixture, or a worked [`example`].
pub fn invoke_entrypoint(
    entrypoint: &Path,
    label: &str,
    flow: &str,
    inputs: &Value,
    environment: &[(String, String)],
    host: Option<&str>,
) -> Option<Invocation> {
    let (project, built) = build_entrypoint(entrypoint, "invoke")?;
    if let Some(source) = host {
        std::fs::write(project.join("host-functions.mjs"), source)
            .expect("the project directory is writable");
    }
    assert!(
        built.status.success(),
        "the composition `{label}` did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let inputs_path = project.join("invoke-inputs.json");
    std::fs::write(
        &inputs_path,
        serde_json::to_string(inputs).expect("the inputs serialize"),
    )
    .expect("the project directory is writable");
    let trace_path = project.join("invoke-trace.json");

    let mut command = bun();
    command
        .arg(driver())
        .arg(&project)
        .arg(flow)
        .arg(&inputs_path)
        .arg(&trace_path)
        .arg(SESSION);
    seal(&mut command, environment);
    let output = command.output().expect("bun runs");
    Some(Invocation {
        run: Run { output },
        trace: trace_path,
        project,
    })
}

/// What one compiled-graph run produced.
pub struct Invocation {
    /// Its stdout, stderr and status, read exactly as a `run`'s are.
    pub run: Run,
    /// Where the driver wrote the routing trace.
    trace: PathBuf,
    /// The built project it ran out of.
    pub project: PathBuf,
}

impl Invocation {
    /// The flow's outputs, as the run printed them.
    pub fn outputs(&self) -> Value {
        self.run.outputs()
    }

    /// Assert the run succeeded.
    pub fn succeeded(&self) -> &Self {
        self.run.succeeded();
        self
    }

    /// Assert the run failed, and answer with what it said.
    pub fn failed(&self) -> String {
        self.run.failed()
    }

    /// Everything the run said on stderr.
    pub fn stderr(&self) -> String {
        self.run.stderr()
    }

    /// Every routing decision the run recorded, in step order (PRD 5.3).
    ///
    /// A **failed** run has one too: `runFlow` raises a `FlowFailure` carrying
    /// the steps that completed plus the entry of the node it stopped at, and
    /// the driver writes that. So a negative assertion about a failing run — "it
    /// did not continue past the node that failed" — is decided by what is in
    /// the trace rather than by its being empty, which would hold whatever the
    /// run had done.
    pub fn trace(&self) -> Vec<Value> {
        let text = std::fs::read_to_string(&self.trace).unwrap_or_else(|error| {
            panic!(
                "the run wrote no trace ({error})\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&self.run.output.stdout),
                self.stderr()
            )
        });
        serde_json::from_str(&text).expect("the trace is a JSON array")
    }

    /// The nodes the run entered, in the order the trace records them.
    pub fn visited(&self) -> Vec<String> {
        self.trace()
            .iter()
            .map(|entry| entry["node"].as_str().expect("a node id").to_string())
            .collect()
    }

    /// One node's trace entries, in step order.
    pub fn entries(&self, node: &str) -> Vec<Value> {
        self.trace()
            .into_iter()
            .filter(|entry| entry["node"] == node)
            .collect()
    }
}

/// `agent-compose run <fixture> <flow> --input k=v …`, pointed at the mock.
///
/// Answers `None` when Bun is absent and this is not CI, the same skip
/// [`invoke`] takes: the command builds a project and then launches it, so it
/// needs the same toolchain a compiled graph does.
pub fn run(
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    provider: &MockProvider,
) -> Option<Run> {
    run_with(name, flow, inputs, &environment(provider))
}

/// The same, with the environment given explicitly — which is how the env-ref
/// presence check is tested: by leaving one out.
pub fn run_with(
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    environment: &[(String, String)],
) -> Option<Run> {
    let out = scratch_project("run")?;
    Some(run_into(&out, name, flow, inputs, None, environment))
}

/// The same again, into a directory the **caller** owns.
///
/// Which is what a session-scoped store needs from this harness: a store's data
/// lives under the built project (`.agent-compose/`), so two runs that are meant
/// to see each other's writes have to be two runs of one directory. Everything
/// else takes a fresh one.
pub fn run_into(
    out: &Path,
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    session: Option<&str>,
    environment: &[(String, String)],
) -> Run {
    run_formatted(out, name, flow, inputs, session, None, environment)
}

/// The same, choosing the report format — the one flag that changes what a run
/// writes on which stream.
pub fn run_formatted(
    out: &Path,
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    session: Option<&str>,
    format: Option<&str>,
    environment: &[(String, String)],
) -> Run {
    let mut command = run_command(out, name, flow, inputs, session, format);
    seal(&mut command, environment);
    let output = command.output().expect("the command runs");
    Run { output }
}

/// `agent-compose run <fixture> <flow> …`, before its environment is sealed.
fn run_command(
    out: &Path,
    name: &str,
    flow: &str,
    inputs: &[(&str, &str)],
    session: Option<&str>,
    format: Option<&str>,
) -> Command {
    let mut command = agent_compose();
    command.arg("run").arg(fixture(name)).arg(flow);
    for (field, value) in inputs {
        command.arg("--input").arg(format!("{field}={value}"));
    }
    if let Some(session) = session {
        command.arg("--session").arg(session);
    }
    if let Some(format) = format {
        command.arg("--format").arg(format);
    }
    command.arg("--out").arg(out);
    command
}

/// The variable that tells an emitted `run` to answer `human` pauses at standard
/// input, whatever that input is (grammar 8.7).
///
/// A terminal is what decides it for a person, and a test has none: the child is
/// a pipe, so `isTTY` is false and a run that waited on one would be a suite
/// that hangs. The variable is not a test seam invented for that — it is the
/// documented way a *script* answers a pause, which is what a test is — and it
/// is why the acceptance suite can drive the surface at all.
pub const INTERACTIVE: &str = "AGENT_COMPOSE_INTERACTIVE";

/// What standard input does once a terminal-answered run has been given its
/// answers.
pub enum Answers {
    /// Closed, which is a script that has said everything it has to say.
    ///
    /// The run then has no answer surface left, so a pause it has not been given
    /// an answer for ends it exactly as a non-interactive run's does.
    Closed,
    /// Held open until the command exits — a terminal nobody is typing at.
    ///
    /// What a wait has to **expire** against: a closed input would end the run
    /// before the budget could run out, and the two ways a prompt ends without
    /// an answer would be one.
    Held,
}

/// One terminal-answered `run`, as [`run_answering`] takes it.
///
/// A struct rather than a parameter list because a `run` this suite *answers*
/// carries everything an ordinary one does plus the script for the questions,
/// and eight positional arguments at a call site say nothing about which is
/// which.
pub struct Answering<'a> {
    /// Where the project is built, so a store's data and the pinned install are
    /// this call's (see [`scratch_project`]).
    pub out: &'a Path,
    /// The acceptance fixture to run, by directory name.
    pub fixture: &'a str,
    /// Its flow address.
    pub flow: &'a str,
    /// The `--input k=v` arguments.
    pub inputs: &'a [(&'a str, &'a str)],
    /// `--format`, where the test is about which stream carries what.
    pub format: Option<&'a str>,
    /// The environment the run resolves its `${ENV}` references from.
    pub environment: &'a [(String, String)],
    /// One line per pause the run is expected to ask, in the order it asks.
    pub answers: &'a [&'a str],
    /// What standard input does once those are written.
    pub afterwards: Answers,
}

/// `agent-compose run …` with its `human` pauses answered at standard input.
///
/// One line per answer, which is the framing the prompt asks for. Both output
/// streams are drained on threads of their own while the answers are written,
/// because a run that pauses writes its prompts to stderr *before* it reads: a
/// caller that wrote first and read afterwards would deadlock the moment a
/// composition's prompts filled the pipe.
pub fn run_answering(asked: Answering<'_>) -> Run {
    let Answering {
        out,
        fixture: name,
        flow,
        inputs,
        format,
        environment,
        answers,
        afterwards,
    } = asked;
    let mut command = run_command(out, name, flow, inputs, None, format);
    let mut sealed: Vec<(String, String)> = environment.to_vec();
    // Not forced over a caller that named it: one test's whole subject is what
    // `AGENT_COMPOSE_INTERACTIVE=0` does to a run whose input is otherwise a
    // perfectly good script.
    if !sealed.iter().any(|(name, _)| name == INTERACTIVE) {
        sealed.push((INTERACTIVE.to_string(), "1".to_string()));
    }
    seal(&mut command, &sealed);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("the command runs");
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let mut stdout = child.stdout.take().expect("stdout is piped");
    let mut stderr = child.stderr.take().expect("stderr is piped");
    let reading_out = std::thread::spawn(move || {
        let mut held = Vec::new();
        let _ = stdout.read_to_end(&mut held);
        held
    });
    let reading_err = std::thread::spawn(move || {
        let mut held = Vec::new();
        let _ = stderr.read_to_end(&mut held);
        held
    });
    for answer in answers {
        let _ = writeln!(stdin, "{answer}");
        let _ = stdin.flush();
    }
    // `Child::wait` closes the handle it holds, and this one is no longer the
    // child's — it was taken above — so keeping it here is what keeps standard
    // input open for a run whose wait has to expire against it.
    let open = match afterwards {
        Answers::Closed => {
            drop(stdin);
            None
        }
        Answers::Held => Some(stdin),
    };
    let status = child.wait().expect("the command is waited on");
    drop(open);
    Run {
        output: Output {
            status,
            stdout: reading_out.join().expect("stdout is read"),
            stderr: reading_err.join().expect("stderr is read"),
        },
    }
}

/// The variables that survive [`seal`], because they are the machine and not
/// the composition: a generated project runs under `bun` and an `exec` node
/// runs a command, so both need `PATH`, and `bun` reads `HOME` for its install
/// cache.
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
/// the JavaScript toolchain would buy nothing for it.
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
///
/// **The whole tree, not the command.** `agent-compose serve` is one process and
/// the app is another — the emitted project, launched by it — so a harness that
/// killed the command alone would leave the app listening, holding its port and
/// its project directory, reparented to init and never told to stop. That is not
/// a slow cleanup; it is a leak per served test, on every `cargo test`, and it
/// makes the suite non-hermetic on a runner that is reused. So [`serve`] puts
/// the command in a **process group of its own** and this kills the group.
///
/// `SIGKILL` rather than a graceful `SIGTERM`, because what is being asserted
/// about a served app is asserted before this runs: a graceful stop would be a
/// second thing to wait for and would make the end of every serve test a race
/// with the app's own shutdown. The command forwards `SIGTERM` on its own — the
/// test that pins it is
/// `stopping_serve_stops_the_app_it_started`, which is where that behaviour is
/// decided rather than here.
pub struct Served {
    child: Child,
    /// Whether the command has already been waited on, and its pid with it.
    ///
    /// A reaped pid is not this process's any more — the number is the system's
    /// to hand out again — so nothing is signalled once this is set. A test that
    /// stops the command itself is the only thing that sets it.
    reaped: bool,
    /// Where the generated app is listening.
    pub base_url: String,
}

impl Served {
    /// Send one signal to the command **alone**, leaving the group to the app.
    ///
    /// Which is the whole point where it is used: a signal delivered to the
    /// command and not to the app is how "the command hands its signals on" is
    /// asked as a question rather than assumed.
    #[cfg(unix)]
    pub fn signal(&self, signal: libc::c_int) {
        assert!(!self.reaped, "the command has already been waited on");
        // SAFETY: a pid this process spawned and has not reaped.
        unsafe {
            libc::kill(
                libc::pid_t::try_from(self.child.id()).expect("a pid"),
                signal,
            );
        }
    }

    /// Wait for the command to exit and answer with its status.
    pub fn wait(&mut self) -> std::process::ExitStatus {
        let status = self.child.wait().expect("the command is waited on");
        self.reaped = true;
        status
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        #[cfg(unix)]
        {
            // The negated pid is the group [`serve`] spawned the command into,
            // so this reaches the emitted app as well as the command that
            // launched it. `ESRCH` — a group whose members are already gone — is
            // an expected answer and is ignored like every other.
            if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
                // SAFETY: `pid` is this process's own child, unreaped until the
                // `wait` below, so its group is still its own.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
        }
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
///
/// The command is spawned into a **process group of its own**, which is what
/// lets [`Served::drop`] end the app the command launched rather than only the
/// command — see there.
pub fn serve(name: &str, provider: &MockProvider) -> Option<Served> {
    let out = scratch_project("serve")?;
    let mut command = agent_compose();
    command
        .arg("serve")
        .arg(fixture(name))
        .args(["--port", "0"])
        .arg("--out")
        .arg(&out)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
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
    Some(Served {
        child,
        reaped: false,
        base_url,
    })
}

/// `agent-compose serve <fixture> --port <port>`, **waited on** rather than
/// listened to.
///
/// [`serve`] reads a readiness line and hands back a live app, which is what a
/// test of the served surface wants and exactly what a test of a *failed start*
/// cannot use: there is no readiness line, and a harness waiting for one would
/// hang instead of failing. So this one runs the command to completion and hands
/// back what it wrote and how it exited.
pub fn serve_refused(name: &str, provider: &MockProvider, port: u16) -> Option<Output> {
    serve_refused_entrypoint(&fixture(name), provider, port)
}

/// The same, for any composition on disk.
///
/// Which is what a *route* collision needs and a fixture cannot give: the
/// composition it takes is one the app refuses to mount, so a fixture of it
/// would be a project the rest of the suite builds, type-checks and can never
/// serve. The compositions written for this are written where they are read.
pub fn serve_refused_entrypoint(
    entrypoint: &Path,
    provider: &MockProvider,
    port: u16,
) -> Option<Output> {
    let out = scratch_project("serve-refused")?;
    let mut command = agent_compose();
    command
        .arg("serve")
        .arg(entrypoint)
        .args(["--port", &port.to_string()])
        .arg("--out")
        .arg(&out);
    seal(&mut command, &environment(provider));
    Some(command.output().expect("the command runs"))
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

/// Where a `callback:` webhook is delivered: a socket that answers `200` and
/// keeps what it was posted.
///
/// The generated app POSTs its completion report to whatever URL the trigger's
/// `callback:` CEL produced (grammar 13.3, PRD 5.11), and nothing else in this
/// harness can receive one — [`MockProvider`] is a *provider* surface, and the
/// served app is the thing under test. So this is the other end of the webhook:
/// a listener a test points a `callback_url` at, which records every delivery in
/// arrival order.
///
/// It answers `200` to anything and reads no route, because what is under test
/// is which requests the app makes rather than what a receiver does with them.
pub struct Receiver {
    /// The base URL to build a `callback_url` from.
    pub base_url: String,
    delivered: Arc<Mutex<Vec<Value>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Receiver {
    /// Bind a receiver on loopback.
    pub fn start() -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let base_url = format!("http://{}", listener.local_addr()?);
        listener.set_nonblocking(true)?;
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let delivered = Arc::clone(&delivered);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if let Some(body) = deliver(&mut stream) {
                                delivered.lock().expect("the deliveries").push(body);
                            }
                        }
                        // Nothing has connected yet; the stop flag is read
                        // between polls, which is how the thread ends.
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            })
        };
        Ok(Self {
            base_url,
            delivered,
            stop,
            thread: Some(thread),
        })
    }

    /// Every delivery so far, in arrival order.
    pub fn delivered(&self) -> Vec<Value> {
        self.delivered.lock().expect("the deliveries").clone()
    }

    /// Wait for `count` deliveries, or say what arrived instead.
    ///
    /// A webhook is fired *after* the run settles, so a test that read the list
    /// straight after a `202` would be asserting about scheduling luck.
    pub fn wait_for(&self, count: usize, budget: Duration) -> Vec<Value> {
        let deadline = Instant::now() + budget;
        loop {
            let held = self.delivered();
            if held.len() >= count {
                return held;
            }
            assert!(
                Instant::now() < deadline,
                "only {} of {count} webhook deliveries arrived: {held:?}",
                held.len()
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Receiver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read one HTTP request off `stream`, answer it `200`, and hand back its body.
fn deliver(stream: &mut TcpStream) -> Option<Value> {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("a read budget");
    let mut buffer: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 1024];
    let mut head: Option<usize> = None;
    let mut length = 0usize;
    loop {
        if head.is_none()
            && let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
        {
            head = Some(at + 4);
            let text = String::from_utf8_lossy(&buffer[..at]).to_lowercase();
            length = text
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .and_then(|value| value.trim().parse().ok())
                .unwrap_or(0);
        }
        if let Some(start) = head
            && buffer.len() >= start + length
        {
            break;
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
        }
    }
    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    let start = head?;
    serde_json::from_slice(&buffer[start..]).ok()
}
