//! `agent-compose store`, end to end through the real binary, on every backend
//! a `kv` store can be bound to (PRD resolved q72, Decision D155).
//!
//! The command reads and writes one `kv` store with no flow written: it builds
//! the project as `run` does and launches the emitted command line's `store`
//! verb, which opens the store through the very binding a flow of that build
//! opens it through. So what is under test is the whole chain a reader types —
//! clap, the build, the launch, the emitted verb, the arm the target bound, and
//! the exit code passed back — and it is driven three times:
//!
//!  * **SQLite**, through `--target local`, which substitutes local storage for
//!    every store and keeps a store's rows in a file under the build directory
//!    (Decision D87) — always, since it needs nothing but the pinned driver;
//!  * **Postgres** and **MySQL**, through a named target whose deploy file binds
//!    the store's alias to the server [`POSTGRES_URL`] or [`MYSQL_URL`] names —
//!    the two variables the store and journal conformance suites read, so one
//!    pair of CI service containers serves all three. Absent, the arm is skipped
//!    **loudly** off CI and is a failure in it, the rule `store_backend_conformance`
//!    states.
//!
//! Every case keys on store names minted per run, so two runs against one server
//! never read each other's rows, and a dialled arm deletes every row its stores
//! wrote when it ends — passing or not ([`Project`]'s `Drop`).
//!
//! What the ruling asks to be verified, and where:
//!
//!  * a `set` → `get` round trip with its version, a conditional `set` declining
//!    with exit `1` and naming the version the key holds, a `delete` under
//!    `--if-version`, `list` paging with `--prefix`/`--limit`/`--after`, a
//!    `count` that agrees, a `--ttl` that reads absent past its expiry — to a
//!    `get`, a `count`, a `list` and an `export` each, while the key is still
//!    in storage — an `export` → edit → `import` round trip at fresh versions
//!    with a remaining `expires_at` honoured and an expired line skipped, an
//!    `export` of more keys than a page holds writing each once, in byte
//!    order, with a summary that counts them all, an import reading a line's
//!    `expires_at` against the store's own clock rather than this machine's, a
//!    schema-refused import line named by number and a schema-refused `set`
//!    value — each exiting `2`, a command line to fix rather than an answer of
//!    no — an import the backend stops part-way naming the line it stopped at
//!    and how many it wrote before it, exit `1`, `--if-absent` seeding without
//!    overwriting, the wrong scope flags and an unknown address exiting `2`,
//!    and the `--format json` shapes — [`drive`], once per arm;
//!  * a store whose rows live only in each execution's process — bound to
//!    `provider: memory` by a named target — refused by every sub-verb with
//!    exit `2` rather than answered out of an empty database —
//!    [`a_store_a_target_binds_to_memory_is_refused_rather_than_read_from_an_empty_one`],
//!    beside [`drive`]'s `scope: execution` under `--target local`;
//!  * a store's `default_ttl:` taken by a `set` with no `--ttl` and not by an
//!    import line with no `expires_at` — [`drive`] too;
//!  * a command whose standard output will not take its answer — a pipe whose
//!    reader has gone, as `| head -1` leaves it, or a full disk — ending all
//!    the same rather than waiting for ever, with **the op's own exit code**:
//!    `0` for a `get` that found its key or a `set` that applied, an export
//!    stopped by a departed reader included, `1` only where the op's answer
//!    is no or an export's lines did not land, and `2` for a value the schema
//!    refused — [`undelivered`], once per arm — and a server that is not
//!    there exiting `1` from every sub-verb, with `{ error }` under
//!    `--format json` — [`no_server`], once per dialled arm;
//!  * **no effect**: no journal is opened by any of it, no ledger row is left,
//!    and a flow run afterwards reads what the operator wrote — the end of
//!    [`drive`];
//!  * **the runtime's partition derivation**, for both scopes a command
//!    addresses: a flow's session partition holds what the command wrote under
//!    `--session`, and on a dialled arm the command, handed a parked run's
//!    execution id, reads what that execution wrote — the very end of
//!    [`drive`];
//!  * a composition the compiler refuses, reported on stderr for an `export`
//!    and never among its lines —
//!    [`a_refused_composition_is_reported_beside_an_exports_summary_not_among_its_lines`];
//!  * clap's own surface for the verb and each sub-verb, pinned byte for byte —
//!    [`the_store_help_pages_are_pinned`] — and what clap refuses, in its own
//!    words and before anything is built —
//!    [`the_store_command_line_refuses_what_it_cannot_parse`].

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use harness::toolchain::required;
use harness::{Scratch, bun, journal_rows, scratch_project, seal, sqlite_rows, sqlite_sql};

/// Where a reachable Postgres is named — `store_backend_conformance`'s variable.
const POSTGRES_URL: &str = "AGENT_COMPOSE_TEST_POSTGRES_URL";

/// …and a MySQL.
const MYSQL_URL: &str = "AGENT_COMPOSE_TEST_MYSQL_URL";

/// The variable the composition's deploy file names its store server in.
///
/// A name of the suite's own rather than the ambient one, for the conformance
/// suite's reason: the emitted project carries the *name* the deploy file
/// wrote, so the command's environment has to hold that name.
const STORE_URL: &str = "AGENT_COMPOSE_CLI_STORE_URL";

/// The variable that moves every store's expiry clock (PRD resolved q69).
const CLOCK_OFFSET: &str = "AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS";

/// The session a case writes and a flow then reads — with a space in it, so
/// the partition's percent-encoding is part of what has to agree.
const SESSION: &str = "user 1";

/// How long a command whose standard output's reader has gone is given to stop:
/// a build and one refused write, with room for a loaded runner — and a bound,
/// because the failure this guards against is a command that never ends.
const CLOSED_PIPE_PATIENCE: Duration = Duration::from_secs(180);

/// One backend the command is driven against.
struct Arm {
    /// The `provider:` keyword, as grammar §14.3 spells it.
    provider: &'static str,
    /// The variable holding a reachable server, or `None` for a backend the
    /// store's own process opens.
    variable: Option<&'static str>,
}

const SQLITE: Arm = Arm {
    provider: "sqlite",
    variable: None,
};
const POSTGRES: Arm = Arm {
    provider: "postgres",
    variable: Some(POSTGRES_URL),
};
const MYSQL: Arm = Arm {
    provider: "mysql",
    variable: Some(MYSQL_URL),
};
/// Not a backend the command reaches but one it refuses: a named target binding
/// the stores' alias to `provider: memory`, whose rows live in each execution's
/// own process
/// ([`a_store_a_target_binds_to_memory_is_refused_rather_than_read_from_an_empty_one`]).
const MEMORY: Arm = Arm {
    provider: "memory",
    variable: None,
};

/// The composition every arm is driven against, with its stores named `suffix`.
///
/// One store per scope a `kv` store may declare, one more declaring a
/// `default_ttl:`, two keeping history — one for good, one for a
/// `history_retention:` — a `blob` store the command must refuse, one flow that reads what an operator left — in the global
/// store and in one session's — which is how "a flow sees the operator's values"
/// is asked of the runtime rather than of this command, and one that writes its
/// own execution's partition and parks there, which is how "the command reads
/// the partition a flow's execution writes" is asked of it.
fn composition(suffix: &str) -> String {
    format!(
        r#"version: "0.1"

state:
  found: {{ type: boolean, default: false }}
  recalled:
    type: object
    properties:
      text: {{ type: string }}
    default: {{ text: nothing }}
  visited: {{ type: boolean, default: false }}

store.notes_{suffix}:
  kind: kv
  scope: global
  description: What an operator seeds and a flow reads.
  backend: cli_db
  value_schema:
    text: {{ type: string, min_length: 1 }}

store.leases_{suffix}:
  kind: kv
  scope: global
  description: Keys that expire an hour after their write unless told otherwise.
  backend: cli_db
  default_ttl: 1h
  value_schema:
    text: {{ type: string }}

store.visits_{suffix}:
  kind: kv
  scope: session
  description: One session's visits.
  backend: cli_db
  value_schema:
    text: {{ type: string }}

store.scratch_{suffix}:
  kind: kv
  scope: execution
  description: One execution's notes.
  backend: cli_db
  value_schema:
    text: {{ type: string }}

store.files_{suffix}:
  kind: blob
  scope: global
  description: Not a kv store, so not one the store command reaches.

store.audits_{suffix}:
  kind: kv
  scope: global
  history: true
  description: Every write to it is kept, with who made it and when.
  backend: cli_db
  value_schema:
    text: {{ type: string }}

store.trail_{suffix}:
  kind: kv
  scope: global
  history: true
  history_retention: 1h
  description: Every write to it is kept for an hour, and no longer.
  backend: cli_db
  value_schema:
    text: {{ type: string }}

flow.recall:
  description: Read what an operator left, globally and in this session.
  inputs:
    key: {{ type: string }}
  outputs:
    found: {{ type: boolean }}
    recalled:
      type: object
      properties:
        text: {{ type: string }}
    visited: {{ type: boolean }}
  nodes:
    look:
      store: store.notes_{suffix}
      op: get
      key: "input.key"
      writes:
        found: found
        value: recalled
    visit:
      store: store.visits_{suffix}
      op: get
      key: "input.key"
      writes:
        found: visited
  edges:
    - {{ from: start, to: look }}
    - {{ from: look, to: visit }}
    - {{ from: visit, to: end }}

flow.park:
  description: Leave a note in this execution's own partition, then wait for a person.
  inputs:
    key: {{ type: string }}
  outputs: {{}}
  nodes:
    note:
      store: store.scratch_{suffix}
      op: set
      key: "input.key"
      value: {{ text: "'left by the run'" }}
    wait:
      human:
        input:
          note: {{ type: string }}
        output:
          ok: {{ type: boolean }}
      input:
        note: "'nobody is here to answer'"
  edges:
    - {{ from: start, to: note }}
    - {{ from: note, to: wait }}
    - {{ from: wait, to: end }}
"#
    )
}

