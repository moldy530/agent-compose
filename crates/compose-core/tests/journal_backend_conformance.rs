//! One contract, three backends: `docs/durability.md` run against each journal
//! a deploy file can bind (grammar §14.7, PRD resolved q62).
//!
//! # Why this suite exists
//!
//! The statements in the emitted `src/journal.ts` are **one** implementation of
//! `docs/durability.md`, shared by SQLite, Postgres and MySQL; an arm supplies a
//! connection, a schema and a writer guard and nothing else. That is what makes
//! the interface claim true by construction — a drift test in
//! `codegen::journal` holds it — and it is also what decides what is left to
//! prove here.
//!
//! What is left is everything a **server** answers, and none of it is visible to
//! `tsc` or to a fake:
//!
//!  * a collation that folds case, where two journal keys differing in case are
//!    two effects and the primary key has to tell them apart (§4);
//!  * a column too small for the payload §3.9 calls the largest thing this
//!    journal holds — MySQL's `TEXT` is 64 KiB, and a harness run's event stream
//!    is not;
//!  * a DDL that is not idempotent, so a second open of a journal this build
//!    already created fails rather than doing nothing (§10, §11.2);
//!  * an `AUTO_INCREMENT` that needs a key of its own, a `BIGSERIAL` that does
//!    not, and the park order both have to produce (`docs/distributed.md` §6.2);
//!  * a placeholder dialect — `?` against `$1` — and an upsert spelled two ways;
//!  * an advisory lock that is, or is not, released when a connection ends (§2).
//!
//! # How it runs, and what it does when a server is absent
//!
//! Every backend is **built and type-checked** on every run, server or no
//! server: neither needs one, and MySQL has no golden carrying its arm, so this
//! is the only place `src/codegen/js/journal-mysql.ts` meets `tsc` on a machine
//! with no database.
//!
//! The **cases** then run where there is something to run them against. SQLite
//! always: it needs nothing but the pinned driver, so a developer machine runs
//! the same cases CI does. Each remote provider runs when its environment
//! variable names a reachable server — [`POSTGRES_URL`] and [`MYSQL_URL`] — and
//! is **skipped loudly** when it does not, naming the variable to look at. Loudly
//! means through [`notice`] rather than `eprintln!`, which libtest swallows for
//! a test that passes. Never silently green: a skip that said nothing would make
//! a laptop's `cargo test` and CI's mean different things while reading the same.
//!
//! CI sets both, against `postgres:17` and `mysql:8` service containers declared
//! in `.github/workflows/ci.yml`. That is the ratified shape of PRD resolved
//! q62's verification clause: "the journal contract becomes a conformance suite
//! run against all three providers — SQLite always, Postgres and MySQL against
//! real servers as dockerized services in CI".

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[path = "support/toolchain.rs"]
mod toolchain;

#[path = "support/goldens.rs"]
mod goldens;

use goldens::repository;
use toolchain::{bun, installed, required, runner};

/// Where a reachable Postgres for the remote arm is named.
const POSTGRES_URL: &str = "AGENT_COMPOSE_TEST_POSTGRES_URL";

/// …and a MySQL.
const MYSQL_URL: &str = "AGENT_COMPOSE_TEST_MYSQL_URL";

/// One provider the suite drives, and how it is reached.
struct Backend {
    /// The `provider:` keyword, as grammar §14.7 spells it.
    provider: &'static str,
    /// The variable holding a reachable server, or `None` for the one that
    /// opens a file and needs nothing.
    variable: Option<&'static str>,
}

/// Every backend `journal:` can bind, in the order §14.7 lists them.
const BACKENDS: &[Backend] = &[
    Backend {
        provider: "sqlite",
        variable: None,
    },
    Backend {
        provider: "postgres",
        variable: Some(POSTGRES_URL),
    },
    Backend {
        provider: "mysql",
        variable: Some(MYSQL_URL),
    },
];

