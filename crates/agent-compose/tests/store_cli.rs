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
//!    `count` that agrees, a `--ttl` that reads absent past its expiry, an
//!    `export` → edit → `import` round trip at fresh versions with a remaining
//!    `expires_at` honoured and an expired line skipped, a schema-refused import
//!    line named by number, `--if-absent` seeding without overwriting, the wrong
//!    scope flags and an unknown address exiting `2`, and the `--format json`
//!    shapes — [`drive`], once per arm;
//!  * **no effect**: no journal is opened by any of it, no ledger row is left,
//!    and a flow run afterwards reads what the operator wrote — the end of
//!    [`drive`];
//!  * clap's own surface for the verb and each sub-verb, pinned byte for byte —
//!    [`the_store_help_pages_are_pinned`].

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use harness::toolchain::required;
use harness::{Scratch, bun, journal_rows, scratch_project, seal, sqlite_rows};

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

/// One backend the command is driven against.
struct Arm {
    /// The `provider:` keyword, as grammar §14.3 spells it.
    provider: &'static str,
    /// The variable holding a reachable server, or `None` for SQLite.
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

/// The composition every arm is driven against, with its stores named `suffix`.
///
/// One store per scope a `kv` store may declare, a `blob` store the command
/// must refuse, and one flow that reads what an operator left — in the global
/// store and in one session's — which is how "a flow sees the operator's values"
/// is asked of the runtime rather than of this command.
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
    environment: Vec<(String, String)>,
    /// The run-unique names of the four stores, without `store.`.
    notes: String,
    visits: String,
    scratch: String,
    files: String,
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
            environment,
            notes: format!("notes_{suffix}"),
            visits: format!("visits_{suffix}"),
            scratch: format!("scratch_{suffix}"),
            files: format!("files_{suffix}"),
        })
    }

    /// `agent-compose store <sub-verb> <path> <arguments>…` under this arm's
    /// target and build directory.
    fn store(&self, sub_verb: &str, arguments: &[&str]) -> Answer {
        self.store_with(sub_verb, arguments, Given::default())
    }

    fn store_with(&self, sub_verb: &str, arguments: &[&str], given: Given<'_>) -> Answer {
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
        if self.provider == "sqlite" {
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
            "SELECT COUNT(*) AS n FROM store_applied WHERE store IN (?, ?, ?)",
            &json!([self.notes, self.visits, self.scratch]),
        ) {
            return count(&rows);
        }
        let mut held = 0;
        for store in [&self.notes, &self.visits] {
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
}

/// Every row this project's stores wrote is gone when the case ends: the build
/// directory with the SQLite files in it, and the rows on a dialled server.
impl Drop for Project {
    fn drop(&mut self) {
        if self.provider != "sqlite" {
            let script = self.script();
            for table in ["store_entries", "store_applied", "store_sequences"] {
                let sql = format!("DELETE FROM {table} WHERE store IN (?, ?, ?)");
                let mut command = bun();
                command
                    .arg(&script)
                    .args([self.provider, STORE_URL, &sql])
                    .arg(json!([self.notes, self.visits, self.scratch]).to_string());
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

    // A `--ttl`, and the key absent to every read once the clock is past it.
    // The command's first op on the store starts the process-start sweep at
    // that op's clock (PRD resolved q69), so a key read past its expiry is a
    // key removed too — which is why the export below writes a lease of its own.
    project
        .store(
            "set",
            &[&notes, "brief", r#"{"text":"held"}"#, "--ttl", "1h"],
        )
        .exits(0);
    project.store("get", &[&notes, "brief"]).exits(0);
    project
        .store_with(
            "get",
            &[&notes, "brief"],
            Given {
                environment: &[(CLOCK_OFFSET, "7200000")],
                ..Given::default()
            },
        )
        .exits(1);
    let counted = project.store_with(
        "count",
        &[&notes, "--format", "json"],
        Given {
            environment: &[(CLOCK_OFFSET, "7200000")],
            ..Given::default()
        },
    );
    assert_eq!(counted.exits(0).json(), json!({ "count": 4 }));
    let listed = project.store_with(
        "list",
        &[&notes, "--format", "json"],
        Given {
            environment: &[(CLOCK_OFFSET, "7200000")],
            ..Given::default()
        },
    );
    assert!(
        !listed.exits(0).json()["keys"]
            .as_array()
            .expect("a page has keys")
            .contains(&json!("brief")),
        "an expired key is not listed"
    );
    assert!(
        project
            .store("export", &[&notes])
            .exits(0)
            .lines()
            .iter()
            .all(|line| line["key"] != "brief"),
        "…nor exported"
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

    // A line the schema refuses fails the import, naming it, and writes nothing.
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
    refused.exits(1);
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
    let report = refused.exits(1).json();
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
        set_refused.exits(1).stderr().contains("refused the value"),
        "a set's value is held to the schema too:\n{}",
        set_refused.stderr()
    );

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
    for operator_only in ["page/", "lease", "brief", "other/x", "refused/", "seed/b"] {
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
/// a missing operand, two conditions, a page size out of range, and both
/// partition flags at once — every one exit `2`.
#[test]
fn the_store_command_line_refuses_what_it_cannot_parse() {
    for arguments in [
        &["store"][..],
        &["store", "frob", "main.yml", "store.notes"],
        &["store", "get", "main.yml", "store.notes"],
        &[
            "store",
            "set",
            "main.yml",
            "store.notes",
            "k",
            "{}",
            "--if",
            "absent",
            "--if-version",
            "1",
        ],
        &[
            "store",
            "set",
            "main.yml",
            "store.notes",
            "k",
            "{}",
            "--if",
            "maybe",
        ],
        &["store", "list", "main.yml", "store.notes", "--limit", "0"],
        &[
            "store",
            "list",
            "main.yml",
            "store.notes",
            "--limit",
            "1001",
        ],
        &[
            "store",
            "get",
            "main.yml",
            "store.notes",
            "k",
            "--session",
            "a",
            "--execution",
            "b",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
            .args(arguments)
            .env("NO_COLOR", "1")
            .output()
            .expect("the command runs");
        assert_eq!(
            output.status.code(),
            Some(2),
            "`agent-compose {}` is a usage error:\n{}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