/// A suffix no other run of this suite, on this machine or another, has used.
fn run_unique() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the clock is past the epoch")
        .subsec_nanos();
    format!(
        "{}_{}_{nanos}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// One composition on disk, the build directory it is run from, and the
/// environment its arm needs.
struct Project {
    /// The composition's own directory, removed when the case ends.
    source: Scratch,
    entrypoint: PathBuf,
    /// Where `--out` builds it, beneath the installed toolchain.
    out: PathBuf,
    target: &'static str,
    provider: &'static str,
    /// Whether the stores are rows on a server, which outlive the case unless
    /// it deletes them.
    dialled: bool,
    environment: Vec<(String, String)>,
    /// The run-unique names of the six stores, without `store.`.
    notes: String,
    leases: String,
    visits: String,
    scratch: String,
    files: String,
    /// …the one that keeps history for good (PRD resolved q74).
    audits: String,
    /// …and the one that keeps it for its `history_retention:` and no longer.
    trail: String,
}

impl Project {
    /// The project for `arm`, or `None` where it cannot run here — no Bun off
    /// CI, or a dialled arm whose server is not named — said loudly.
    fn new(arm: &Arm) -> Option<Self> {
        let address = match arm.variable {
            None => None,
            Some(variable) => match std::env::var(variable) {
                Ok(value) if !value.is_empty() => Some(value),
                _ => {
                    assert!(
                        !required(),
                        "`{variable}` is required in CI: it names the `{}` server the store \
                         command is driven against (PRD resolved q72)",
                        arm.provider
                    );
                    notice(&format!(
                        "warning: skipping the store command's `{}` arm — `{variable}` names no \
                         server. Point it at one to run it.",
                        arm.provider
                    ));
                    return None;
                }
            },
        };
        let out = scratch_project(&format!("store-cli-{}", arm.provider))?;
        let suffix = run_unique();
        let source = Scratch::new(&format!("store-cli-{}", arm.provider));
        let entrypoint = source.path().join("main.yml");
        std::fs::write(&entrypoint, composition(&suffix)).expect("the scratch area is writable");
        let (target, environment) = match address {
            None if arm.provider == "memory" => {
                std::fs::create_dir_all(source.path().join("deploy"))
                    .expect("the scratch area is writable");
                std::fs::write(
                    source.path().join("deploy/memory.yml"),
                    "version: \"0.1\"\nstorage_backends:\n  aliases:\n    cli_db: { provider: memory }\n",
                )
                .expect("the scratch area is writable");
                ("memory", Vec::new())
            }
            None => ("local", Vec::new()),
            Some(address) => {
                std::fs::create_dir_all(source.path().join("deploy"))
                    .expect("the scratch area is writable");
                std::fs::write(
                    source.path().join("deploy/remote.yml"),
                    format!(
                        "version: \"0.1\"\nstorage_backends:\n  aliases:\n    cli_db: {{ provider: {}, url: \"${{{STORE_URL}}}\" }}\n",
                        arm.provider
                    ),
                )
                .expect("the scratch area is writable");
                ("remote", vec![(STORE_URL.to_string(), address)])
            }
        };
        Some(Self {
            source,
            entrypoint,
            out,
            target,
            provider: arm.provider,
            dialled: arm.variable.is_some(),
            environment,
            notes: format!("notes_{suffix}"),
            leases: format!("leases_{suffix}"),
            visits: format!("visits_{suffix}"),
            scratch: format!("scratch_{suffix}"),
            files: format!("files_{suffix}"),
            audits: format!("audits_{suffix}"),
            trail: format!("trail_{suffix}"),
        })
    }

    /// `agent-compose store <sub-verb> <path> <arguments>…` under this arm's
    /// target and build directory.
    fn store(&self, sub_verb: &str, arguments: &[&str]) -> Answer {
        self.store_with(sub_verb, arguments, Given::default())
    }

    /// The command [`Project::store_with`] starts, not yet started.
    fn command(&self, sub_verb: &str, arguments: &[&str], given: &Given<'_>) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .args(["store", sub_verb, "--target", self.target])
            .arg("--out")
            .arg(&self.out)
            .arg(&self.entrypoint)
            .args(arguments);
        let mut environment = self.environment.clone();
        environment.extend(
            given
                .environment
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string())),
        );
        seal(&mut command, &environment);
        if let Some(directory) = given.directory {
            command.current_dir(directory);
        }
        command
    }

    fn store_with(&self, sub_verb: &str, arguments: &[&str], given: Given<'_>) -> Answer {
        let mut command = self.command(sub_verb, arguments, &given);
        command
            .stdin(if given.input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().expect("the command starts");
        if let Some(input) = given.input {
            child
                .stdin
                .take()
                .expect("stdin is piped")
                .write_all(input.as_bytes())
                .expect("the command reads its input");
        }
        let output = child.wait_with_output().expect("the command runs");
        Answer {
            what: format!("store {sub_verb} {}", arguments.join(" ")),
            output,
        }
    }

    /// `agent-compose store <sub-verb> …` with standard output a pipe whose
    /// reader has already gone — `| head -1` once `head` has its line, or a
    /// `jq` that failed — waited on for [`CLOSED_PIPE_PATIENCE`] and no longer.
    ///
    /// The reader is gone before the command writes anything, so its first
    /// write is the one refused, on every run. The command is in a process
    /// group of its own, so a regression that leaves it waiting on the pipe is
    /// a killed tree and a named failure, rather than an emitted project left
    /// spinning behind the suite.
    fn store_into_closed_pipe(&self, sub_verb: &str, arguments: &[&str]) -> Answer {
        let what = format!(
            "store {sub_verb} {} | (a reader that has gone)",
            arguments.join(" ")
        );
        let mut command = self.command(sub_verb, arguments, &Given::default());
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        let mut child = command.spawn().expect("the command starts");
        drop(child.stdout.take());
        let mut stderr = child.stderr.take().expect("stderr is piped");
        let said = std::thread::spawn(move || {
            let mut held = Vec::new();
            let _ = stderr.read_to_end(&mut held);
            held
        });
        let deadline = Instant::now() + CLOSED_PIPE_PATIENCE;
        let status = loop {
            if let Some(status) = child.try_wait().expect("the command is waited on") {
                break Some(status);
            }
            if Instant::now() >= deadline {
                #[cfg(unix)]
                if let Ok(group) = libc::pid_t::try_from(child.id()) {
                    // SAFETY: this process's own child, unreaped until the
                    // `wait` below, so its group is still its own.
                    unsafe { libc::kill(-group, libc::SIGKILL) };
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let stderr = said.join().unwrap_or_default();
        let Some(status) = status else {
            panic!(
                "`agent-compose {what}` was still running after {CLOSED_PIPE_PATIENCE:?} and was \
                 killed: a command whose reader has gone has to stop, not wait for a pipe nobody \
                 will drain.\nstderr: {}",
                String::from_utf8_lossy(&stderr)
            );
        };
        Answer {
            what,
            output: Output {
                status,
                stdout: Vec::new(),
                stderr,
            },
        }
    }

    /// `agent-compose store <sub-verb> …` with standard output `/dev/full`, a
    /// stream that refuses every write with `ENOSPC` while nobody has left —
    /// or `None`, said loudly, where this machine has no such device.
    fn store_into_full_disk(&self, sub_verb: &str, arguments: &[&str]) -> Option<Answer> {
        let Ok(full) = std::fs::OpenOptions::new().write(true).open("/dev/full") else {
            notice(
                "warning: skipping the full-disk half of the store command's undelivered-answer \
                 case — this machine has no `/dev/full`",
            );
            return None;
        };
        let mut command = self.command(sub_verb, arguments, &Given::default());
        let output = command
            .stdin(Stdio::null())
            .stdout(full)
            .stderr(Stdio::piped())
            .output()
            .expect("the command runs");
        Some(Answer {
            what: format!("store {sub_verb} {} > /dev/full", arguments.join(" ")),
            output,
        })
    }

    /// `agent-compose run main.yml flow.park --input key=<key>`, which writes
    /// its own `scope: execution` partition and then stops at a `human` pause
    /// with nobody to ask.
    fn park(&self, key: &str) -> Answer {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .args(["run"])
            .arg(&self.entrypoint)
            .args(["flow.park", "--input"])
            .arg(format!("key={key}"))
            .args(["--target", self.target])
            .arg("--out")
            .arg(&self.out);
        seal(&mut command, &self.environment);
        let output = command
            .stdin(Stdio::null())
            .output()
            .expect("the command runs");
        Answer {
            what: format!("run flow.park --input key={key}"),
            output,
        }
    }

    /// `agent-compose run main.yml flow.recall --input key=<key> --session …`.
    fn recall(&self, key: &str) -> Answer {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .args(["run"])
            .arg(&self.entrypoint)
            .args(["flow.recall", "--input"])
            .arg(format!("key={key}"))
            .args(["--session", SESSION, "--target", self.target])
            .arg("--out")
            .arg(&self.out);
        seal(&mut command, &self.environment);
        let output = command
            .stdin(Stdio::null())
            .output()
            .expect("the command runs");
        Answer {
            what: format!("run flow.recall --input key={key}"),
            output,
        }
    }

    /// Rows a statement answers on this arm's server — `None` on SQLite.
    fn server(&self, sql: &str, parameters: &Value) -> Option<Value> {
        if !self.dialled {
            return None;
        }
        let script = self.script();
        let mut command = bun();
        command
            .arg(&script)
            .args([self.provider, STORE_URL, sql])
            .arg(parameters.to_string());
        seal(&mut command, &self.environment);
        let output = command.output().expect("bun runs");
        assert!(
            output.status.success(),
            "`{sql}` failed on the {} server:\n{}",
            self.provider,
            String::from_utf8_lossy(&output.stderr)
        );
        Some(serde_json::from_slice(&output.stdout).expect("the script prints JSON rows"))
    }

    /// `tests/store_cli/server-sql.mjs`, copied into the build directory so the
    /// pinned drivers resolve from it as the emitted project's do.
    fn script(&self) -> PathBuf {
        let script = self.out.join("server-sql.mjs");
        let _ = std::fs::create_dir_all(&self.out);
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/store_cli/server-sql.mjs"),
            &script,
        )
        .expect("the script is copied into the project");
        script
    }

    /// The idempotency ledger rows this project's stores hold, on any arm.
    fn ledger_rows(&self) -> u64 {
        if let Some(rows) = self.server(
            "SELECT COUNT(*) AS n FROM store_applied WHERE store IN (?, ?, ?, ?)",
            &json!([self.notes, self.leases, self.visits, self.scratch]),
        ) {
            return count(&rows);
        }
        let mut held = 0;
        for store in [&self.notes, &self.leases, &self.visits] {
            let file = self
                .out
                .join(".agent-compose/stores")
                .join(format!("{store}.sqlite"));
            if file.is_file() {
                held += count(&sqlite_rows(
                    &self.out,
                    &file,
                    "SELECT COUNT(*) AS n FROM applied",
                ));
            }
        }
        held
    }

    /// The history rows one of this project's stores holds **in storage** —
    /// a row past its retention that no sweep has removed yet among them —
    /// on any arm: `store_history` on a server, the store file's `history`
    /// table under `--target local`.
    fn history_rows(&self, store: &str) -> u64 {
        if let Some(rows) = self.server(
            "SELECT COUNT(*) AS n FROM store_history WHERE store = ?",
            &json!([store]),
        ) {
            return count(&rows);
        }
        let file = self
            .out
            .join(".agent-compose/stores")
            .join(format!("{store}.sqlite"));
        assert!(
            file.is_file(),
            "a written store has its file: {}",
            file.display()
        );
        count(&sqlite_rows(
            &self.out,
            &file,
            "SELECT COUNT(*) AS n FROM history",
        ))
    }
}

/// Every row this project's stores wrote is gone when the case ends: the build
/// directory with the SQLite files in it, and the rows on a dialled server.
impl Drop for Project {
    fn drop(&mut self) {
        if self.dialled {
            let script = self.script();
            for table in [
                "store_entries",
                "store_applied",
                "store_history",
                "store_sequences",
            ] {
                let sql = format!("DELETE FROM {table} WHERE store IN (?, ?, ?, ?, ?, ?)");
                let mut command = bun();
                command
                    .arg(&script)
                    .args([self.provider, STORE_URL, &sql])
                    .arg(
                        json!([
                            self.notes,
                            self.leases,
                            self.visits,
                            self.scratch,
                            self.audits,
                            self.trail
                        ])
                        .to_string(),
                    );
                seal(&mut command, &self.environment);
                let _ = command.output();
            }
        }
        let _ = std::fs::remove_dir_all(&self.out);
    }
}

/// What a command is handed beside its arguments.
#[derive(Default)]
struct Given<'a> {
    /// Standard input, or nothing (a closed one).
    input: Option<&'a str>,
    /// Variables beside the arm's.
    environment: &'a [(&'a str, &'a str)],
    /// The directory it is started in.
    directory: Option<&'a Path>,
}

/// What one command answered.
struct Answer {
    what: String,
    output: Output,
}

impl Answer {
    fn code(&self) -> i32 {
        self.output
            .status
            .code()
            .expect("the command was not signalled")
    }

    fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.output.stdout).into_owned()
    }

    fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.output.stderr).into_owned()
    }

    /// Assert the exit code, naming the command and what it wrote.
    #[track_caller]
    fn exits(&self, code: i32) -> &Self {
        assert_eq!(
            self.code(),
            code,
            "`agent-compose {}` exits {code}\nstdout: {}\nstderr: {}",
            self.what,
            self.stdout(),
            self.stderr()
        );
        self
    }

    /// stdout as the one JSON document `--format json` answers with.
    #[track_caller]
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout()).unwrap_or_else(|error| {
            panic!(
                "`agent-compose {}` answers one JSON document ({error})\nstdout: {}\nstderr: {}",
                self.what,
                self.stdout(),
                self.stderr()
            )
        })
    }

    /// stdout as JSON Lines, one entry per line.
    #[track_caller]
    fn lines(&self) -> Vec<Value> {
        self.stdout()
            .lines()
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|error| {
                    panic!(
                        "`agent-compose {}` writes JSON Lines ({error}): {line}",
                        self.what
                    )
                })
            })
            .collect()
    }
}