/// The composition every backend is driven against.
///
/// Deliberately the smallest one that builds: what is under test is the journal,
/// and every case the runner drives reaches it directly rather than through a
/// graph. A richer composition would make the emitted project slower to
/// type-check and would prove nothing more about a database.
const COMPOSITION: &str = "version: \"0.1\"\n\
flow.review:\n  \
  outputs: {}\n  \
  nodes:\n    \
    step: { exec: { command: \"true\" } }\n  \
  edges:\n    \
    - { from: start, to: step }\n    \
    - { from: step, to: end }\n";

/// The address the emitted project's journal dials, for a remote backend.
///
/// One variable per provider, named here rather than reused from the ambient
/// environment: the emitted `src/deployment.ts` carries the *name* the deploy
/// file wrote, so the runner's process has to hold that name — and holding the
/// suite's own variable under it keeps the deploy file honest about being an
/// `${ENV}` reference rather than an address.
const JOURNAL_URL: &str = "AGENT_COMPOSE_JOURNAL_URL";

/// Build one project whose target binds `provider`, and answer where it landed.
fn built(root: &Path, provider: &str) -> PathBuf {
    // Under `projects/`, which `.gitignore` already covers: everything this
    // suite writes is scratch, and a fixture left in the working tree would be a
    // composition nobody authored turning up as an untracked file.
    let source = root.join("projects").join("journal-sources").join(provider);
    let _ = fs::remove_dir_all(&source);
    fs::create_dir_all(source.join("deploy")).expect("the scratch area is writable");
    fs::write(source.join("main.yml"), COMPOSITION).expect("the entrypoint is writable");
    // `local` refuses the block (Decision D87), so the target has a name — which
    // is also the shape an operator writes, since a journal on a server is a
    // deployment rather than a laptop.
    let deploy = if provider == "sqlite" {
        // The default, written out. A target that declared nothing would bind
        // the same thing; writing it is what proves the keyword is live.
        format!("version: \"0.1\"\njournal:\n  provider: {provider}\n")
    } else {
        format!("version: \"0.1\"\njournal:\n  provider: {provider}\n  url: ${{{JOURNAL_URL}}}\n")
    };
    fs::write(source.join("deploy/remote.yml"), deploy).expect("the deploy file is writable");

    let entrypoint = source.join("main.yml");
    let resolution = compose_core::resolve_with_target(&entrypoint, "remote");
    assert!(
        resolution.diagnostics.is_empty(),
        "the `{provider}` fixture does not resolve: {:#?}",
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "the `{provider}` fixture does not validate: {diagnostics:#?}"
    );

    let authored = compose_core::Authored::read(&ir, &source)
        .expect("the fixture references no module binding");
    let destination = root.join("projects").join("journal").join(provider);
    let _ = fs::remove_dir_all(&destination);
    for file in compose_core::emit(&ir, &authored).files() {
        let target = destination.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(target.parent().expect("an emitted path has a parent"))
            .expect("the scratch area is writable");
        fs::write(&target, &file.contents).expect("an emitted file is writable");
    }
    destination
}

/// Say something a plain `cargo test` really prints.
///
/// libtest captures a **passing** test's output, so the skip notices below
/// written with `eprintln!` reached nobody: `cargo test` printed
/// `2 passed; 0 ignored` and nothing else, and a laptop run that drove one of
/// three backends read exactly like CI's run that drove all three — the state
/// this module's docs say a skip must never leave. The capture is installed on
/// the `print!`/`eprint!` family (`std::io::_print` and `_eprint` consult it);
/// a write to the process's own stderr handle is fd 2 and goes to the terminal.
fn notice(message: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
    let _ = stderr.flush();
}

/// Drive the contract runner against one built project, or `None` where the
/// backend's server is absent.
fn contract(root: &Path, backend: &Backend) -> Option<BTreeMap<String, Value>> {
    // **Built and type-checked before the server is asked for**, because
    // neither needs one and the two things that would go unchecked are not
    // small. A remote arm is emitted only into a project that pins its driver,
    // and this is where that pairing is checked for the two arms no golden
    // carries: the Postgres one rides along on `triage-fanout-staging`, and
    // **MySQL has no golden at all**, so a wrong `RowDataPacket` generic or a
    // renamed export from `mysql2/promise` would pass `cargo fmt`, `cargo
    // clippy` and `cargo test --workspace` on a machine with no MySQL and fail
    // only in CI. What an absent server skips is the **run**, below.
    let project = built(root, backend.provider);
    let checked = bun()
        .args(["run", "typecheck"])
        .current_dir(&project)
        .output()
        .expect("bun runs");
    assert!(
        checked.status.success(),
        "the `{}` journal project does not type-check:\n{}\n{}",
        backend.provider,
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr),
    );

    let address = match backend.variable {
        None => None,
        Some(variable) => match std::env::var(variable) {
            Ok(value) if !value.is_empty() => Some(value),
            _ => {
                assert!(
                    !required(),
                    "`{variable}` is required in CI: `.github/workflows/ci.yml` runs a \
                     `{}` service container for it, and this suite is what makes the \
                     `{}` journal a checked promise rather than a paragraph in \
                     `docs/durability.md` §10 (PRD resolved q62)",
                    backend.provider,
                    backend.provider,
                );
                notice(&format!(
                    "warning: skipping the `{}` journal conformance cases — `{variable}` is \
                     unset, so there is no server to drive them against. The project was still \
                     built and type-checked. It is set in CI (see .github/workflows/ci.yml); \
                     set it locally to run them.",
                    backend.provider
                ));
                return None;
            }
        },
    };

    let mut command = runner("journal-contract.mjs");
    command.arg(&project);
    if let Some(address) = address {
        command.env(JOURNAL_URL, address);
    }
    let output = command.output().expect("bun runs");
    assert!(
        output.status.success(),
        "the `{}` journal did not answer the contract:\n{}",
        backend.provider,
        String::from_utf8_lossy(&output.stderr),
    );
    Some(serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object"))
}