/// A `COUNT(*)` answered as a number by one driver and a string by another.
fn count(rows: &Value) -> u64 {
    let held = &rows[0]["n"];
    held.as_u64()
        .or_else(|| held.as_str().and_then(|text| text.parse().ok()))
        .unwrap_or_else(|| panic!("a count answers a number: {rows}"))
}

/// Say something a passing test's captured output would otherwise swallow.
fn notice(message: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
    let _ = stderr.flush();
}

/// An hour, in milliseconds: the `default_ttl:` the composition's `leases`
/// store declares.
const HOUR: i64 = 3_600_000;

/// How far the store's clock may sit from this process's when an expiry is
/// read against the instants a write was made between: a dialled server keeps
/// its own clock (PRD resolved q69), on this machine or a service container
/// beside it.
const CLOCK_SLACK: i64 = 2_000;

/// How many entries one page of an `export` reads — the emitted `cli.ts`'s
/// `STORE_EXPORT_PAGE`.
const EXPORT_PAGE: usize = 1000;

/// How many keys the multi-page export is driven over: two full pages and a
/// part of a third, so the walk crosses two cursors and ends on a short page.
const BULK_KEYS: usize = 2 * EXPORT_PAGE + 345;

/// Now, in epoch milliseconds.
fn now_millis() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("the clock is past the epoch")
            .as_millis(),
    )
    .expect("an instant in range")
}

/// Every key one export answered, by key, with what it carried.
fn by_key(lines: &[Value]) -> std::collections::BTreeMap<String, Value> {
    lines
        .iter()
        .map(|line| {
            (
                line["key"].as_str().expect("a line has a key").to_string(),
                line.clone(),
            )
        })
        .collect()
}

/// An instant an export wrote, in epoch milliseconds.
fn instant(line: &Value) -> i64 {
    let written = line["expires_at"]
        .as_str()
        .unwrap_or_else(|| panic!("the line carries an `expires_at`: {line}"));
    // `YYYY-MM-DDTHH:MM:SS.mmmZ`, which is what `Date.toISOString` writes.
    let (date, time) = written
        .strip_suffix('Z')
        .and_then(|held| held.split_once('T'))
        .unwrap_or_else(|| panic!("`{written}` is an ISO 8601 UTC instant"));
    let fields: Vec<i64> = date
        .split('-')
        .chain(time.split([':', '.']))
        .map(|field| field.parse().expect("a numeric field"))
        .collect();
    let [year, month, day, hour, minute, second, millis] = fields[..] else {
        panic!("`{written}` has seven fields");
    };
    // Days from the civil date, Howard Hinnant's algorithm.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    ((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis
}

/// An instant in epoch milliseconds as the ISO 8601 UTC instant an export
/// writes — [`instant`] read backwards.
fn iso(millis: i64) -> String {
    let days = millis.div_euclid(86_400_000);
    let of_day = millis.rem_euclid(86_400_000);
    // The civil date from days, Howard Hinnant's algorithm.
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let of_era = shifted - era * 146_097;
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let from_march = (5 * of_year + 2) / 153;
    let day = of_year - (153 * from_march + 2) / 5 + 1;
    let month = if from_march < 10 {
        from_march + 3
    } else {
        from_march - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        of_day / 3_600_000,
        of_day / 60_000 % 60,
        of_day / 1000 % 60,
        of_day % 1000
    )
}