/// **The journal contract, against every backend a deploy file can bind.**
///
/// One set of cases and one set of expectations, because there is one contract:
/// `docs/durability.md` is backend-invariant everywhere but §2, and a case that
/// answered differently on two backends would be that document's promise broken
/// on one of them. The provider is in the failure message rather than in the
/// expectation.
#[test]
fn every_journal_backend_answers_the_same_contract() {
    let Some(root) = installed() else {
        return;
    };
    let mut driven: Vec<&str> = Vec::new();
    for backend in BACKENDS {
        let Some(answered) = contract(root, backend) else {
            continue;
        };
        driven.push(backend.provider);
        let provider = backend.provider;

        assert_eq!(
            answered.get("provider").and_then(Value::as_str),
            Some(provider),
            "the `{provider}` project opened a journal that says it is something else, so \
             `src/deployment.ts`'s binding and the arm `build` emitted have parted company"
        );
        assert_eq!(
            answered.get("journal_version").and_then(Value::as_u64),
            Some(journal_version()),
            "the `{provider}` journal reports a version `docs/durability.md` does not head \
             with (§11)"
        );

        for case in TRUE_EVERYWHERE {
            assert_eq!(
                answered.get(*case).and_then(Value::as_bool),
                Some(true),
                "the `{provider}` journal fails `{case}`, which `docs/durability.md` promises \
                 of every backend: {answered:#?}"
            );
        }

        // The effect history a redispatched node replays, in the key order §4
        // derives — which on a server is the column's collation answering, and
        // is why every key column is declared byte-wise (§10).
        assert_eq!(
            answered
                .get("effects_under_a_site")
                .and_then(Value::as_array),
            Some(&vec![
                Value::from("review/0#model/0"),
                Value::from("review/0#model/1"),
            ]),
            "the `{provider}` journal answers `effectsUnder` in another order or with other \
             rows, so a redispatch would replay a history this execution did not have \
             (`docs/durability.md` §4, `docs/distributed.md` §7.2)"
        );

        // Park order, which is the case a `map`'s fan-out makes: five rows
        // sharing one `parked_at`, drained in the order they went on the board
        // rather than in the order their `wait` strings sort.
        assert_eq!(
            answered
                .get("park_order_is_insertion_order")
                .and_then(Value::as_array),
            Some(&vec![
                Value::from("map/0/0"),
                Value::from("map/0/1"),
                Value::from("map/0/2"),
                Value::from("map/0/10"),
                Value::from("map/0/11"),
            ]),
            "the `{provider}` journal drains its board out of park order, so a one-worker \
             pool runs a `map`'s items 0, 1, 10, 11, 2 and the item that has waited longest \
             is not the one taken next (`docs/distributed.md` §6.2)"
        );

        // …and the writer guard, which is the one case §2 states per backend.
        let guard = answered
            .get("writer_guard_refuses_a_second_opener")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if backend.variable.is_none() {
            assert_eq!(
                guard, "not-applicable",
                "SQLite's lock is broken after a deadline by design, so a second opener goes \
                 in rather than being refused (`docs/durability.md` §2.2, §12)"
            );
        } else {
            assert_ne!(
                guard, "not-refused",
                "the `{provider}` journal let a second process open it while this one held \
                 it, so two writers would interleave into one record \
                 (`docs/durability.md` §2.3)"
            );
            for named in [
                "one process at a time",
                "released when that session ends",
                // …and the bound, which is the half a takeover needs. A dead
                // host's session holds the guard until its server notices, so a
                // refusal that only said "a process that died is not holding
                // this" would send an operator hunting for a live `serve` that
                // does not exist (§2.3).
                "300s",
            ] {
                assert!(
                    guard.contains(named),
                    "the `{provider}` journal refused a second opener without saying \
                     `{named}`, so a reader meets a lock with no account of who holds it or \
                     when it goes: {guard}"
                );
            }
        }

        // **…and the guard of a hub that is gone.** The case above is a *live*
        // second opener, which is the easy half. §2.3's headline property —
        // "a `serve` restarted on a fresh machine recovers every open
        // execution" — is a promise about taking over from a host that has
        // stopped, and a guard that outlived its holder refuses exactly that
        // takeover while the refusal text sends the operator hunting for a
        // `serve` that does not exist. The runner ends the holder's session
        // from another connection, which is the path the server's own reap
        // takes without the wait for it.
        let takeover = answered
            .get("a_takeover_is_not_locked_out_by_a_dead_session")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let after = answered
            .get("a_lost_connection_is_refused_rather_than_fatal")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if backend.variable.is_none() {
            assert_eq!(
                takeover, "not-applicable",
                "SQLite has no session to end; its stale lock is broken after a deadline \
                 instead (`docs/durability.md` §2.2)"
            );
        } else if let Some(reason) = takeover.strip_prefix("not-available: ") {
            // Not a failure and not silence: a server this runner may not ask
            // to end a session is a case that could not run, and saying which
            // is the difference between an unchecked promise and an unnoticed
            // one.
            notice(&format!(
                "warning: the `{provider}` journal's dead-owner case did not run ({reason}), so \
                 `docs/durability.md` §2.3's cross-host takeover is unchecked on this server. \
                 It needs a connection allowed to end another session of its own user — which \
                 the service containers in .github/workflows/ci.yml are."
            ));
        } else {
            assert_eq!(
                takeover, "admitted",
                "the `{provider}` journal refused a takeover after the session holding its \
                 writer guard had ended, so a `serve` restarted on a fresh machine is locked \
                 out of the record it exists to resume — which is the property a remote \
                 journal is for (`docs/durability.md` §2.3)"
            );
            // …and the process that lost that connection is still running,
            // which is only true because each arm listens for its driver's
            // `error` event: an `EventEmitter` emitting `error` with nothing
            // listening ends the process, and a `serve` ended that way loses
            // every in-flight execution's in-process state.
            assert_ne!(
                after, "answered",
                "the `{provider}` journal answered a statement over a connection whose session \
                 the server had ended, so it is reading something other than that server"
            );
            assert!(
                !after.starts_with("not-available"),
                "the `{provider}` journal's lost-connection case did not run: {after}"
            );
        }
    }
    assert!(
        driven.contains(&"sqlite"),
        "SQLite needs no server and is always driven; a run that skipped it read nothing"
    );
    notice(&format!(
        "note: the journal contract ran against {driven:?}; every backend was built and \
         type-checked"
    ));
}

/// **The runner and the module name one writer guard.**
///
/// The runner has to name MySQL's lock to find the session holding it —
/// `IS_USED_LOCK` takes the name — so that literal exists twice. A copy nothing
/// compares is a copy that drifts, and the drift is silent in the worst
/// direction: `IS_USED_LOCK` of a name nobody took answers `NULL`, the runner
/// reports the dead-owner case as unavailable, and the suite goes green having
/// checked the one thing it was added for in neither direction.
///
/// **Both halves of the name**, because MySQL's is two. User-level locks are
/// keyed on the name alone across the whole server — unlike Postgres' advisory
/// locks, which the server already scopes to the database — so the arm takes its
/// guard under `WRITER_GUARD` qualified with the schema it is connected to
/// (`MYSQL_GUARD_NAME`), and a runner that asked for either half alone would
/// find nothing.
#[test]
fn the_runner_and_the_module_take_one_writer_guard() {
    const MODULE: &str = include_str!("../src/codegen/js/journal.ts");
    const ARM: &str = include_str!("../src/codegen/js/journal-mysql.ts");
    const RUNNER: &str = include_str!("toolchain/journal-contract.mjs");
    let literal = |source: &str, declaration: &str, whose: &str| -> String {
        source
            .split_once(&format!("const {declaration} = \""))
            .unwrap_or_else(|| panic!("{whose} declares `{declaration}`"))
            .1
            .split('"')
            .next()
            .unwrap_or_else(|| panic!("{whose} declares `{declaration}` as a string literal"))
            .to_string()
    };
    assert_eq!(
        literal(MODULE, "WRITER_GUARD", "`src/codegen/js/journal.ts`"),
        literal(
            RUNNER,
            "WRITER_GUARD",
            "`tests/toolchain/journal-contract.mjs`"
        ),
        "the journal takes its MySQL guard under one name and the conformance runner looks for \
         another, so the dead-owner case finds nothing holding a lock and reports itself \
         unavailable on a server that is working perfectly (PRD resolved q62)"
    );
    assert_eq!(
        literal(ARM, "MYSQL_GUARD_NAME", "`src/codegen/js/journal-mysql.ts`"),
        literal(
            RUNNER,
            "MYSQL_GUARD_NAME",
            "`tests/toolchain/journal-contract.mjs`"
        ),
        "the journal qualifies its MySQL guard's name with one expression and the conformance \
         runner asks `IS_USED_LOCK` for another, so the dead-owner case finds nothing holding a \
         lock and reports itself unavailable on a server that is working perfectly \
         (`docs/durability.md` §2.3)"
    );
}

/// The version `docs/durability.md` heads with, read off the document.
fn journal_version() -> u64 {
    let document = fs::read_to_string(repository().join("docs/durability.md"))
        .expect("`docs/durability.md` is readable");
    document
        .lines()
        .find_map(|line| line.strip_prefix("**Journal version:** "))
        .and_then(|rest| rest.trim().parse().ok())
        .expect("`docs/durability.md` heads with its journal version")
}