/// Every claim of the ruling, on one arm.
#[allow(
    clippy::too_many_lines,
    reason = "one ordered scenario per arm: each step reads the state the step before it left, \
              which is what makes it the operator's session rather than a set of unit cases"
)]
fn drive(arm: &Arm) {
    let Some(project) = Project::new(arm) else {
        return;
    };
    let notes = format!("store.{}", project.notes);
    let visits = format!("store.{}", project.visits);
    let scratch = format!("store.{}", project.scratch);
    let files = format!("store.{}", project.files);

    // A set, then a get of what it wrote, with the version it took.
    let set = project.store("set", &[&notes, "greet", r#"{"text":"hello"}"#]);
    set.exits(0);
    assert!(
        set.stdout().contains("greet") && set.stdout().contains("true"),
        "a set reports what it applied as a table:\n{}",
        set.stdout()
    );
    let got = project.store("get", &[&notes, "greet", "--format", "json"]);
    assert_eq!(
        got.exits(0).json(),
        json!({ "value": { "text": "hello" }, "found": true, "version": 1 }),
        "a get answers the op's own row"
    );
    let shown = project.store("get", &[&notes, "greet"]);
    assert!(
        shown.exits(0).stdout().contains(r#"{"text":"hello"}"#),
        "the human table shows the value:\n{}",
        shown.stdout()
    );

    // A conditional set that does not hold: exit 1, nothing written, and the
    // version the key holds — on both formats.
    let declined = project.store(
        "set",
        &[&notes, "greet", r#"{"text":"again"}"#, "--if", "absent"],
    );
    declined.exits(1);
    assert!(
        declined.stderr().contains("holds version 1"),
        "a declined write names the version the key holds:\n{}",
        declined.stderr()
    );
    let declined = project.store(
        "set",
        &[
            &notes,
            "greet",
            r#"{"text":"again"}"#,
            "--if",
            "absent",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        declined.exits(1).json(),
        json!({ "key": "greet", "applied": false, "version": 1, "deduped": false })
    );
    project
        .store(
            "set",
            &[&notes, "never", r#"{"text":"x"}"#, "--if", "present"],
        )
        .exits(1);
    let swapped = project.store(
        "set",
        &[
            &notes,
            "greet",
            r#"{"text":"again"}"#,
            "--if-version",
            "1",
            "--format",
            "json",
        ],
    );
    assert_eq!(
        swapped.exits(0).json(),
        json!({ "key": "greet", "applied": true, "version": 2, "deduped": false }),
        "an operator's write carries no idempotency key, so nothing dedupes it"
    );

    // A delete under `--if-version`: the wrong version declines, the right one
    // applies, and the key is gone.
    let stale = project.store("delete", &[&notes, "greet", "--if-version", "1"]);
    assert!(
        stale.exits(1).stderr().contains("holds version 2"),
        "{}",
        stale.stderr()
    );
    let deleted = project.store(
        "delete",
        &[&notes, "greet", "--if-version", "2", "--format", "json"],
    );
    assert_eq!(
        deleted.exits(0).json(),
        json!({ "deleted": true, "applied": true, "deduped": false })
    );
    let gone = project.store("get", &[&notes, "greet"]);
    assert!(
        gone.exits(1).stderr().contains("holds nothing"),
        "{}",
        gone.stderr()
    );
    assert_eq!(
        project
            .store("get", &[&notes, "greet", "--format", "json"])
            .exits(1)
            .json(),
        json!({ "found": false })
    );

    // A value from standard input.
    project
        .store_with(
            "set",
            &[&notes, "other/x", "-"],
            Given {
                input: Some(r#"{"text":"piped"}"#),
                ..Given::default()
            },
        )
        .exits(0);

    // `list` pages under a prefix, with a cursor, and `count` agrees.
    for key in ["page/a", "page/b", "page/c"] {
        project
            .store("set", &[&notes, key, &format!(r#"{{"text":"{key}"}}"#)])
            .exits(0);
    }
    let first = project.store(
        "list",
        &[
            &notes, "--prefix", "page/", "--limit", "2", "--format", "json",
        ],
    );
    assert_eq!(
        first.exits(0).json(),
        json!({ "keys": ["page/a", "page/b"], "next": "page/b" })
    );
    let second = project.store(
        "list",
        &[
            &notes, "--prefix", "page/", "--limit", "2", "--after", "page/b", "--format", "json",
        ],
    );
    assert_eq!(second.exits(0).json(), json!({ "keys": ["page/c"] }));
    let valued = project.store(
        "list",
        &[&notes, "--prefix", "page/", "--values", "--format", "json"],
    );
    let entries = valued.exits(0).json()["entries"].clone();
    assert_eq!(
        entries
            .as_array()
            .expect("`--values` answers entries")
            .iter()
            .map(|entry| entry["value"]["text"].clone())
            .collect::<Vec<_>>(),
        vec![json!("page/a"), json!("page/b"), json!("page/c")]
    );
    let human = project.store("list", &[&notes, "--prefix", "page/", "--limit", "2"]);
    assert!(
        human.exits(0).stdout().contains("next: page/b"),
        "a human page names the cursor of the next:\n{}",
        human.stdout()
    );
    assert_eq!(
        project
            .store("count", &[&notes, "--prefix", "page/", "--format", "json"])
            .exits(0)
            .json(),
        json!({ "count": 3 })
    );
    assert_eq!(
        project
            .store("count", &[&notes, "--format", "json"])
            .exits(0)
            .json(),
        json!({ "count": 4 })
    );

    // A `--ttl`, and the key absent to every read once the clock is past it
    // (PRD resolved q69, q72).
    //
    // A command's first op on the store starts the process-start sweep at that
    // op's clock — **behind** the op — so the op itself reads a store that
    // still holds the expired key, and only the read's own liveness filter
    // keeps the key out of its answer. The sweep removes it afterwards, so a
    // read in a later command would prove the sweep and not the filter: each
    // read below is handed an expired key of its own, written just before it,
    // and is the first op of a command whose clock is past that key's expiry.
    let past = || Given {
        environment: &[(CLOCK_OFFSET, "7200000")],
        ..Given::default()
    };
    let brief = || {
        project
            .store(
                "set",
                &[&notes, "brief", r#"{"text":"held"}"#, "--ttl", "1h"],
            )
            .exits(0);
    };
    brief();
    project.store("get", &[&notes, "brief"]).exits(0);
    project
        .store_with("get", &[&notes, "brief"], past())
        .exits(1);
    brief();
    assert_eq!(
        project
            .store_with("count", &[&notes, "--format", "json"], past())
            .exits(0)
            .json(),
        json!({ "count": 4 }),
        "an expired key is not counted"
    );
    brief();
    assert!(
        !project
            .store_with("list", &[&notes, "--format", "json"], past())
            .exits(0)
            .json()["keys"]
            .as_array()
            .expect("a page has keys")
            .contains(&json!("brief")),
        "an expired key is not listed"
    );
    brief();
    let unexported = project.store_with("export", &[&notes, "--format", "json"], past());
    assert_eq!(
        unexported
            .exits(0)
            .lines()
            .iter()
            .map(|line| line["key"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("other/x"),
            json!("page/a"),
            json!("page/b"),
            json!("page/c")
        ],
        "…nor exported, though the store still held it when the export read its page"
    );
    assert_eq!(
        serde_json::from_str::<Value>(unexported.stderr().trim()).expect("the summary is JSON"),
        json!({ "written": 4 }),
        "…nor counted in the export's summary"
    );

    // Export, edit, import: fresh versions, the remaining `expires_at` kept,
    // an expired line skipped.
    project
        .store(
            "set",
            &[&notes, "lease", r#"{"text":"held"}"#, "--ttl", "1h"],
        )
        .exits(0);
    let exported = project.store("export", &[&notes, "--format", "json"]);
    let lines = exported.exits(0).lines();
    assert_eq!(
        serde_json::from_str::<Value>(exported.stderr().trim()).expect("the summary is JSON"),
        json!({ "written": 5 }),
        "export's summary is on stderr, its lines are stdout"
    );
    assert_eq!(
        lines
            .iter()
            .map(|line| line["key"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!("lease"),
            json!("other/x"),
            json!("page/a"),
            json!("page/b"),
            json!("page/c")
        ],
        "every live key, in byte order"
    );
    let before = by_key(&lines);
    assert!(
        before["lease"]["expires_at"].is_string() && before["page/a"].get("expires_at").is_none(),
        "`expires_at` is written exactly for a key that expires: {lines:?}"
    );
    let highest = lines
        .iter()
        .map(|line| line["version"].as_u64().expect("a version"))
        .max()
        .expect("lines were exported");
    for key in before.keys() {
        project.store("delete", &[&notes, key]).exits(0);
    }
    let mut edited: Vec<String> = lines
        .iter()
        .map(|line| {
            let mut line = line.clone();
            if line["key"] == "page/a" {
                line["value"] = json!({ "text": "edited" });
            }
            line.to_string()
        })
        .collect();
    edited.push(
        json!({ "key": "stale", "value": { "text": "gone" }, "version": 3, "expires_at": "2001-01-01T00:00:00.000Z" })
            .to_string(),
    );
    let imported = project.store_with(
        "import",
        &[&notes, "--format", "json"],
        Given {
            input: Some(&format!("{}\n", edited.join("\n"))),
            ..Given::default()
        },
    );
    assert_eq!(
        imported.exits(0).json(),
        json!({ "read": 6, "written": 5, "skipped": 1, "expired": 1, "present": 0 })
    );
    project.store("get", &[&notes, "stale"]).exits(1);
    let after = by_key(&project.store("export", &[&notes]).exits(0).lines());
    assert_eq!(
        after.keys().collect::<Vec<_>>(),
        before.keys().collect::<Vec<_>>(),
        "the round trip leaves the keys it exported"
    );
    assert_eq!(after["page/a"]["value"], json!({ "text": "edited" }));
    assert_eq!(after["other/x"]["value"], json!({ "text": "piped" }));
    for (key, line) in &after {
        assert!(
            line["version"].as_u64().expect("a version") > highest,
            "`{key}` was imported at a fresh version, past every one exported ({highest}): {line}"
        );
    }
    let drift = instant(&after["lease"]) - instant(&before["lease"]);
    assert!(
        (-1_000..5_000).contains(&drift),
        "the remaining `expires_at` became the `ttl` that keeps it, to within the write \
         ({} then {})",
        before["lease"]["expires_at"],
        after["lease"]["expires_at"]
    );
    assert!(
        after["page/a"].get("expires_at").is_none(),
        "a line with no `expires_at` imports a key that does not expire"
    );

    // What remains of a line's `expires_at` is measured against **the store's
    // own clock** — the one every read and every `--ttl` is dated by, which on
    // a dialled arm is the server's — never this machine's (PRD resolved q69,
    // D155). Here that clock runs two hours ahead: a line due an hour from now
    // is already past to it, so it is skipped and counted as expired, and one
    // due three hours from now has an hour left, which is the `ttl` its write
    // takes — so the key the store holds expires at the very instant the line
    // named. Measured against this machine's clock instead, the first would be
    // written as live and the second would keep three hours, from a clock two
    // hours ahead.
    let now = now_millis();
    let (lapsed, kept) = (iso(now + HOUR), iso(now + 3 * HOUR));
    assert_eq!(
        instant(&json!({ "expires_at": &kept })),
        now + 3 * HOUR,
        "the instants this case writes read back as the instants it meant"
    );
    let clocked = format!(
        "{}\n{}\n",
        json!({ "key": "clocked/lapsed", "value": { "text": "lapsed" }, "expires_at": &lapsed }),
        json!({ "key": "clocked/kept", "value": { "text": "kept" }, "expires_at": &kept })
    );
    let imported = project.store_with(
        "import",
        &[&notes, "--format", "json"],
        Given {
            input: Some(&clocked),
            ..past()
        },
    );
    assert_eq!(
        imported.exits(0).json(),
        json!({ "read": 2, "written": 1, "skipped": 1, "expired": 1, "present": 0 }),
        "a line due an hour from now is past to a store whose clock is two hours ahead"
    );
    project.store("get", &[&notes, "clocked/lapsed"]).exits(1);
    let held = by_key(
        &project
            .store_with("export", &[&notes, "--prefix", "clocked/"], past())
            .exits(0)
            .lines(),
    );
    assert_eq!(
        held.keys().collect::<Vec<_>>(),
        ["clocked/kept"],
        "only the line with time left on the store's clock was written: {held:?}"
    );
    let drift = instant(&held["clocked/kept"]) - (now + 3 * HOUR);
    assert!(
        (-1_000..5_000).contains(&drift),
        "the line's remaining hour by the store's clock became its `ttl`, so it expires when \
         the line said ({kept}), not two hours later: {}",
        held["clocked/kept"]
    );
    project
        .store_with(
            "get",
            &[&notes, "clocked/kept"],
            Given {
                environment: &[(CLOCK_OFFSET, "14400000")],
                ..Given::default()
            },
        )
        .exits(1);
    project.store("delete", &[&notes, "clocked/kept"]).exits(0);

    // An export longer than one page: it follows each page's cursor to the
    // last, so every key is written exactly once, in byte order, and the
    // summary counts them all. The keys are numbered without padding, so their
    // byte order is not the order they were written in, and a page boundary
    // falls between keys that order differently as numbers.
    let bulk: Vec<String> = (0..BULK_KEYS).map(|n| format!("bulk/{n}")).collect();
    std::fs::write(
        project.source.path().join("bulk.jsonl"),
        bulk.iter()
            .map(|key| format!("{}\n", json!({ "key": key, "value": { "text": key } })))
            .collect::<String>(),
    )
    .expect("the scratch area is writable");
    assert_eq!(
        project
            .store_with(
                "import",
                &[&notes, "--file", "bulk.jsonl", "--format", "json"],
                Given {
                    directory: Some(project.source.path()),
                    ..Given::default()
                },
            )
            .exits(0)
            .json(),
        json!({ "read": BULK_KEYS, "written": BULK_KEYS, "skipped": 0, "expired": 0, "present": 0 })
    );
    let paged = project.store("export", &[&notes, "--prefix", "bulk/", "--format", "json"]);
    let paged_lines = paged.exits(0).lines();
    let mut in_byte_order = bulk.clone();
    in_byte_order.sort_unstable();
    assert_ne!(
        in_byte_order, bulk,
        "the keys' byte order is not their numbering"
    );
    let exported_keys: Vec<&str> = paged_lines
        .iter()
        .map(|line| line["key"].as_str().expect("a line has a key"))
        .collect();
    assert_eq!(
        exported_keys.len(),
        BULK_KEYS,
        "an export of {BULK_KEYS} keys — more than {EXPORT_PAGE} to a page — writes a line for \
         every one"
    );
    if let Some(at) = exported_keys
        .iter()
        .zip(&in_byte_order)
        .position(|(held, key)| held != key)
    {
        panic!(
            "an export longer than a page writes each key exactly once, in byte order: line {} \
             is `{}` where `{}` belongs",
            at + 1,
            exported_keys[at],
            in_byte_order[at]
        );
    }
    for line in &paged_lines {
        assert_eq!(
            line["value"]["text"], line["key"],
            "each line carries its own key's value, across the pages: {line}"
        );
    }
    assert_eq!(
        serde_json::from_str::<Value>(paged.stderr().trim()).expect("the summary is JSON"),
        json!({ "written": BULK_KEYS }),
        "the summary counts every page's lines"
    );

    // A line the schema refuses fails the import, naming it, and writes
    // nothing — exit `2`: nothing was read or written and the file is what is
    // to fix, so it is usage, as a `run --input` that does not fit its type is,
    // and not one of the ruling's `1`s — a key missing, a condition declined, a
    // backend down — that a supervisor retrying on `1` would retry for ever.
    let refusing = [
        json!({ "key": "refused/ok", "value": { "text": "fine" } }).to_string(),
        json!({ "key": "refused/bad", "value": { "text": "" } }).to_string(),
        json!({ "key": "refused/typo", "value": { "text": "x" }, "expire_at": "2099-01-01T00:00:00Z" })
            .to_string(),
    ]
    .join("\n");
    let refused = project.store_with(
        "import",
        &[&notes],
        Given {
            input: Some(&refusing),
            ..Given::default()
        },
    );
    refused.exits(2);
    assert!(
        refused.stderr().contains("line 2:")
            && refused.stderr().contains("text")
            && refused.stderr().contains("line 3:")
            && refused.stderr().contains("expire_at")
            && !refused.stderr().contains("line 1:"),
        "a refused import names each line it refused, and why:\n{}",
        refused.stderr()
    );
    let refused = project.store_with(
        "import",
        &[&notes, "--format", "json"],
        Given {
            input: Some(&refusing),
            ..Given::default()
        },
    );
    let report = refused.exits(2).json();
    assert_eq!(
        report["lines"]
            .as_array()
            .expect("the refusal lists its lines")
            .iter()
            .map(|line| line["line"].clone())
            .collect::<Vec<_>>(),
        vec![json!(2), json!(3)]
    );
    project.store("get", &[&notes, "refused/ok"]).exits(1);
    let set_refused = project.store("set", &[&notes, "refused/set", r#"{"text":3}"#]);
    assert!(
        set_refused.exits(2).stderr().contains("refused the value"),
        "a set's value is held to the schema too, and is usage as a value that is not JSON \
         is:\n{}",
        set_refused.stderr()
    );
    assert!(
        project
            .store(
                "set",
                &[&notes, "refused/set", r#"{"text":3}"#, "--format", "json"]
            )
            .exits(2)
            .json()["error"]
            .as_str()
            .is_some_and(|error| error.contains("refused the value")),
        "…answered as `{{ error }}` under `--format json`, naming the issues"
    );
    project
        .store("set", &[&notes, "refused/set", "{text"])
        .exits(2);
    project.store("get", &[&notes, "refused/set"]).exits(1);

    // A backend that fails part-way through an import is another matter: the
    // lines before the one it refused are written, and stay written, so the
    // command says which line it stopped at and how many it wrote before it —
    // exit `1`, the store failing rather than the file (D155, the `cli` topic).
    // Every line here is one the schema admits, so it is the backend that
    // refuses the one marked: on SQLite a trigger this case installs on the
    // store's own file, and on a server a key its own column will not hold —
    // longer than MySQL's `VARCHAR(2048)`, or holding the NUL a Postgres
    // `text` never can.
    let store_file = project
        .out
        .join(".agent-compose/stores")
        .join(format!("{}.sqlite", project.notes));
    let unwritable = match project.provider {
        "sqlite" => {
            let installed = sqlite_sql(
                &project.out,
                &store_file,
                "CREATE TRIGGER partway_refused BEFORE INSERT ON entries \
                 WHEN NEW.key = 'partway/refused' \
                 BEGIN SELECT RAISE(ABORT, 'the case refuses this key'); END;",
            );
            assert!(
                installed.status.success(),
                "the trigger is installed on {}:\n{}",
                store_file.display(),
                String::from_utf8_lossy(&installed.stderr)
            );
            "partway/refused".to_string()
        }
        "mysql" => format!("partway/{}", "k".repeat(3000)),
        _ => "partway/\u{0}".to_string(),
    };
    // One line per name, keyed under `partway/` — and `refused`, the key the
    // backend will not take.
    let partway = |names: &[&str]| {
        names
            .iter()
            .map(|name| {
                let key = if *name == "refused" {
                    unwritable.clone()
                } else {
                    format!("partway/{name}")
                };
                format!("{}\n", json!({ "key": key, "value": { "text": name } }))
            })
            .collect::<String>()
    };
    let stopped = project.store_with(
        "import",
        &[&notes],
        Given {
            input: Some(&partway(&["a", "refused", "c"])),
            ..Given::default()
        },
    );
    assert!(
        stopped
            .exits(1)
            .stderr()
            .contains("error: line 2 could not be written (")
            && stopped
                .stderr()
                .contains("); the 1 line before it was written, and nothing after it was"),
        "an import a backend stopped part-way names the line it stopped at and how many were \
         written before it:\n{}",
        stopped.stderr()
    );
    project.store("get", &[&notes, "partway/a"]).exits(0);
    project.store("get", &[&notes, "partway/c"]).exits(1);
    let stopped = project.store_with(
        "import",
        &[&notes, "--format", "json"],
        Given {
            input: Some(&partway(&["a", "b", "refused", "c"])),
            ..Given::default()
        },
    );
    let error = stopped.exits(1).json();
    assert!(
        error["error"].as_str().is_some_and(|error| {
            error.starts_with("line 3 could not be written (")
                && error
                    .ends_with("); the 2 lines before it were written, and nothing after it was")
        }),
        "…answered as `{{ error }}` under `--format json`: {error}"
    );
    assert_eq!(
        error
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["error"],
        "{error}"
    );
    project.store("get", &[&notes, "partway/b"]).exits(0);
    project.store("get", &[&notes, "partway/c"]).exits(1);
    for key in ["partway/a", "partway/b"] {
        project.store("delete", &[&notes, key]).exits(0);
    }
    if project.provider == "sqlite" {
        let dropped = sqlite_sql(&project.out, &store_file, "DROP TRIGGER partway_refused;");
        assert!(
            dropped.status.success(),
            "the trigger is dropped from {}:\n{}",
            store_file.display(),
            String::from_utf8_lossy(&dropped.stderr)
        );
    }

    // `--if-absent` seeds without overwriting — and `--file` is read from where
    // the command was typed, not from the build directory the child runs in.
    project
        .store("set", &[&notes, "seed/a", r#"{"text":"kept"}"#])
        .exits(0);
    std::fs::write(
        project.source.path().join("seed.jsonl"),
        format!(
            "{}\n{}\n",
            json!({ "key": "seed/a", "value": { "text": "overwritten" } }),
            json!({ "key": "seed/b", "value": { "text": "new" } })
        ),
    )
    .expect("the scratch area is writable");
    let seeded = project.store_with(
        "import",
        &[
            &notes,
            "--if-absent",
            "--file",
            "seed.jsonl",
            "--format",
            "json",
        ],
        Given {
            directory: Some(project.source.path()),
            ..Given::default()
        },
    );
    assert_eq!(
        seeded.exits(0).json(),
        json!({ "read": 2, "written": 1, "skipped": 1, "expired": 0, "present": 1 })
    );
    assert_eq!(
        project
            .store("get", &[&notes, "seed/a", "--format", "json"])
            .exits(0)
            .json()["value"],
        json!({ "text": "kept" })
    );
    assert_eq!(
        project
            .store("get", &[&notes, "seed/b", "--format", "json"])
            .exits(0)
            .json()["value"],
        json!({ "text": "new" })
    );

    // A store's `default_ttl:` — an hour, on `leases`: a `set` with no `--ttl`
    // takes it, `--ttl never` overrides it, and an import line with no
    // `expires_at` writes a key that never expires, whatever the default, so an
    // export and its import leave the same keys alive (D155).
    let leases = format!("store.{}", project.leases);
    let before_write = now_millis();
    project
        .store("set", &[&leases, "defaulted", r#"{"text":"lapses"}"#])
        .exits(0);
    let after_write = now_millis();
    project
        .store(
            "set",
            &[&leases, "forever", r#"{"text":"stays"}"#, "--ttl", "never"],
        )
        .exits(0);
    let imported = project.store_with(
        "import",
        &[&leases, "--format", "json"],
        Given {
            input: Some(&format!(
                "{}\n",
                json!({ "key": "imported", "value": { "text": "stays" } })
            )),
            ..Given::default()
        },
    );
    assert_eq!(
        imported.exits(0).json(),
        json!({ "read": 1, "written": 1, "skipped": 0, "expired": 0, "present": 0 })
    );
    let held = by_key(&project.store("export", &[&leases]).exits(0).lines());
    assert_eq!(
        held.keys().collect::<Vec<_>>(),
        ["defaulted", "forever", "imported"]
    );
    assert!(
        held["defaulted"]["expires_at"].is_string(),
        "a `set` with no `--ttl` takes the store's `default_ttl: 1h`, so the key expires: {}",
        held["defaulted"]
    );
    let lapses = instant(&held["defaulted"]);
    assert!(
        (before_write + HOUR - CLOCK_SLACK..=after_write + HOUR + CLOCK_SLACK).contains(&lapses),
        "a `set` with no `--ttl` takes the store's `default_ttl: 1h`, so it expires an hour after \
         the write, made between {before_write} and {after_write}: {}",
        held["defaulted"]
    );
    assert!(
        held["forever"].get("expires_at").is_none(),
        "`--ttl never` overrides the store's default: {}",
        held["forever"]
    );
    assert!(
        held["imported"].get("expires_at").is_none(),
        "an import line with no `expires_at` writes a key that never expires, whatever the \
         store's `default_ttl:` says: {}",
        held["imported"]
    );
    // …and every read agrees, once the store's clock is past the default's hour.
    let later = project.store_with(
        "list",
        &[&leases, "--format", "json"],
        Given {
            environment: &[(CLOCK_OFFSET, "7200000")],
            ..Given::default()
        },
    );
    assert_eq!(
        later.exits(0).json(),
        json!({ "keys": ["forever", "imported"] }),
        "past an hour, the defaulted key is gone and the two that never expire are not"
    );

    // A session's partition, by the session identity.
    project
        .store(
            "set",
            &[
                &visits,
                "seed/a",
                r#"{"text":"came"}"#,
                "--session",
                SESSION,
            ],
        )
        .exits(0);
    project
        .store("get", &[&visits, "seed/a", "--session", SESSION])
        .exits(0);
    project
        .store("get", &[&visits, "seed/a", "--session", "user 2"])
        .exits(1);

    // The wrong scope flags, an unknown address and a store that is not `kv`
    // are usage errors.
    for (arguments, says) in [
        (
            vec![notes.as_str(), "k", "--session", SESSION],
            "scope: global",
        ),
        (
            vec![notes.as_str(), "k", "--execution", "exec_x"],
            "scope: global",
        ),
        (vec![visits.as_str(), "k"], "--session <key>"),
        (
            vec![visits.as_str(), "k", "--execution", "exec_x"],
            "scope: session",
        ),
        (
            vec!["store.nowhere", "k"],
            "is not a store of this composition",
        ),
        (vec![files.as_str(), "k"], "is a `blob` store"),
    ] {
        let refused = project.store("get", &arguments);
        assert!(
            refused.exits(2).stderr().contains(says),
            "`store get {}` names what is wrong ({says}):\n{}",
            arguments.join(" "),
            refused.stderr()
        );
    }
    project
        .store("get", &[&notes, "k", "--session", "a", "--execution", "b"])
        .exits(2);

    // A `scope: execution` store is rows on a server on a dialled arm, which
    // the command reaches; on SQLite it is each execution's own memory, which
    // it cannot, and says so.
    if project.provider == "sqlite" {
        for arguments in [
            vec![scratch.as_str(), "k", "--execution", "exec_x"],
            vec![scratch.as_str(), "k"],
        ] {
            let unreachable = project.store("get", &arguments);
            assert!(
                unreachable
                    .exits(2)
                    .stderr()
                    .contains("memory of the process"),
                "{}",
                unreachable.stderr()
            );
        }
    } else {
        for (arguments, says) in [
            (vec![scratch.as_str(), "k"], "--execution <id>"),
            (
                vec![scratch.as_str(), "k", "--session", SESSION],
                "scope: execution",
            ),
        ] {
            let refused = project.store("get", &arguments);
            assert!(
                refused.exits(2).stderr().contains(says),
                "`store get {}` names what is wrong ({says}):\n{}",
                arguments.join(" "),
                refused.stderr()
            );
        }
        project
            .store(
                "set",
                &[
                    &scratch,
                    "k",
                    r#"{"text":"mine"}"#,
                    "--execution",
                    "exec_cli",
                ],
            )
            .exits(0);
        project
            .store("get", &[&scratch, "k", "--execution", "exec_cli"])
            .exits(0);
        project
            .store("get", &[&scratch, "k", "--execution", "exec_other"])
            .exits(1);
        project
            .store("delete", &[&scratch, "k", "--execution", "exec_cli"])
            .exits(0);
    }

    // **Not an effect.** Nothing above opened a journal — there is not even a
    // file — and no write left a ledger row.
    let journal = project.out.join(".agent-compose/journal.sqlite");
    assert!(
        !journal.exists(),
        "an operator's op opens no journal: {}",
        journal.display()
    );
    assert!(
        !project.out.join(".agent-compose/traces").exists(),
        "…and files no trace"
    );
    assert_eq!(
        project.ledger_rows(),
        0,
        "an operator's write carries no idempotency key, so it leaves no ledger row"
    );

    // …and a flow run afterwards reads what the operator wrote, through the
    // partitions the command addressed.
    let run = project.recall("seed/a");
    run.exits(0);
    assert_eq!(
        serde_json::from_str::<Value>(&run.stdout()).expect("a run answers its outputs"),
        json!({ "found": true, "recalled": { "text": "kept" }, "visited": true }),
        "{}",
        run.stderr()
    );
    // The run's own reads are in the journal now, and none of the operator's
    // ops is: no row names a key only the command touched.
    let effects = journal_rows(&project.out, "SELECT request FROM effects");
    let rows = effects.as_array().expect("the journal answers rows");
    assert!(
        rows.iter().any(|row| row.to_string().contains("seed/a")),
        "the run journaled its own reads: {effects}"
    );
    for operator_only in [
        "page/", "lease", "brief", "other/x", "bulk/", "refused/", "partway/", "seed/b", "clocked/",
    ] {
        assert!(
            !rows
                .iter()
                .any(|row| row.to_string().contains(operator_only)),
            "the journal holds no row of the operator's `{operator_only}` ops: {effects}"
        );
    }
    assert_eq!(
        project.ledger_rows(),
        0,
        "a flow's reads leave no ledger row either"
    );

    // **The execution partition is the runtime's.** A flow writes its own
    // `scope: execution` partition and stops at a `human` pause nobody is there
    // to answer, so the execution stays open and its partition is not released;
    // handed the id that run printed, the command reads what the flow wrote —
    // which only a partition derived as the runtime derives it can find — and
    // another execution's partition holds nothing. (On SQLite those rows are
    // the run's own memory, gone with its process, which the command refuses
    // above.)
    if project.provider != "sqlite" {
        let parked = project.park("left/by-run");
        parked.exits(3);
        let said = parked.stderr();
        let execution = said
            .lines()
            .find_map(|line| line.strip_prefix("execution: "))
            .map(str::trim)
            .unwrap_or_else(|| panic!("the parked run names its execution:\n{said}"));
        assert!(
            execution.starts_with("exec_"),
            "an execution id: {execution}"
        );
        assert_eq!(
            project
                .store(
                    "get",
                    &[
                        &scratch,
                        "left/by-run",
                        "--execution",
                        execution,
                        "--format",
                        "json"
                    ],
                )
                .exits(0)
                .json(),
            json!({ "value": { "text": "left by the run" }, "found": true, "version": 1 }),
            "the command reads the partition the run's execution wrote"
        );
        assert_eq!(
            project
                .store(
                    "list",
                    &[&scratch, "--execution", execution, "--format", "json"],
                )
                .exits(0)
                .json(),
            json!({ "keys": ["left/by-run"] })
        );
        project
            .store(
                "get",
                &[&scratch, "left/by-run", "--execution", "exec_other"],
            )
            .exits(1);
    }

    history(&project, &notes);
}

/// **`store log` reads one key's history** on a store that declares
/// `history: true`, and an operator's write is recorded there as actor `cli`
/// (PRD resolved q74, q72).
///
/// A set, a declined conditional set, a conditional set that applies and a
/// delete record three rows — the declined one none — oldest first, each with
/// the value before and after, the version it left and the actor `cli`, which
/// names no execution. The table shows each row; `--format json` answers the
/// `history` op's own page; `--limit` and `--after` page it as a `list` pages
/// keys, with `next` absent on the last page; a store with no
/// `history_retention:` keeps every row however far its clock moves, and one
/// with an hour's keeps a row for that hour — compiled to the milliseconds the
/// runtime counts — and then neither answers it nor holds it once a command's
/// sweep has run; a key with no history answers an empty page and exits `0`,
/// as a `list` of no keys does; and a store without history, a cursor that is
/// not one — digits past the largest safe integer among them — and a page
/// size out of range are usage errors, `2`.
fn history(project: &Project, notes: &str) {
    let audits = format!("store.{}", project.audits);
    project
        .store("set", &[&audits, "acct", r#"{"text":"opened"}"#])
        .exits(0);
    project
        .store(
            "set",
            &[&audits, "acct", r#"{"text":"again"}"#, "--if", "absent"],
        )
        .exits(1);
    project
        .store(
            "set",
            &[&audits, "acct", r#"{"text":"moved"}"#, "--if-version", "1"],
        )
        .exits(0);
    project.store("delete", &[&audits, "acct"]).exits(0);

    let page = project
        .store("log", &[&audits, "acct", "--format", "json"])
        .exits(0)
        .json();
    let rows = page["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("a log answers the history op's page: {page}"));
    assert_eq!(
        rows.len(),
        3,
        "three writes landed, and the declined one recorded nothing: {page}"
    );
    assert!(page.get("next").is_none(), "one page holds them: {page}");
    let ops: Vec<&str> = rows.iter().filter_map(|row| row["op"].as_str()).collect();
    assert_eq!(ops, ["set", "set", "delete"], "{page}");
    for row in rows {
        assert_eq!(
            row["actor"],
            json!({ "kind": "cli" }),
            "an operator's write is recorded as actor `cli`, naming no execution: {page}"
        );
        assert!(row["at"].as_u64().is_some_and(|at| at > 0), "{page}");
    }
    assert!(
        rows[0]["seq"].as_u64() < rows[1]["seq"].as_u64()
            && rows[1]["seq"].as_u64() < rows[2]["seq"].as_u64(),
        "oldest first: {page}"
    );
    // No `history_retention:` is no cut-off: with the store's clock moved
    // ten years on, every row is still read.
    assert_eq!(
        project
            .store_with(
                "log",
                &[&audits, "acct", "--format", "json"],
                Given {
                    environment: &[(CLOCK_OFFSET, "315360000000")],
                    ..Given::default()
                },
            )
            .exits(0)
            .json(),
        page,
        "a store that declares no retention keeps every row for good"
    );
    assert!(rows[0].get("old_value").is_none(), "{page}");
    assert_eq!(rows[0]["new_value"], json!({ "text": "opened" }), "{page}");
    assert_eq!(rows[0]["version"], json!(1), "{page}");
    assert_eq!(rows[1]["old_value"], json!({ "text": "opened" }), "{page}");
    assert_eq!(rows[1]["new_value"], json!({ "text": "moved" }), "{page}");
    assert_eq!(rows[1]["version"], json!(2), "{page}");
    assert_eq!(rows[2]["old_value"], json!({ "text": "moved" }), "{page}");
    assert!(
        rows[2].get("new_value").is_none() && rows[2].get("version").is_none(),
        "a delete leaves no value and no version: {page}"
    );

    let shown = project.store("log", &[&audits, "acct"]).exits(0).stdout();
    let header = shown.lines().next().unwrap_or_default();
    for column in [
        "seq",
        "at",
        "op",
        "version",
        "actor",
        "old_value",
        "new_value",
    ] {
        assert!(
            header.split_whitespace().any(|title| title == column),
            "the table names `{column}`:\n{shown}"
        );
    }
    assert_eq!(
        shown.lines().count(),
        4,
        "a header and three rows:\n{shown}"
    );
    assert!(
        shown.contains(r#"{"text":"opened"}"#) && shown.contains(r#"{"text":"moved"}"#),
        "the table shows the values:\n{shown}"
    );
    assert!(
        shown.lines().skip(1).all(|line| line.contains(" cli ")),
        "every row names the operator:\n{shown}"
    );

    // Paged, as `list` pages keys.
    let first = project
        .store(
            "log",
            &[&audits, "acct", "--limit", "2", "--format", "json"],
        )
        .exits(0)
        .json();
    let next = first["next"]
        .as_str()
        .unwrap_or_else(|| panic!("a page with more after it names its cursor: {first}"))
        .to_string();
    assert_eq!(first["entries"], json!(rows[..2]), "{first}");
    assert_eq!(next, rows[1]["seq"].to_string(), "{first}");
    let second = project
        .store(
            "log",
            &[
                &audits, "acct", "--limit", "2", "--after", &next, "--format", "json",
            ],
        )
        .exits(0)
        .json();
    assert_eq!(
        second,
        json!({ "entries": [rows[2]] }),
        "the last page carries no cursor"
    );
    let paged = project
        .store("log", &[&audits, "acct", "--limit", "2"])
        .exits(0)
        .stdout();
    assert!(
        paged.contains(&format!("next: {next}")),
        "the table names the cursor of the page after it:\n{paged}"
    );

    // An import's writes are an operator's too: each line it writes is a `set`
    // recorded as actor `cli`, and the history the export did not carry is
    // not written back — the imported key's history begins at the import.
    let imported = project.store_with(
        "import",
        &[&audits, "--format", "json"],
        Given {
            input: Some(&format!(
                "{}\n",
                json!({ "key": "restored", "value": { "text": "back" }, "version": 7 })
            )),
            ..Given::default()
        },
    );
    assert_eq!(
        imported.exits(0).json(),
        json!({ "read": 1, "written": 1, "skipped": 0, "expired": 0, "present": 0 })
    );
    let restored = project
        .store("log", &[&audits, "restored", "--format", "json"])
        .exits(0)
        .json();
    let restored = restored["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("a log answers the history op's page: {restored}"));
    assert_eq!(restored.len(), 1, "one line, one row: {restored:?}");
    assert_eq!(restored[0]["op"], "set", "{restored:?}");
    assert_eq!(
        restored[0]["actor"],
        json!({ "kind": "cli" }),
        "{restored:?}"
    );
    assert_eq!(
        restored[0]["new_value"],
        json!({ "text": "back" }),
        "{restored:?}"
    );
    assert!(
        restored[0].get("old_value").is_none(),
        "an import writes no history back, so its row is the key's first: {restored:?}"
    );

    // A `history_retention:` keeps a row for that long by the store's clock
    // and no longer (PRD resolved q74), and reaches the runtime in the
    // milliseconds its clock counts. With the clock moved half an hour —
    // inside the hour — the row is read, which a retention bound in seconds
    // (an hour read as 3.6 s) would already have hidden; moved two hours, past
    // it, the read answers no row, and the command's own store-wide sweep has
    // removed it from storage by the time the command exits — that store's
    // rows only, never another store's.
    let trail = format!("store.{}", project.trail);
    project
        .store("set", &[&trail, "acct", r#"{"text":"brief"}"#])
        .exits(0);
    let fresh = project
        .store("log", &[&trail, "acct", "--format", "json"])
        .exits(0)
        .json();
    let fresh_rows = fresh["entries"]
        .as_array()
        .unwrap_or_else(|| panic!("a log answers the history op's page: {fresh}"));
    assert_eq!(fresh_rows.len(), 1, "{fresh}");
    assert_eq!(
        fresh_rows[0]["new_value"],
        json!({ "text": "brief" }),
        "{fresh}"
    );
    assert_eq!(
        project
            .store_with(
                "log",
                &[&trail, "acct", "--format", "json"],
                Given {
                    environment: &[(CLOCK_OFFSET, "1800000")],
                    ..Given::default()
                },
            )
            .exits(0)
            .json(),
        fresh,
        "half an hour on, the row is inside its hour's retention"
    );
    assert_eq!(
        project.history_rows(&project.trail),
        1,
        "a read inside the retention sweeps nothing"
    );
    let audited = project.history_rows(&project.audits);
    assert_eq!(
        project
            .store_with(
                "log",
                &[&trail, "acct", "--format", "json"],
                Given {
                    environment: &[(CLOCK_OFFSET, "7200000")],
                    ..Given::default()
                },
            )
            .exits(0)
            .json(),
        json!({ "entries": [] }),
        "two hours on, the row is past its retention and no read answers it"
    );
    assert_eq!(
        project.history_rows(&project.trail),
        0,
        "…and the command's sweep removed it from storage before it exited"
    );
    assert_eq!(
        project.history_rows(&project.audits),
        audited,
        "a store with no retention loses no row to another store's sweep"
    );

    // A key with no history is an empty page, not a failure.
    let none = project.store("log", &[&audits, "nobody"]);
    none.exits(0);
    assert!(none.stdout().is_empty(), "{}", none.stdout());
    assert!(
        none.stderr().contains("has no history"),
        "{}",
        none.stderr()
    );
    assert_eq!(
        project
            .store("log", &[&audits, "nobody", "--format", "json"])
            .exits(0)
            .json(),
        json!({ "entries": [] })
    );

    // What the command cannot do is usage.
    let kept_none = project.store("log", &[notes, "acct"]);
    assert!(
        kept_none.exits(2).stderr().contains("keeps no history"),
        "{}",
        kept_none.stderr()
    );
    // A cursor is a whole number the op can hold, so digits past the largest
    // safe integer are refused as usage before the op runs, as a word or a
    // fraction is — never the op's own failure, which would exit `1`.
    for cursor in ["seven", "1.5", "99999999999999999999", "9007199254740992"] {
        let malformed = project.store("log", &[&audits, "acct", "--after", cursor]);
        let stderr = malformed.exits(2).stderr();
        assert!(
            stderr.contains("is not a history cursor")
                && stderr.contains(&format!("`--after {cursor}`")),
            "`--after {cursor}`: {stderr}"
        );
    }
    // The largest safe integer is a cursor: past every row, an empty page.
    assert_eq!(
        project
            .store(
                "log",
                &[
                    &audits,
                    "acct",
                    "--after",
                    "9007199254740991",
                    "--format",
                    "json"
                ],
            )
            .exits(0)
            .json(),
        json!({ "entries": [] })
    );
    project
        .store("log", &[&audits, "acct", "--limit", "0"])
        .exits(2);
}

/// A command whose standard output will not take its answer, on one arm — and
/// the exit code it ends with all the same, which is **the op's own** (PRD
/// resolved q72's exit table, the `cli` topic's rule for a reader that stops
/// reading).
///
/// A reader that has gone is never a failure: a `get` that found its key and a
/// `set` that applied end `0`, a `get` that found nothing and a `set` whose
/// condition declined end `1`, a `set` whose value the schema refused ends
/// `2`, an answer that was itself a failure is said on stderr instead, and an
/// `export` stops at the page the pipe refused and ends `0`, its summary
/// saying where. A standard output that fails under a reader still there — a
/// full disk — is said on stderr, and for an `export` is the export failing.
fn undelivered(arm: &Arm) {
    let Some(project) = Project::new(arm) else {
        return;
    };
    let notes = format!("store.{}", project.notes);
    for key in ["cut/a", "cut/b"] {
        project
            .store("set", &[&notes, key, &format!(r#"{{"text":"{key}"}}"#)])
            .exits(0);
    }

    // An export whose reader has gone stops there — rather than waiting for
    // ever on a pipe nobody will drain — and ends `0`, its summary on stderr
    // saying where it stopped: here before its first line.
    let cut = project.store_into_closed_pipe("export", &[&notes]);
    assert!(
        cut.exits(0).stderr().contains("standard output closed")
            && cut
                .stderr()
                .contains("after 0 lines, so the export stopped there and read nothing more"),
        "an export into a closed pipe says it stopped, and where:\n{}",
        cut.stderr()
    );
    assert!(!cut.stderr().contains("error"), "{}", cut.stderr());
    let cut = project.store_into_closed_pipe("export", &[&notes, "--format", "json"]);
    let said: Value = serde_json::from_str(cut.exits(0).stderr().trim())
        .unwrap_or_else(|error| panic!("one JSON object on stderr ({error}):\n{}", cut.stderr()));
    assert_eq!(
        said.as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["stopped", "written"],
        "{said}"
    );
    assert_eq!(said["written"], json!(0), "{said}");
    assert!(
        said["stopped"]
            .as_str()
            .is_some_and(|stopped| stopped.contains("standard output closed")),
        "{said}"
    );

    // Every other sub-verb ends with its op's own code, in both formats, and
    // says nothing more about a reader that chose to leave.
    for (sub_verb, arguments, code) in [
        ("get", vec![notes.as_str(), "cut/a"], 0),
        ("get", vec![notes.as_str(), "cut/a", "--format", "json"], 0),
        (
            "get",
            vec![notes.as_str(), "cut/nothing", "--format", "json"],
            1,
        ),
        ("list", vec![notes.as_str(), "--prefix", "cut/"], 0),
        ("count", vec![notes.as_str(), "--format", "json"], 0),
        (
            "set",
            vec![
                notes.as_str(),
                "cut/a",
                r#"{"text":"again"}"#,
                "--if",
                "absent",
            ],
            1,
        ),
    ] {
        let cut = project.store_into_closed_pipe(sub_verb, &arguments);
        cut.exits(code);
        assert!(
            !cut.stderr().contains("standard output"),
            "a reader that left is not this command's to report:\n{}",
            cut.stderr()
        );
    }
    // A write whose report nobody read applied, and says so with `0` — so a
    // supervisor retrying on `1` does not write it twice.
    project
        .store_into_closed_pipe(
            "set",
            &[
                &notes,
                "cut/set",
                r#"{"text":"written"}"#,
                "--if",
                "absent",
                "--format",
                "json",
            ],
        )
        .exits(0);
    assert_eq!(
        project
            .store("get", &[&notes, "cut/set", "--format", "json"])
            .exits(0)
            .json()["value"],
        json!({ "text": "written" }),
        "the write whose report was lost was made"
    );
    project
        .store_into_closed_pipe("delete", &[&notes, "cut/set"])
        .exits(0);
    project.store("get", &[&notes, "cut/set"]).exits(1);
    // An answer that was itself a failure is said on stderr instead.
    let refused = project.store_into_closed_pipe(
        "set",
        &[&notes, "cut/refused", r#"{"text":3}"#, "--format", "json"],
    );
    let said: Value =
        serde_json::from_str(refused.exits(2).stderr().trim()).unwrap_or_else(|error| {
            panic!(
                "the lost failure is one JSON object on stderr ({error}):\n{}",
                refused.stderr()
            )
        });
    assert!(
        said["error"]
            .as_str()
            .is_some_and(|error| error.contains("refused the value")),
        "{said}"
    );

    // A standard output that fails under a reader still there is another
    // matter: a full disk under an export's redirect is the export failing,
    // since the file does not hold the partition — exit `1`, said on stderr —
    // while an op's answer lost to it still ends with the op's code, said.
    let Some(full) = project.store_into_full_disk("export", &[&notes, "--format", "json"]) else {
        return;
    };
    let said: Value = serde_json::from_str(full.exits(1).stderr().trim())
        .unwrap_or_else(|error| panic!("one JSON object on stderr ({error}):\n{}", full.stderr()));
    assert!(
        said["error"].as_str().is_some_and(|error| {
            error.contains("standard output failed") && error.contains("ENOSPC")
        }) && said["written"] == json!(0),
        "{said}"
    );
    let full = project
        .store_into_full_disk("set", &[&notes, "cut/full", r#"{"text":"written"}"#])
        .expect("`/dev/full` is still there");
    assert!(
        full.exits(0).stderr().contains("standard output failed")
            && full.stderr().contains("the exit code is its own"),
        "{}",
        full.stderr()
    );
    project.store("get", &[&notes, "cut/full"]).exits(0);
}

/// A target whose server is not there, on one dialled arm: the ruling's
/// "backend failure" is exit `1`, never a usage error's `2` and never an
/// unhandled rejection, from every sub-verb and in both formats.
fn no_server(arm: &Arm) {
    let Some(project) = Project::new(arm) else {
        return;
    };
    let notes = format!("store.{}", project.notes);
    // A server that is not there is a failing backend: exit `1` from every
    // sub-verb, the driver's reason on stderr, `{ error }` under
    // `--format json` — on stdout, or on stderr for `export`, whose stdout
    // is its lines and stays empty.
    let dead = dead_server(project.provider);
    let dialling_nothing = Given {
        environment: &[(STORE_URL, dead.as_str())],
        ..Given::default()
    };
    let line = format!(
        "{}\n",
        json!({ "key": "dead/k", "value": { "text": "never" } })
    );
    for (sub_verb, arguments) in [
        ("get", vec![notes.as_str(), "dead/k"]),
        ("set", vec![notes.as_str(), "dead/k", r#"{"text":"never"}"#]),
        ("delete", vec![notes.as_str(), "dead/k"]),
        ("list", vec![notes.as_str()]),
        ("count", vec![notes.as_str()]),
        ("export", vec![notes.as_str()]),
        ("import", vec![notes.as_str()]),
    ] {
        for json_format in [false, true] {
            let mut arguments = arguments.clone();
            if json_format {
                arguments.extend(["--format", "json"]);
            }
            let failed = project.store_with(
                sub_verb,
                &arguments,
                Given {
                    input: (sub_verb == "import").then_some(line.as_str()),
                    ..dialling_nothing
                },
            );
            failed.exits(1);
            let answer: Option<Value> = if !json_format {
                assert!(
                    failed.stderr().contains("error: ") && failed.stderr().contains("ECONNREFUSED"),
                    "`store {sub_verb}` against no server names the refused connection:\n{}",
                    failed.stderr()
                );
                None
            } else if sub_verb == "export" {
                Some(
                    serde_json::from_str(failed.stderr().trim()).unwrap_or_else(|error| {
                        panic!(
                            "an export's failure is one JSON object on stderr ({error}):\n{}",
                            failed.stderr()
                        )
                    }),
                )
            } else {
                Some(failed.json())
            };
            if sub_verb == "export" {
                assert_eq!(
                    failed.stdout(),
                    "",
                    "an export that read nothing wrote no line"
                );
            }
            if let Some(answer) = answer {
                let error = answer["error"].as_str().unwrap_or_else(|| {
                    panic!("`store {sub_verb}` answers `{{ error }}`: {answer}")
                });
                assert!(error.contains("ECONNREFUSED"), "{answer}");
                let expected: &[&str] = if sub_verb == "export" {
                    &["error", "written"]
                } else {
                    &["error"]
                };
                assert_eq!(
                    answer
                        .as_object()
                        .expect("an object")
                        .keys()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                    expected,
                    "{answer}"
                );
            }
        }
    }
}

/// A URL of `provider`'s on a loopback port nothing listens on — taken from the
/// operating system and let go — so a connection to it is refused at once.
fn dead_server(provider: &str) -> String {
    let port = TcpListener::bind("127.0.0.1:0")
        .expect("a loopback port")
        .local_addr()
        .expect("a bound address")
        .port();
    format!("{provider}://agent_compose:unused@127.0.0.1:{port}/agent_compose")
}

#[test]
fn the_store_command_reads_and_writes_a_sqlite_store_through_local() {
    drive(&SQLITE);
}

#[test]
fn the_store_command_reads_and_writes_a_postgres_store_through_its_target() {
    drive(&POSTGRES);
}

#[test]
fn the_store_command_reads_and_writes_a_mysql_store_through_its_target() {
    drive(&MYSQL);
}

#[test]
fn a_store_answer_stdout_does_not_take_still_ends_with_the_ops_exit_code_on_sqlite() {
    undelivered(&SQLITE);
}

#[test]
fn a_store_answer_stdout_does_not_take_still_ends_with_the_ops_exit_code_on_postgres() {
    undelivered(&POSTGRES);
}

#[test]
fn a_store_answer_stdout_does_not_take_still_ends_with_the_ops_exit_code_on_mysql() {
    undelivered(&MYSQL);
}

#[test]
fn a_store_command_against_a_postgres_server_that_is_not_there_exits_1() {
    no_server(&POSTGRES);
}

#[test]
fn a_store_command_against_a_mysql_server_that_is_not_there_exits_1() {
    no_server(&MYSQL);
}

/// A store a **named** target binds to `provider: memory` is refused, exit `2`,
/// by every sub-verb and for every scope — never answered out of the empty
/// database the command's own process would open for it (D155, the `cli`
/// topic: "any store bound to `provider: memory` … since this command is
/// another process").
///
/// The rows of such a store live only in the memory of the process running each
/// execution, so an answer would be a lie of either kind: a `get` saying "not
/// found", exit `1`, of a key a running flow holds, or a `set` saying
/// "applied", exit `0`, of a row that dies with the command. `--target local`'s
/// `scope: execution` stores are the other half of the same refusal, and
/// [`drive`] asks it of the SQLite arm; this is the half a deploy file chooses.
#[test]
fn a_store_a_target_binds_to_memory_is_refused_rather_than_read_from_an_empty_one() {
    let Some(project) = Project::new(&MEMORY) else {
        return;
    };
    let notes = format!("store.{}", project.notes);
    let visits = format!("store.{}", project.visits);
    let scratch = format!("store.{}", project.scratch);
    let line = format!(
        "{}\n",
        json!({ "key": "memory/k", "value": { "text": "lost" } })
    );
    for (sub_verb, arguments) in [
        ("get", vec![notes.as_str(), "memory/k"]),
        ("get", vec![notes.as_str(), "memory/k", "--format", "json"]),
        (
            "set",
            vec![notes.as_str(), "memory/k", r#"{"text":"lost"}"#],
        ),
        (
            "set",
            vec![
                notes.as_str(),
                "memory/k",
                r#"{"text":"lost"}"#,
                "--format",
                "json",
            ],
        ),
        ("delete", vec![notes.as_str(), "memory/k"]),
        ("list", vec![notes.as_str()]),
        ("count", vec![notes.as_str(), "--format", "json"]),
        ("export", vec![notes.as_str()]),
        ("import", vec![notes.as_str()]),
        (
            "get",
            vec![visits.as_str(), "memory/k", "--session", SESSION],
        ),
        (
            "get",
            vec![scratch.as_str(), "memory/k", "--execution", "exec_x"],
        ),
    ] {
        let refused = project.store_with(
            sub_verb,
            &arguments,
            Given {
                input: (sub_verb == "import").then_some(line.as_str()),
                ..Given::default()
            },
        );
        refused.exits(2);
        assert!(
            refused.stderr().contains("is bound to `provider: memory`")
                && refused.stderr().contains("memory of the process"),
            "`store {sub_verb} {}` says the store's rows are not this process's to reach:\n{}",
            arguments.join(" "),
            refused.stderr()
        );
        assert_eq!(
            refused.stdout(),
            "",
            "`store {sub_verb} {}` answers nothing: an empty database's answer would be a \
             wrong one",
            arguments.join(" ")
        );
    }
}

/// clap's own pages for the verb and each sub-verb, byte for byte.
///
/// `--help` is the first thing a coding agent reads (PRD resolved q23), and a
/// sub-verb's usage line is what the discovery binds count required arguments
/// from — so a renamed placeholder or a dropped flag is a reviewed diff here
/// rather than a document that quietly stopped matching.
#[test]
fn the_store_help_pages_are_pinned() {
    for command in [
        &["store"][..],
        &["store", "get"],
        &["store", "set"],
        &["store", "delete"],
        &["store", "list"],
        &["store", "count"],
        &["store", "export"],
        &["store", "import"],
        &["store", "log"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
            .args(command)
            .arg("--help")
            .env("NO_COLOR", "1")
            .output()
            .expect("the command runs");
        assert_eq!(output.status.code(), Some(0), "{command:?} --help");
        insta::assert_snapshot!(
            format!("{}_help", command.join("_")),
            String::from_utf8(output.stdout).expect("help is UTF-8")
        );
    }
}

/// What clap refuses before anything is built: a sub-verb that does not exist,
/// a missing operand, two conditions, a value `--if` does not take, a page size
/// out of range, and both partition flags at once — every one exit `2`, with
/// clap's own words for it.
///
/// Each names a composition that is there and a store it declares, so the only
/// thing wrong with the command line is the thing the case is about. The exit
/// code alone cannot say clap refused it: a missing entrypoint exits `2` too,
/// and so does a case that got past clap — refused again by the emitted verb in
/// words of its own, or stopped by a build directory with nothing installed in
/// it. What tells clap's refusal apart is what it says and that nothing was
/// built — the `--out` it was handed does not exist, and nothing else was
/// written beside the composition.
#[test]
fn the_store_command_line_refuses_what_it_cannot_parse() {
    let source = Scratch::new("store-cli-unparsed");
    let entrypoint = source.path().join("main.yml");
    let suffix = run_unique();
    std::fs::write(&entrypoint, composition(&suffix)).expect("the scratch area is writable");
    let main = entrypoint.to_str().expect("a UTF-8 scratch path");
    let notes = format!("store.notes_{suffix}");
    let visits = format!("store.visits_{suffix}");
    let out = source.path().join("build");
    let built = out.to_str().expect("a UTF-8 scratch path");
    let cases: [(Vec<&str>, &[&str]); 8] = [
        (
            vec!["store"],
            &["Usage: agent-compose store <COMMAND>", "Commands:"],
        ),
        (
            vec!["store", "frob", "--out", built, main, &notes],
            &["error: unrecognized subcommand 'frob'"],
        ),
        (
            vec!["store", "get", "--out", built, main, &notes],
            &[
                "error: the following required arguments were not provided:",
                "<KEY>",
            ],
        ),
        (
            vec![
                "store",
                "set",
                "--out",
                built,
                main,
                &notes,
                "k",
                r#"{"text":"x"}"#,
                "--if",
                "absent",
                "--if-version",
                "1",
            ],
            &[
                "error: the argument '--if <CONDITION>' cannot be used with '--if-version <VERSION>'",
            ],
        ),
        (
            vec![
                "store",
                "set",
                "--out",
                built,
                main,
                &notes,
                "k",
                r#"{"text":"x"}"#,
                "--if",
                "maybe",
            ],
            &[
                "error: invalid value 'maybe' for '--if <CONDITION>'",
                "[possible values: absent, present]",
            ],
        ),
        (
            vec![
                "store", "list", "--out", built, main, &notes, "--limit", "0",
            ],
            &["error: invalid value '0' for '--limit <N>': 0 is not in 1..=1000"],
        ),
        (
            vec![
                "store", "list", "--out", built, main, &notes, "--limit", "1001",
            ],
            &["error: invalid value '1001' for '--limit <N>': 1001 is not in 1..=1000"],
        ),
        (
            vec![
                "store",
                "get",
                "--out",
                built,
                main,
                &visits,
                "k",
                "--session",
                "a",
                "--execution",
                "b",
            ],
            &["error: the argument '--session <KEY>' cannot be used with '--execution <ID>'"],
        ),
    ];
    for (arguments, says) in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
            .args(&arguments)
            .current_dir(source.path())
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .expect("the command runs");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(2),
            "`agent-compose {}` is a usage error:\n{stderr}",
            arguments.join(" "),
        );
        for said in says {
            assert!(
                stderr.contains(said),
                "`agent-compose {}` is refused by clap, which says `{said}`:\n{stderr}",
                arguments.join(" "),
            );
        }
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "",
            "`agent-compose {}` answers nothing on stdout",
            arguments.join(" "),
        );
        let beside: Vec<String> = std::fs::read_dir(source.path())
            .expect("the scratch area is readable")
            .map(|entry| {
                entry
                    .expect("an entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(
            beside,
            ["main.yml"],
            "`agent-compose {}` was refused before anything was built: neither its `--out` nor \
             any other build directory exists",
            arguments.join(" "),
        );
    }
}

/// A composition the compiler refuses is reported where the sub-verb keeps its
/// failures: as one JSON object on stdout for an op, whose answer the report
/// stands in for — as `validate`'s does — and on **stderr** for an `export`,
/// whose stdout is only ever the lines an import reads back, so a redirect
/// collecting them never collects a report instead (D155). Refused before
/// anything is built or launched, so it needs no backend and no runtime.
#[test]
fn a_refused_composition_is_reported_beside_an_exports_summary_not_among_its_lines() {
    let source = Scratch::new("store-cli-refused");
    let entrypoint = source.path().join("main.yml");
    let suffix = run_unique();
    let refusing = composition(&suffix).replacen(
        &format!("store: store.notes_{suffix}"),
        "store: store.nonexistent",
        1,
    );
    assert_ne!(
        refusing,
        composition(&suffix),
        "one node names a store nobody declares"
    );
    std::fs::write(&entrypoint, refusing).expect("the scratch area is writable");
    let store = format!("store.notes_{suffix}");
    let out = source.path().join("build");
    let refused = |sub_verb: &str, operands: &[&str]| {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
            .args(["store", sub_verb, "--out"])
            .arg(&out)
            .arg(&entrypoint)
            .arg(&store)
            .args(operands)
            .args(["--format", "json"])
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .output()
            .expect("the command runs");
        assert_eq!(
            output.status.code(),
            Some(1),
            "`store {sub_verb}` on a refused composition exits 1\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        output
    };
    let names_the_store = |report: &[u8], stream: &str| {
        let report: Value = serde_json::from_slice(report).unwrap_or_else(|error| {
            panic!(
                "the report is one JSON object on {stream} ({error}): {}",
                String::from_utf8_lossy(report)
            )
        });
        assert!(
            report["diagnostics"]
                .as_array()
                .expect("the report lists its diagnostics")
                .iter()
                .any(|diagnostic| diagnostic["code"] == "undefined-reference"),
            "{report}"
        );
    };

    let export = refused("export", &[]);
    assert_eq!(
        String::from_utf8_lossy(&export.stdout),
        "",
        "an export's stdout is only ever its lines, and a refused composition has none"
    );
    names_the_store(&export.stderr, "stderr");
    assert!(
        !out.exists(),
        "nothing was built for a composition the compiler refused"
    );

    let get = refused("get", &["k"]);
    names_the_store(&get.stdout, "stdout");
    assert_eq!(String::from_utf8_lossy(&get.stderr), "", "{get:?}");
}