/// The cases every backend must answer `true`.
///
/// One list, because there is one contract. Each names the promise it is about,
/// so a failure reads as the sentence of `docs/durability.md` it broke rather
/// than as an index into a runner.
const TRUE_EVERYWHERE: &[&str] = &[
    // §10, §11.2 — the schema is created on first open, and a second open —
    // a released journal re-opened, so the DDL really runs again against tables
    // that already exist — does nothing rather than failing.
    "open_is_idempotent",
    // …and one process gets one handle. `openJournal` memoizes its promise,
    // which is what keeps a `serve` recovering several executions at once from
    // opening several connections — on a remote binding the second would meet
    // its own writer guard and refuse (§2.3).
    "open_memoizes_one_handle",
    // §3.5 — a lifecycle row is written once and the second `begin` of one id is
    // a no-op, which is what a recovered execution re-entering `openExecution`
    // relies on.
    "begin_is_idempotent",
    "lifecycle_round_trips_unicode",
    // §3, §11.1 — a payload round-trips, in both the shapes a server can
    // quietly truncate or re-encode.
    "payload_round_trips_unicode",
    "payload_round_trips_a_large_blob",
    "a_record_reads_back_its_request",
    "append_is_idempotent",
    // …and the columns that are not payloads, which is where the sizes really
    // diverge: MySQL's `TEXT` is 64 KiB and its shipped `STRICT_TRANS_TABLES`
    // makes an over-long value an error rather than a truncation, so a failure
    // message or an attempt's detail this size is a write two backends take and
    // one could refuse — leaving a lifecycle row `open` for ever (§3.6, §10).
    "a_large_error_round_trips",
    "a_large_delivery_detail_round_trips",
    // §4 — two keys differing only in case are two effects.
    "keys_are_case_sensitive",
    // §5 — the frontier is the first key the journal does not hold.
    "frontier_is_the_first_key_not_held",
    // §6.1 — the recovery scan, in both directions.
    "recovery_scan_finds_the_open_execution",
    "recovery_scan_drops_a_closed_execution",
    "a_closed_row_keeps_its_outcome",
    // §7 — the mark that tells resolved q29's second divergence from a mismatch
    // the composition already had.
    "a_refusal_is_recorded",
    // §3.7 — the delivery ledger's ordinals and its one-outcome-per-row rule.
    "delivery_ordinals_increase",
    "a_settled_delivery_is_not_reopened",
    "a_pending_delivery_is_owed",
    // §3.8 — the dispatch board.
    "park_is_idempotent",
    "a_claim_hands_the_row_over",
    "a_second_claim_takes_nothing",
    "a_settle_answers_once",
    "a_settled_dispatch_keeps_its_outcome",
];

/// **Every backend the grammar spells is driven, or says why it is not.**
///
/// The sibling above quantifies over [`BACKENDS`], which is only the whole story
/// if that list is grammar §14.7's. A fourth provider added to the compiler and
/// not added here would be a backend this suite reports nothing about while
/// reading as though it covered them all.
#[test]
fn the_suite_drives_every_provider_the_grammar_spells() {
    let spelled: Vec<&str> = compose_core::ast::deploy::JournalProvider::ALL
        .iter()
        .map(|provider| provider.as_str())
        .collect();
    let driven: Vec<&str> = BACKENDS.iter().map(|backend| backend.provider).collect();
    assert_eq!(
        driven, spelled,
        "`journal:` binds a provider this suite does not drive, so `docs/durability.md`'s \
         contract is unchecked on one of the backends a deploy file can name \
         (grammar §14.7, PRD resolved q62)"
    );

    // …and each remote one names a variable of its own, which is what the skip
    // above prints and what `.github/workflows/ci.yml` sets.
    let workflow = fs::read_to_string(repository().join(".github/workflows/ci.yml"))
        .expect("the workflow is readable");
    for backend in BACKENDS {
        let opens_a_file = compose_core::ast::deploy::JournalProvider::ALL
            .iter()
            .find(|provider| provider.as_str() == backend.provider)
            .expect("every driven provider is one the grammar spells")
            .opens_in_process();
        assert_eq!(
            backend.variable.is_none(),
            opens_a_file,
            "`{}` opens in process: {opens_a_file}, and names a server variable: {}. A \
             provider that dials out needs one and a provider that opens a file has none",
            backend.provider,
            backend.variable.is_some()
        );
        let Some(variable) = backend.variable else {
            continue;
        };
        assert!(
            workflow.contains(variable),
            "`.github/workflows/ci.yml` sets no `{variable}`, so CI skips the `{}` journal \
             while reporting the suite green — which is exactly what the service containers \
             were ratified to prevent (PRD resolved q62)",
            backend.provider
        );
    }
}
