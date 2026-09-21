//! `src/journal.ts`: the record that makes an execution survive its process.
//!
//! [`super::runtime`] emits what a node *does*; this module emits the log of
//! what it did. PRD resolved q26 settles the mechanism — "a journal + replay of
//! the record this runtime already keeps, not a LangGraph checkpointer" — and
//! resolved q27 settles where it lives: a deploy-target slot, exactly as
//! `storage_backends` are, with `--target local` binding a SQLite file beside
//! the project. Every target this compiler release can build is process-local,
//! so SQLite is what every project gets and the composition says nothing about
//! it (`docs/durability.md` §10).
//!
//! It is **assembled** the way [`super::harness`] is, and for that module's
//! reason: an invariant half that is byte-identical in every project this
//! compiler release builds, plus the arm the target's `journal:` bound
//! (grammar §14.7, PRD resolved q62). A project whose target says nothing, or
//! says `provider: sqlite`, gets the invariant half alone — the same bytes every
//! project has always had — and neither the network driver nor the code that
//! would import one. That is "emit only the drivers a composition uses", read
//! one construct along from the harness SDKs.
//!
//! Every half is edited as TypeScript in the compiler's own tree
//! (`src/codegen/js/journal.ts`, `journal-postgres.ts`, `journal-mysql.ts`)
//! rather than as a Rust string literal.
//!
//! # Why it is a module of its own
//!
//! Two reasons, and the second is the load-bearing one.
//!
//! It is the **leaf** of the emitted import graph. `src/stores.ts` imports
//! `src/runtime.ts` and `src/runtime.ts` imports this file, so a journal that
//! reached back for either would close a cycle; putting the project's data
//! directory here — re-exported by `src/stores.ts`, which is where an ejected
//! reader learned the name — is what keeps the graph acyclic.
//!
//! And the journal is not the trace. `docs/trace.md` §11 keeps model
//! completions, tool results and human answers *out* of the trace, and replay
//! needs exactly those; a reader who has to be told that two artifacts with one
//! keying discipline have different sensitivity is better served by two files
//! than by one file with a rule in the middle of it.

use crate::ast::deploy::JournalProvider;
use crate::ir::Ir;
use crate::ir::deploy::journal_of;

/// The invariant half: the record, the keys, the statements, and the SQLite arm.
const SOURCE: &str = include_str!("js/journal.ts");

/// The Postgres arm, emitted where the target binds one.
const POSTGRES: &str = include_str!("js/journal-postgres.ts");

/// …and the MySQL arm.
const MYSQL: &str = include_str!("js/journal-mysql.ts");

/// The driver each remote provider is reached through, pinned exactly.
///
/// The discipline [`super::project::PINS`] and [`super::harness::HARNESS_PINS`]
/// are under, for their reason: what a journal does is what a compiled graph
/// survives, so a release that let a driver float would change that with no
/// commit saying so (PRD §9.18, §5.12).
///
/// `@types/pg` is in the list because `pg` ships no types of its own and this
/// project type-checks under `strict`; it is a **development** pin, which
/// [`development_pins_of`] is what separates out. `mysql2` ships its own, so its
/// row is one entry long.
pub const JOURNAL_PINS: &[(JournalProvider, &[(&str, &str)])] = &[
    (JournalProvider::Postgres, &[("pg", "8.23.0")]),
    (JournalProvider::Mysql, &[("mysql2", "3.24.4")]),
];

/// The development dependencies one remote provider brings, pinned.
///
/// Only Postgres has any, and only because `pg` publishes no type declarations:
/// a `tsc --noEmit` over a project that imports it would fail on the import
/// rather than on anything this compiler emitted.
pub const JOURNAL_DEV_PINS: &[(JournalProvider, &[(&str, &str)])] =
    &[(JournalProvider::Postgres, &[("@types/pg", "8.23.1")])];

/// The packages one provider's arm brings, pinned — empty for `sqlite`, whose
/// driver `./stores.ts` already pins for every project.
#[must_use]
pub fn pins_of(provider: JournalProvider) -> &'static [(&'static str, &'static str)] {
    JOURNAL_PINS
        .iter()
        .find(|(held, _)| *held == provider)
        .map_or(&[], |(_, pins)| *pins)
}

/// …and the development ones.
#[must_use]
pub fn development_pins_of(provider: JournalProvider) -> &'static [(&'static str, &'static str)] {
    JOURNAL_DEV_PINS
        .iter()
        .find(|(held, _)| *held == provider)
        .map_or(&[], |(_, pins)| *pins)
}

/// `src/journal.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    // The arm the target bound, appended. It assigns itself into `BACKENDS`,
    // which is what makes the dispatch in the invariant half above a lookup
    // rather than a `switch` naming providers this project has no driver for.
    match journal_of(ir) {
        JournalProvider::Sqlite => {}
        JournalProvider::Postgres => {
            contents.push('\n');
            contents.push_str(POSTGRES);
        }
        JournalProvider::Mysql => {
            contents.push('\n');
            contents.push_str(MYSQL);
        }
    }
    super::GeneratedFile {
        path: "src/journal.ts".to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn the_module_is_emitted_verbatim_under_the_header() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.starts_with("// This file was generated by agent-compose"));
        assert!(emitted.ends_with(SOURCE), "the journal is emitted verbatim");
    }

    /// Every composition gets the same journal *under one binding*, like the
    /// runtime beside it.
    ///
    /// The qualifier is grammar §14.7's and is the whole of what resolved q62
    /// changed here: what a project's journal module holds is a function of the
    /// **target's** `journal:` and of nothing else, so two compositions built
    /// for one target are byte-identical and the composition still says nothing.
    #[test]
    fn every_composition_gets_the_same_journal() {
        let empty = module(&ir_of("version: \"0.1\"\n")).contents;
        let full = module(&ir_of(crate::codegen::test_support::EVERY_FORM)).contents;
        assert_eq!(empty.replace("main.yml", ""), full.replace("main.yml", ""));
    }

    /// **The arm the target bound, and no other** (grammar §14.7, PRD resolved
    /// q62).
    ///
    /// Three claims in one, and the third is the one a reader of the ruling
    /// should be able to check: a target that binds the default gets the bytes
    /// every project has always had — no `pg`, no `mysql2`, and no code that
    /// would import either — which is what makes "the zero-infra local build
    /// keeps its dependency set unchanged" a property of the artifact rather
    /// than of a branch nobody takes.
    #[test]
    fn a_project_carries_the_journal_arm_its_target_bound_and_no_other() {
        for (provider, present, absent) in [
            (JournalProvider::Sqlite, None, vec![POSTGRES, MYSQL]),
            (JournalProvider::Postgres, Some(POSTGRES), vec![MYSQL]),
            (JournalProvider::Mysql, Some(MYSQL), vec![POSTGRES]),
        ] {
            let emitted = module(&journal_ir(provider)).contents;
            assert!(
                emitted.contains(SOURCE),
                "`{}` lost the invariant half of the journal",
                provider.as_str()
            );
            if let Some(arm) = present {
                assert!(
                    emitted.contains(arm),
                    "a target binding `{}` gets no arm for it, so `openJournal` would refuse \
                     its own build",
                    provider.as_str()
                );
            }
            for other in absent {
                assert!(
                    !emitted.contains(other),
                    "a target binding `{}` carries an arm it never dispatches to, and the \
                     driver that arm imports is one its `package.json` does not pin",
                    provider.as_str()
                );
            }
            // …and the driver goes exactly where the arm does.
            let manifest = crate::codegen::project::package_json(&journal_ir(provider)).contents;
            for (package, _) in pins_of(provider)
                .iter()
                .chain(development_pins_of(provider))
            {
                assert!(
                    manifest.contains(&format!("\"{package}\"")),
                    "a target binding `{}` does not pin `{package}`",
                    provider.as_str()
                );
            }
            for (other, pins) in JOURNAL_PINS.iter().chain(JOURNAL_DEV_PINS) {
                if *other == provider {
                    continue;
                }
                for (package, _) in *pins {
                    assert!(
                        !manifest.contains(&format!("\"{package}\"")),
                        "a target binding `{}` pins `{package}`, which only a `{}` journal \
                         reaches",
                        provider.as_str(),
                        other.as_str()
                    );
                }
            }
        }
    }

    /// **The statements are one implementation, not three** (PRD resolved q62).
    ///
    /// The ruling puts all three backends "behind the one journal interface",
    /// and the way this module keeps that true is structural: every statement
    /// lives in the invariant half, and an arm supplies a connection, a schema
    /// and a writer guard. An arm that grew a statement of its own would be a
    /// second reading of `docs/durability.md` that nothing compares against the
    /// first — and the conformance suite would be proving the divergence rather
    /// than the servers.
    ///
    /// The schemas are the exception, and are excluded by name: a DDL is how a
    /// backend spells the *shape* the statements need, which really is
    /// per-backend — `BIGSERIAL` against `AUTO_INCREMENT`, `COLLATE "C"` against
    /// `ascii_bin`.
    #[test]
    fn every_statement_the_journal_runs_is_in_its_invariant_half() {
        for (arm, name, schema) in [
            (POSTGRES, "journal-postgres.ts", "POSTGRES_SCHEMA"),
            (MYSQL, "journal-mysql.ts", "MYSQL_SCHEMA"),
        ] {
            let (_, beyond) = arm
                .split_once(schema)
                .expect("each arm declares its schema by name");
            // The clause MySQL spells its no-op upsert with is a *fragment*
            // `SqlJournal` splices into its own insert, so the verb inside it is
            // not a statement of this arm's: it is taken out before the scan.
            let beyond = beyond.replace("ON DUPLICATE KEY UPDATE ", "");
            for verb in ["INSERT INTO", "UPDATE ", "DELETE FROM", "SELECT * FROM"] {
                assert!(
                    !beyond.contains(verb),
                    "`{name}` runs a `{verb}` of its own: every statement of the journal is \
                     `SqlJournal`'s, so that one reading of `docs/durability.md` is right for \
                     all three backends (PRD resolved q62)"
                );
            }
        }
    }

    /// **The one column whose name is a keyword is quoted, and MySQL is told to
    /// read that quote as one.**
    ///
    /// `key` is a reserved word in MySQL and a plain identifier in SQLite and
    /// Postgres, so the shared statements spell it `"key"` — which SQLite and
    /// Postgres read as an identifier natively and MySQL reads as a *string*
    /// unless its session says otherwise. `ANSI_QUOTES` is what says otherwise,
    /// and it is load-bearing in the way a reader cannot see from either file
    /// alone: without it, every statement naming that column silently compares
    /// against the three-letter string `key` instead of a column, and the
    /// journal answers nothing for every effect it holds.
    ///
    /// So the dependency is written down in one place, here, in both
    /// directions: the statements quote it, and the arm that needs the setting
    /// sets it.
    #[test]
    fn the_reserved_column_is_quoted_and_mysql_is_told_to_read_the_quote() {
        let source = include_str!("js/journal.ts");
        assert!(
            source.contains("\"key\""),
            "the shared statements no longer quote `key`, which is a reserved word on one of \
             the three backends (grammar §14.7, PRD resolved q62)"
        );
        for statement in source
            .lines()
            .map(str::trim)
            .filter(|line| line.contains("FROM effects") || line.contains("INTO effects"))
        {
            assert!(
                !statement.contains(" key ") && !statement.contains("(key"),
                "a statement names `key` unquoted, which MySQL reads as a reserved word: \
                 {statement}"
            );
        }
        // …and the setting is read off the **statement** `openMysql` sends, not
        // off the file: the arm's own doc comment names `ANSI_QUOTES` too, so a
        // `MYSQL.contains` here stayed green with the functional
        // `SET SESSION sql_mode` replaced by a no-op — a false guarantee in the
        // one test that claims to hold the dependency a reader cannot see from
        // either file alone.
        let opening = function_code(MYSQL, "openMysql");
        assert!(
            opening.contains("SET SESSION sql_mode") && opening.contains("'ANSI_QUOTES'"),
            "`openMysql` no longer sets `ANSI_QUOTES` on its session, so every shared statement \
             naming `\"key\"` compares against a three-letter string rather than against the \
             column, and the journal answers nothing for every effect it holds"
        );
        assert!(
            !POSTGRES.contains("ANSI_QUOTES"),
            "the Postgres arm sets a MySQL session variable"
        );
    }

    /// **Each arm takes a writer guard, and says what a second opener is told**
    /// (`docs/durability.md` §2, PRD resolved q42, q62).
    ///
    /// Defence in depth under the one-writer rule, and the reason the two
    /// primitives are the right ones is a property a reader cannot see from the
    /// call: both are **session-scoped**, so the server drops them when the
    /// connection ends and a hub that was killed mid-write leaves nothing
    /// holding the journal it has to be resumed from. SQLite's file lock is the
    /// opposite — it outlives its owner, which is why that arm has to break one
    /// — and a guard that acquired *blocking* would be a third behaviour again:
    /// a `resume` typed beside a live `serve` would hang rather than be told.
    #[test]
    fn each_remote_arm_takes_a_guard_that_does_not_outlive_its_connection() {
        for (arm, name, acquire, blocking) in [
            (
                POSTGRES,
                "journal-postgres.ts",
                "pg_try_advisory_lock($1, $2)",
                "pg_advisory_lock(",
            ),
            (
                MYSQL,
                "journal-mysql.ts",
                "GET_LOCK(${MYSQL_GUARD_NAME}, 0)",
                "GET_LOCK(${MYSQL_GUARD_NAME}, -1)",
            ),
        ] {
            assert!(
                arm.contains(acquire),
                "`{name}` takes no writer guard, so two processes would write one journal \
                 (`docs/durability.md` §2)"
            );
            assert!(
                arm.contains("guardHeld()"),
                "`{name}` takes a guard and says nothing when it is held: a second opener is \
                 refused by name rather than left to interleave"
            );
            assert!(
                !arm.contains("RELEASE_LOCK") && !arm.contains("pg_advisory_unlock"),
                "`{name}` releases its guard by statement, which is a release a crash skips: \
                 the point of a session-scoped lock is that the server drops it when the \
                 connection ends"
            );
            assert!(
                !arm.contains(blocking),
                "`{name}` acquires its guard blocking (`{blocking}`), so a `resume` typed \
                 beside a live `serve` hangs instead of being told what is happening"
            );
        }
        // …and the refusal says the two things a reader has to know: who may
        // hold it, and when it goes. "When it goes" is bounded rather than
        // instant — a host that vanished holds it until its server notices —
        // and the refusal states the bound rather than the comfortable half of
        // it, or a reader whose takeover was refused goes hunting for a `serve`
        // that does not exist.
        let refusal = function_body(include_str!("js/journal.ts"), "guardHeld");
        for named in [
            "released when that session ends",
            "one process at a time",
            "${GUARD_REAP_SECONDS}s",
        ] {
            assert!(
                refusal.contains(named),
                "the guard refusal does not say `{named}`, so a reader meets a lock with no \
                 account of who holds it or when it goes"
            );
        }
    }

    /// How the MySQL arm spells the name it takes its guard under.
    ///
    /// Read by the two tests below, and spelled a third time by
    /// `tests/toolchain/journal-contract.mjs` — which
    /// `the_runner_and_the_module_take_one_writer_guard` holds to this one,
    /// because a runner asking `IS_USED_LOCK` for another name finds nothing
    /// holding the lock and reports the dead-owner case unavailable against a
    /// server that is working perfectly.
    const MYSQL_GUARD_NAME: &str = "CONCAT(?, ':', LEFT(SHA2(DATABASE(), 256), 32))";

    /// **One database, one journal, one writer — on MySQL too**
    /// (`docs/durability.md` §2.3, PRD resolved q62).
    ///
    /// The guard's name is a constant because the *tables* are shared by
    /// everything pointed at one database, which is the reasoning
    /// [`WRITER_GUARD`]'s doc comment gives and which Postgres makes true for
    /// free: `pg_try_advisory_lock` is scoped to the database the session
    /// connected to, so `staging` and `prod` in two databases of one cluster
    /// take two locks under one name.
    ///
    /// **MySQL's user-level locks are server-wide.** The name alone is the key,
    /// with no schema component anywhere in it — so the same two deployments on
    /// one managed MySQL server would take the *same* lock, and the second
    /// `serve` to start would be refused by [`guardHeld`] with advice that
    /// cannot come true: the message says a vanished host's guard is released
    /// within `GUARD_REAP_SECONDS`, and this one is held by a healthy,
    /// unrelated deployment that will hold it for as long as it is up. No
    /// conformance case can see it either — one suite run drives one database on
    /// one server — so the asymmetry is bound here, where the two arms sit side
    /// by side.
    #[test]
    fn the_mysql_guard_is_scoped_to_the_schema_the_postgres_one_gets_for_free() {
        assert!(
            MYSQL.contains(&format!("const MYSQL_GUARD_NAME = \"{MYSQL_GUARD_NAME}\";")),
            "the MySQL arm no longer qualifies its writer guard with `DATABASE()`, so a lock \
             name MySQL keys server-wide is shared by every deployment on the server: two \
             journals in two databases of one instance lock each other out, and the refusal \
             tells the second operator to wait out a host loss that never happened \
             (`docs/durability.md` §2.3)"
        );
        assert!(
            function_code(MYSQL, "openMysql").contains("GET_LOCK(${MYSQL_GUARD_NAME}, 0)"),
            "the MySQL arm declares a qualified guard name and takes its lock under something \
             else"
        );
        // …and the Postgres arm does not reach for one: the key it locks on is
        // `WRITER_GUARD_KEYS`, and the database it locked in is the one it
        // connected to.
        assert!(
            !POSTGRES.contains("DATABASE()") && !POSTGRES.contains("current_database()"),
            "the Postgres arm qualifies its advisory lock by database, which the server already \
             does — two deployments in two databases of one cluster hold two locks under one \
             key"
        );
    }

    /// **A guard nobody is holding is given back in minutes, not in hours**
    /// (`docs/durability.md` §2.3, PRD resolved q62).
    ///
    /// The sibling above binds the *primitive*; this binds the property the
    /// primitive only has once each arm asks for it. A session-scoped lock is
    /// dropped when the **session** ends, and a session ends when the server
    /// notices its peer is gone — which is immediate when a process is killed on
    /// a machine that is still running, and is the server's own default when the
    /// machine itself is lost. Those defaults are two hours (Linux's
    /// `tcp_keepalive_time`, which Postgres leaves alone) and eight
    /// (`wait_timeout`). A crash, a power loss or a partition is exactly the case
    /// "a `serve` restarted on a fresh machine recovers every open execution"
    /// exists for, so an unshortened window makes the headline property of a
    /// remote journal unavailable for most of a working day — with the refusal
    /// text telling the operator to go and look for a live `serve` that is not
    /// there.
    ///
    /// So each arm shortens the window for its own session and keeps a heartbeat
    /// on it, and both halves are read here: a shortened window with no heartbeat
    /// reaps a `serve` that is merely idle, which is worse than the bug.
    ///
    /// **Every assertion below reads the statement rather than the identifier**,
    /// and that is the difference between this test and the one it replaced. A
    /// setting is only a bound once it is *sent*: `arm.contains("…idle")` is
    /// satisfied by the constant's own declaration and by the paragraph above it,
    /// so deleting the `query(POSTGRES_KEEPALIVES)` line left the suite green
    /// with the window back at Linux's two hours and nothing else in the project
    /// observing it — the live conformance case ends the guard session by
    /// *killing* the backend, so no test waits out a reap.
    #[test]
    fn each_remote_arm_bounds_how_long_a_lost_host_holds_the_guard() {
        assert!(
            SOURCE.contains("const GUARD_REAP_SECONDS = 300;")
                && SOURCE.contains("const GUARD_HEARTBEAT_MS = 30_000;"),
            "the window and the heartbeat are one pair of numbers for both arms, stated in the \
             invariant half beside the guard they are about (`docs/durability.md` §2.3)"
        );
        for (arm, name, open, setting, shortens) in [
            (
                POSTGRES,
                "journal-postgres.ts",
                "openPostgres",
                "POSTGRES_KEEPALIVES",
                "tcp_keepalives_idle",
            ),
            (
                MYSQL,
                "journal-mysql.ts",
                "openMysql",
                "MYSQL_WAIT_TIMEOUT",
                "SET SESSION wait_timeout",
            ),
        ] {
            let declared = declaration(arm, setting);
            assert!(
                declared.contains(shortens),
                "`{name}`'s `{setting}` no longer says `{shortens}`, so a host that vanished \
                 holds this journal's writer guard for hours and the takeover `serve` is \
                 refused for all of them (`docs/durability.md` §2.3)"
            );
            assert!(
                code(arm).contains("GUARD_REAP_SECONDS"),
                "`{name}` shortens the window to a number of its own, so the two backends \
                 promise different bounds while §2.3 states one"
            );
            // …and it is **applied**, in the function that takes the guard the
            // window is about. A setting declared and never sent is the
            // server's default with a constant beside it.
            assert!(
                function_code(arm, open).contains(&format!("query({setting})")),
                "`{name}` declares `{setting}` and never sends it from `{open}`, so the server \
                 reaps a silent session on its own schedule — two hours, or eight — while \
                 `docs/durability.md` §2.3's table and `guardHeld`'s \
                 `${{GUARD_REAP_SECONDS}}s` both promise five minutes \
                 (`docs/durability.md` §2.3)"
            );
            // …and the heartbeat that keeps the shortened window off a hub which
            // is merely idle is a timer with that period, not a keepalive option
            // that happens to name the constant.
            assert!(
                code(arm).contains("}, GUARD_HEARTBEAT_MS);"),
                "`{name}` shortens the window and never says it is alive on a timer of that \
                 period, so a `serve` that journals nothing for an afternoon is reaped \
                 mid-deployment"
            );
            assert!(
                code(arm).contains("clearInterval(this.#heartbeat);"),
                "`{name}` heartbeats and never stops, so a finished `run` does not exit"
            );
        }
    }

    /// **Neither arm lets a disconnect take the process with it**
    /// (`docs/durability.md` §2.3).
    ///
    /// Both drivers are `EventEmitter`s that emit `error` on a connection the far
    /// end closed with no command in flight, and an `EventEmitter` that emits
    /// `error` with nothing listening raises `ERR_UNHANDLED_ERROR` — which for a
    /// `serve` is every in-flight execution's in-process state, lost because a
    /// server closed an idle socket. Nothing in the emitted app installs an
    /// `uncaughtException` handler, and nothing should: this is the listener that
    /// belongs beside the connection.
    ///
    /// The listener **records** rather than redials, and that is the one-writer
    /// rule at the moment it matters most: the guard went with the connection, so
    /// another hub may already hold the journal, and a redial would put two
    /// writers into one record.
    #[test]
    fn a_lost_connection_is_refused_rather_than_taking_the_process_or_redialling() {
        assert!(
            SOURCE.contains("function connectionLost("),
            "the invariant half no longer says what a statement over a lost connection is \
             refused with, so each arm would answer it its own way"
        );
        for (arm, name) in [
            (POSTGRES, "journal-postgres.ts"),
            (MYSQL, "journal-mysql.ts"),
        ] {
            assert!(
                arm.contains(".on(\"error\", (reported: unknown) =>"),
                "`{name}` opens a connection and never listens for its `error` event, so a \
                 server-side disconnect — a MySQL `wait_timeout`, a restarted Postgres — ends \
                 the whole process with `ERR_UNHANDLED_ERROR` (`docs/durability.md` §2.3)"
            );
            assert!(
                arm.contains("fault.error ??= connectionLost(reported)"),
                "`{name}` hears the fault and does not keep it, so the statements after it go \
                 to a connection that is gone"
            );
            assert!(
                arm.contains("if (this.#fault.error !== undefined) throw this.#fault.error;"),
                "`{name}`'s driver runs statements without asking whether the connection it \
                 holds is still there"
            );
        }
        // …and each arm dials **once**, in the function that takes the guard.
        // A second dial anywhere is a redial by another name.
        for (arm, name, open, dial) in [
            (POSTGRES, "journal-postgres.ts", "openPostgres", "connect()"),
            (MYSQL, "journal-mysql.ts", "openMysql", "createConnection({"),
        ] {
            assert_eq!(
                arm.matches(dial).count(),
                1,
                "`{name}` opens a connection in more than one place, so something other than \
                 `{open}` can dial: the writer guard went with the connection it lost, and a \
                 redial would take it back from a hub that may already hold this journal \
                 (`docs/durability.md` §2)"
            );
            assert!(
                function_body(arm, open).contains(dial),
                "`{name}` dials somewhere other than `{open}`, which is the one place that \
                 takes the writer guard"
            );
        }
    }

    /// **The guard is taken before the schema is created** (§2.3).
    ///
    /// Two processes opening one fresh remote journal at the same time — a
    /// `serve` restart overlapping the one it replaces, an `agent-compose resume`
    /// typed beside a live `serve` — would otherwise both run the DDL. Postgres
    /// documents `CREATE TABLE IF NOT EXISTS` as *not* atomic against a
    /// concurrent creator, so one of the two can fail on a duplicate key in
    /// `pg_type`: a catalog error on open rather than the refusal by name this
    /// design promises. Under the guard there is one creator by construction, and
    /// the opener about to be refused runs no DDL against an operator's database
    /// at all.
    #[test]
    fn a_remote_arm_takes_the_guard_before_it_creates_anything() {
        for (arm, name, open, acquire, schema) in [
            (
                POSTGRES,
                "journal-postgres.ts",
                "openPostgres",
                "pg_try_advisory_lock",
                "POSTGRES_SCHEMA",
            ),
            (
                MYSQL,
                "journal-mysql.ts",
                "openMysql",
                "GET_LOCK(${MYSQL_GUARD_NAME}, 0)",
                "MYSQL_SCHEMA",
            ),
        ] {
            let body = function_body(arm, open);
            let taken = body
                .find(acquire)
                .unwrap_or_else(|| panic!("`{name}` takes a writer guard in `{open}`"));
            let created = body
                .find(schema)
                .unwrap_or_else(|| panic!("`{name}` creates its schema in `{open}`"));
            assert!(
                taken < created,
                "`{name}` runs its DDL before it takes the guard, so a second opener that is \
                 about to be refused still creates tables in an operator's database — and two \
                 concurrent creators is a race Postgres does not make atomic \
                 (`docs/durability.md` §2.3)"
            );
        }
    }

    /// **Every backend's schema holds every column the statements name.**
    ///
    /// The statements are shared (see the sibling above), so a table one arm
    /// declares a column short of is a backend on which the very first `INSERT`
    /// fails — at run time, on a server, in a deployment. That is the one class
    /// of divergence a shared implementation makes *more* likely rather than
    /// less, and it is the one a type-check cannot see: SQL is a string.
    ///
    /// So the SQLite schema is read as the inventory — it is the arm that has
    /// always been right, and `docs/durability.md` §3 is written against it —
    /// and each remote schema is held to naming the same columns of the same
    /// tables. Types are deliberately **not** compared: `TEXT` against
    /// `LONGTEXT` against `VARCHAR(512) … ascii_bin` is exactly what a
    /// per-backend schema is for, and §10 states why each is what it is.
    ///
    /// The conformance suite is what proves the columns then *behave*
    /// (`journal_backend_conformance`); this is what fails in a second rather
    /// than only where a server is reachable.
    #[test]
    fn every_backend_declares_every_column_the_statements_name() {
        let expected = columns(SOURCE);
        assert_eq!(
            expected.keys().collect::<Vec<_>>(),
            vec!["deliveries", "dispatches", "effects", "executions"],
            "the SQLite schema is the inventory every other backend is held to, and it no \
             longer holds the four tables `docs/durability.md` §3 describes"
        );
        for (arm, name) in [
            (POSTGRES, "journal-postgres.ts"),
            (MYSQL, "journal-mysql.ts"),
        ] {
            let held = columns(arm);
            for (table, wanted) in &expected {
                let found = held.get(table).unwrap_or_else(|| {
                    panic!(
                        "`{name}` declares no `{table}` table, so every statement of \
                         `SqlJournal` that names one fails on that backend"
                    )
                });
                for column in wanted {
                    assert!(
                        found.contains(column),
                        "`{name}`'s `{table}` declares no `{column}`, so the shared statement \
                         that names it fails on the first row (`docs/durability.md` §10)"
                    );
                }
            }
            // …and `seq`, which only a backend without SQLite's implicit `rowid`
            // needs, and which park order is derived from
            // (`docs/distributed.md` §6.2).
            assert!(
                held.get("dispatches")
                    .is_some_and(|found| found.contains(&"seq".to_string())),
                "`{name}`'s `dispatches` declares no `seq`: a backend with no implicit rowid \
                 has to carry insertion order as a column, or a fan-out's five rows sharing \
                 one `parked_at` drain in an order nothing defines"
            );
        }
    }

    /// The columns each `CREATE TABLE` in a schema declares, by table.
    ///
    /// A reading of the DDL rather than of a list written beside it: a list is
    /// only as current as the last person to extend it, and the failure this
    /// guards is precisely somebody adding a column to one schema and not the
    /// others.
    fn columns(source: &str) -> std::collections::BTreeMap<String, Vec<String>> {
        let mut found: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut table: Option<String> = None;
        for line in source.lines() {
            // A schema is a template literal, and MySQL's is an array of them —
            // so a `CREATE TABLE` can open a line behind the backtick that opens
            // its literal.
            let trimmed = line.trim().trim_start_matches(['`', '"']);
            if let Some(rest) = trimmed.strip_prefix("CREATE TABLE IF NOT EXISTS ") {
                table = rest
                    .split_whitespace()
                    .next()
                    .map(|name| name.trim_end_matches('(').to_string());
                if let Some(name) = &table {
                    found.entry(name.clone()).or_default();
                }
                continue;
            }
            let Some(name) = table.clone() else { continue };
            if trimmed.starts_with(')') {
                table = None;
                continue;
            }
            // A column line opens with the column's own name; a constraint line
            // opens with the keyword that names the constraint.
            let Some(word) = trimmed.split_whitespace().next() else {
                continue;
            };
            let column = word.trim_matches(|held| held == '"' || held == '`');
            if column.is_empty()
                || matches!(
                    column.to_ascii_uppercase().as_str(),
                    "PRIMARY" | "UNIQUE" | "KEY" | "INDEX" | "CONSTRAINT" | "FOREIGN" | "--"
                )
            {
                continue;
            }
            found.entry(name).or_default().push(column.to_string());
        }
        found
    }

    /// An artifact for one journal binding, and nothing else different.
    fn journal_ir(provider: JournalProvider) -> Ir {
        if provider == JournalProvider::DEFAULT {
            return ir_of("version: \"0.1\"\n");
        }
        crate::codegen::test_support::ir_of_mesh(
            "version: \"0.1\"\n",
            &format!(
                "version: \"0.1\"\njournal:\n  provider: {}\n  url: ${{JOURNAL_URL}}\n",
                provider.as_str()
            ),
        )
    }

    /// The driver is the one PRD §9.18 admits, and the two it refuses are named
    /// nowhere — the same bind `codegen::stores` puts on the store backends.
    ///
    /// The spec that opened this work asked for `bun:sqlite` on the grounds
    /// that generated artifacts run under Bun. They also run under Node, which
    /// PRD §9.18 makes a supported fallback with a static gate over the whole
    /// golden corpus behind it, and `node-sqlite3-wasm` is already pinned for
    /// the stores — so the same driver costs no new dependency *and* keeps the
    /// promise. This is the assertion that keeps the choice from drifting back.
    #[test]
    fn the_journal_names_no_runtime_specific_sqlite() {
        assert!(SOURCE.contains("await import(\"node-sqlite3-wasm\")"));
        assert!(
            !SOURCE.contains("\"bun:sqlite\"") && !SOURCE.contains("\"node:sqlite\""),
            "a generated module may not reach for an API one supported runtime lacks (PRD §9.18)"
        );
    }

    /// The version a resumed execution is held to is the one the document pins.
    #[test]
    fn the_journal_version_is_the_one_the_document_pins() {
        assert!(SOURCE.contains("export const JOURNAL_VERSION = 1;"));
        let document = include_str!("../../../../docs/durability.md");
        assert!(
            document.contains("**Journal version:** 1"),
            "`docs/durability.md` heads with the version this module emits"
        );
    }

    /// One effect is one statement, which is what makes a crash mid-write
    /// leave a row absent rather than half present (`docs/durability.md` §2).
    #[test]
    fn an_effect_is_appended_by_one_statement() {
        assert!(SOURCE.contains("INSERT INTO effects"));
        assert!(
            !SOURCE.contains("BEGIN IMMEDIATE"),
            "an append that opened a transaction of its own would have a window a crash could land in"
        );
    }

    /// **Every effect site is journaled, and there are no others.**
    ///
    /// `docs/durability.md` §3 makes completeness the invariant — "an effect
    /// that is not journaled is one a replay re-executes" — and tells a reader
    /// to verify it by finding the call sites. This is that reading, held
    /// mechanically, in both directions.
    ///
    /// The **first** direction is the one a new effect kind breaks: a surface
    /// added to `src/runtime.ts` that calls the world and does not reach the
    /// journal is a replay that re-issues it, and no other test in this
    /// repository would notice — the run would succeed, twice.
    ///
    /// The **second** is the one a documentation drift breaks: a site that is
    /// journaled and is not in the table above. Both are counted rather than
    /// merely searched for, because a search satisfied by any occurrence would
    /// be satisfied by the one this comment mentions.
    #[test]
    fn every_effect_site_reaches_the_journal_and_the_document_names_them_all() {
        let runtime = include_str!("js/runtime.ts");
        let stores = include_str!("js/stores.ts");
        let document = include_str!("../../../../docs/durability.md");

        // Each site, the module it lives in, and how it reaches the journal —
        // `journaled(…)` for a call whose whole answer is the record, and
        // `claim(…)` for the two whose record is assembled first (a model
        // call's ladder, a wait's settlement).
        for (site, source, module) in [
            ("callModel", runtime, "src/runtime.ts"),
            ("runExec", runtime, "src/runtime.ts"),
            ("runBuiltin", runtime, "src/runtime.ts"),
            ("runHttp", runtime, "src/runtime.ts"),
            ("callFunction", runtime, "src/runtime.ts"),
            ("callModule", runtime, "src/runtime.ts"),
            ("runHuman", runtime, "src/runtime.ts"),
            ("runCoder", runtime, "src/runtime.ts"),
            ("runStoreOp", stores, "src/stores.ts"),
        ] {
            let body = function_body(source, site);
            assert!(
                body.contains("journaled(") || body.contains(".claim("),
                "`{site}` performs an effect and does not reach the journal, so a replay                  would issue it a second time (`docs/durability.md` §3)"
            );
            assert!(
                document.contains(&format!("`{site}`")) && document.contains(module),
                "`docs/durability.md` §3's inventory does not name `{site}` in `{module}`"
            );
        }

        // …and no tenth. The count is over both emitted modules, because the
        // document's table is.
        let reached = runtime.matches("journaled(").count()
            + runtime.matches(".claim(").count()
            + stores.matches("journaled(").count()
            + stores.matches(".claim(").count();
        assert_eq!(
            reached, 9,
            "`docs/durability.md` §3 says there are exactly nine effect sites and this build              has {reached}: a new one belongs in that table, and a lost one is a replay that              re-issues an effect"
        );
    }

    /// **§4's key vocabulary is the one the journal writes.**
    ///
    /// The inventory above binds §3's *sites*. A key carries a **kind**, which
    /// §4 enumerates separately — and separately is how a fifth kind reached
    /// §3's table while §4 went on naming four. That drift has a reader: a key
    /// is `<site>#<kind>/<ordinal>`, and anything parsing one against §4's
    /// stated vocabulary treats every key of the unnamed kind as malformed.
    /// So the list is read off the type the runtime writes, in both directions
    /// — every member is named, and nothing else is.
    #[test]
    fn the_documented_key_vocabulary_is_the_one_the_journal_writes() {
        let document = include_str!("../../../../docs/durability.md");

        let union = SOURCE
            .split_once("export type EffectKind =")
            .expect("`src/journal.ts` declares the kinds a key can carry")
            .1;
        let union = &union[..union.find(';').expect("…as one union type")];
        let kinds: Vec<&str> = union
            .split('|')
            .map(|member| member.trim().trim_matches('"'))
            .filter(|member| !member.is_empty())
            .collect();

        let bullet = document
            .split_once("* `<kind>` is one of")
            .expect("`docs/durability.md` §4 enumerates the kinds a key can carry")
            .1;
        let bullet = &bullet[..bullet
            .find("\n* `<ordinal>`")
            .expect("…in the bullet before the ordinal's")];
        let named: Vec<&str> = bullet.split('`').skip(1).step_by(2).collect();

        assert_eq!(
            named, kinds,
            "`docs/durability.md` §4's key vocabulary and `EffectKind` have parted company: a \
             kind the runtime writes into a key and §4 does not name is a key a reader holding \
             this document to its word calls malformed"
        );
    }

    /// **A delivery is journaled before it is attempted**
    /// (`docs/durability.md` §3.7, PRD resolved q35).
    ///
    /// The inventory above is about the nine sites a *replay* consumes, and a
    /// callback delivery is deliberately none of them: nothing in the graph
    /// dispatches it, it is addressed by an execution and an ordinal rather
    /// than by an instance path, and no replay ever reads it back. So it sits
    /// in a ledger of its own — and `attemptDelivery`, the one declaration in
    /// the emitted app that calls the world for one, is exempted from the
    /// primitive walk above.
    ///
    /// An exemption with nothing else holding it is how a journaled delivery
    /// decays into a bare `fetch`. This is what holds it, in the direction the
    /// failure runs: the **intent** is recorded before anything is sent, and
    /// each attempt's outcome after it — which is what makes a delivery
    /// at-least-once across a restart rather than at-most-once inside one
    /// process. Read off the seams rather than off the file, because the order
    /// is what is being asserted and a file-wide search would find both calls
    /// wherever they were.
    #[test]
    fn a_delivery_is_journaled_before_it_is_attempted() {
        let serve = include_str!("js/serve.ts");
        let delivery = include_str!("js/delivery.ts");
        let document = include_str!("../../../../docs/durability.md");

        let opening = function_body(serve, "opening");
        let intent = opening
            .find("intendDelivery(")
            .expect("a delivery records its intent");
        let sending = opening
            .find("workDelivery(")
            .expect("a delivery is then worked on its schedule");
        assert!(
            intent < sending,
            "the intent of a delivery is recorded **before** the first attempt, or a process \
             that dies mid-attempt leaves nothing for a later start to finish \
             (`docs/durability.md` §3.7)"
        );
        assert!(
            opening.contains("refuseDelivery("),
            "a callback URL the allowlist admits nowhere is a recorded refusal rather than a \
             silent drop (grammar 13.3, PRD resolved q33)"
        );

        let attempts = function_body(delivery, "workDelivery");
        let attempted = attempts
            .find("attemptDelivery(")
            .expect("the schedule makes attempts");
        let recorded = attempts
            .find("journaling(")
            .expect("…and records what each one did");
        assert!(
            attempted < recorded,
            "an attempt's outcome is recorded after the attempt, which is the only order that \
             can hold one"
        );
        assert!(
            function_body(delivery, "journaling").contains("recordDeliveryAttempt("),
            "the seam `workDelivery` hands its outcomes to no longer reaches the journal, so an \
             attempt is made and recorded nowhere (`docs/durability.md` §3.7)"
        );

        // The **trace sink's** intent, on the same ledger and in the same order:
        // recorded where the lifecycle row closes, and never attempted from
        // there (grammar 14.5, PRD resolved q50).
        let ship = function_body(delivery, "shipTrace");
        assert!(
            ship.contains("intendDelivery("),
            "a trace export is journaled like every other delivery, or a sink outage is a trace \
             nothing will ever ship"
        );
        assert!(
            !ship.contains("fetch("),
            "`shipTrace` sends the trace itself, so the hook that closes a lifecycle row waits \
             on a collector"
        );

        // …and the one place a delivery leaves the process is the declaration
        // the walk above exempts by name, rather than wherever a later edit put
        // a second `fetch`.
        let sites: Vec<String> = declarations(serve)
            .into_iter()
            .chain(declarations(delivery))
            .filter(|(_, body)| body.contains("fetch("))
            .map(|(name, _)| name)
            .collect();
        assert_eq!(
            sites,
            ["attemptDelivery"],
            "the emitted app calls the world in exactly one place, and it is the one \
             `NOT_AN_EFFECT` names"
        );

        for named in ["`attemptDelivery`", "`src/delivery.ts`", "delivery"] {
            assert!(
                document.contains(named),
                "`docs/durability.md` §3.7 does not name {named}, so the ledger this test binds \
                 is documented nowhere"
            );
        }
    }

    /// **One loop per delivery, and one outcome per row**
    /// (`docs/durability.md` §3.7).
    ///
    /// The sibling above binds the *order* of a delivery's two journal writes.
    /// This binds who may make them, which is the half a restart puts under
    /// pressure: one start reaches [`attempts`] from two directions. `recover`
    /// walks the open executions without waiting for the replays it starts
    /// (§6.1), so an execution that re-parks journals a `parked` intent and sets
    /// its schedule going while that walk is still going on — and the walk over
    /// every `pending` row that follows it then reads the row written a moment
    /// ago.
    ///
    /// Two loops over one row would POST it twice under one id, which a receiver
    /// dedupes, and would each append attempts to one row, which nothing
    /// dedupes: the journal would hold a delivery that made more attempts than
    /// the schedule §3.7 bounds, and a loop writing `pending` after the other
    /// wrote `delivered` would leave the next start owing a webhook already
    /// taken. So the claim is read off the app — a row is claimed before it is
    /// attempted and released when the loop ends — and the guard under it off
    /// the journal: **every** statement that moves a delivery row carries the
    /// predicate that it is still `pending`, so a late writer meets a row with
    /// an outcome and changes nothing.
    #[test]
    fn one_delivery_is_worked_once_and_a_row_that_ended_is_not_reopened() {
        let serve = include_str!("js/serve.ts");
        let delivery = include_str!("js/delivery.ts");
        let journal = include_str!("js/journal.ts");

        let attempts = function_body(delivery, "workDelivery");
        let claimed = attempts
            .find("working.add(")
            .expect("a delivery is claimed by the loop that works it");
        let attempted = attempts
            .find("attemptDelivery(")
            .expect("…which is the loop that sends it");
        assert!(
            claimed < attempted,
            "a delivery is claimed **before** it is attempted, or the second loop over one row \
             is already sending it by the time the first says so (`docs/durability.md` §3.7)"
        );
        assert!(
            attempts.contains("working.delete("),
            "a claim that is never released is a delivery this process would not pick up again"
        );
        assert!(
            function_body(serve, "resumeDelivery").contains("beingWorked("),
            "the start's walk over every `pending` row is the second direction one delivery is \
             reached from, so it is the one that has to ask whether the row is already being \
             worked"
        );

        let moved: Vec<&str> = journal
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("\"UPDATE deliveries SET"))
            .collect();
        assert_eq!(
            moved.len(),
            3,
            "the three ways a delivery row moves are a refusal, an exhaustion and an attempt: \
             {moved:?}"
        );
        let mut refusals = 0;
        for statement in moved {
            assert!(
                statement.contains("AND status = 'pending'"),
                "a delivery has one outcome, so a row that already reached one is left as it is: \
                 {statement}"
            );
            // …and the refusal carries one predicate more. `refused` is a
            // **callback** row's outcome: a trace sink's address is the
            // operator's and no list admits it, so there is nothing for one to
            // fail to match (grammar 14.5, PRD resolved q50). The type of
            // `refuseDelivery` says that for a row being opened; this is what
            // says it for a row already on the ledger, which is addressed by
            // execution and ordinal and carries no type to hold.
            if statement.contains("status = 'refused'") {
                refusals += 1;
                assert!(
                    statement.contains("AND (kind IS NULL OR kind = 'callback')"),
                    "a statement that refuses a delivery would refuse a trace export too: \
                     {statement}"
                );
            }
        }
        assert_eq!(
            refusals, 1,
            "the ledger refuses a delivery in one statement, and this checked {refusals}"
        );
    }

    /// **A journal write that fails loses neither the event nor the row's end**
    /// (`docs/durability.md` §3.7).
    ///
    /// The two siblings above bind the *order* of a delivery's journal writes
    /// and *who* may make them. This binds the third thing, which is what
    /// happens when one of them does not land — a second process holding the
    /// file past the lock wait, a disk momentarily full, both states §2 says a
    /// healthy deployment reaches. Neither of the two failures it prevents is
    /// visible from outside the process, and both are silent for the life of a
    /// journal:
    ///
    ///  * a **parking announced to nobody**. `parking` marks a quiescence's
    ///    pauses as reported before the intent is journaled, because the guard
    ///    it marks them for is synchronous. Marked and then not journaled, every
    ///    later quiescence of that execution finds the set already reported —
    ///    and a receiver that subscribed to the question is told about the
    ///    settle and never about the question, which under `respond: async` is
    ///    the whole of what it was subscribed for.
    ///  * a **row left `pending` past its schedule**. An attempt whose outcome
    ///    the journal would not take leaves the row under-counting its attempts,
    ///    and `attempts` on the row is the one thing a later start reads to
    ///    decide how much of the schedule is left: the status route reports the
    ///    execution as owing a webhook for ever, and the next start resumes the
    ///    delivery below the offset it really reached and POSTs past the bound
    ///    §3.7 calls normative.
    ///  * a **settle announced to nobody**. `closed` journals its intent in the
    ///    last moment the lifecycle row is open, and the row closes as it
    ///    returns: after that `recover` walks no `open` execution for it and the
    ///    walk over `pending` rows finds none, so an intent the journal refused
    ///    once is a settle nothing will ever send — to a caller holding a `202`
    ///    who, by resolved q34's reasoning, is not polling either.
    ///
    /// So all three are read off the seams: the marks come back off where the
    /// intent did not go down, an attempt the journal refused is **carried**
    /// rather than dropped — kept for the write that does land — and the two
    /// writes nothing would ever come back to are insisted on where they stand.
    #[test]
    fn a_write_the_journal_refuses_leaves_neither_a_lost_event_nor_a_row_that_never_ends() {
        let serve = include_str!("js/serve.ts");
        let delivery = include_str!("js/delivery.ts");

        let parking = function_body(serve, "parking");
        assert!(
            parking.contains("execution.reported.add(") && parking.contains("announcing("),
            "a parking marks its pauses and hands the journaling to the seam that can put the \
             marks back, or a failed intent is a question announced to nobody"
        );
        assert!(
            !parking.contains("deliver("),
            "`parking` journals its intent directly again, so nothing observes whether the \
             journal took it and the marks it left stand for a row that does not exist"
        );

        let announcing = function_body(serve, "announcing");
        let asked = announcing
            .find("deliver(")
            .expect("`announcing` is what journals a parking's intent");
        let unmarked = announcing
            .find("execution.reported.delete(")
            .expect("…and what puts the pauses back where the journal would not take it");
        assert!(
            asked < unmarked,
            "the marks come off **after** the journal has refused the intent, not before it is \
             offered one"
        );
        assert!(
            function_body(serve, "deliver").contains("return false;"),
            "`deliver` no longer answers whether the journal took the intent, so `announcing` \
             cannot tell a parking that was recorded from one that was lost"
        );

        let attempts = function_body(delivery, "workDelivery");
        let held = attempts
            .find("owed.push(")
            .expect("an attempt's outcome is held before it is written");
        let written = attempts
            .find("journaling(")
            .expect("…and then offered to the journal");
        assert!(
            held < written,
            "an attempt is recorded in this process before it is offered to the journal, which \
             is what lets the next write carry what this one could not put down"
        );
        assert!(
            attempts.contains("insisting("),
            "the write that ends a delivery is the one nothing comes back to, so it is insisted \
             on rather than tried once: a row left `pending` past its schedule is neither of \
             §3.7's two ends"
        );

        let journaling = function_body(delivery, "journaling");
        let refused = journaling
            .find("return false;")
            .expect("`journaling` says when the journal would not take an attempt");
        let dropped = journaling
            .find("owed.shift()")
            .expect("…and drops an attempt only once it is down");
        assert!(
            refused < dropped,
            "an attempt the journal refused is dropped anyway, so the row under-counts its \
             attempts and the next start POSTs past the bound §3.7 states"
        );
        let closed = function_body(serve, "closed");
        let insisted = closed
            .find("insisting(")
            .expect("the settle's intent is insisted on, not tried once");
        assert!(
            closed[insisted..].contains("deliver(execution, \"settled\""),
            "`closed` journals the settle's intent with a write nothing observes: the lifecycle \
             row closes as it returns, so a journal that says no once loses the settle for the \
             life of the journal (`docs/durability.md` §3.7)"
        );
        assert!(
            closed[insisted..].contains("shipping(execution"),
            "…and the trace export beside it, which the same closing row makes unrecoverable: a \
             sink intent written after the row closed is one no start would ever find \
             (grammar 14.5, PRD resolved q50)"
        );

        assert!(
            function_body(delivery, "insisting").contains("JOURNAL_RETRY_MS["),
            "the ladder a refused write is retried on is unbounded, which is a queue rather \
             than the courtesy §3.7 calls a webhook"
        );
    }

    /// **Every column added to a table after it existed has a probe.**
    ///
    /// `CREATE TABLE IF NOT EXISTS` leaves a table that exists exactly as it is,
    /// so a column the schema grew is a column an older file does not have — and
    /// an `INSERT` naming it fails every new execution in that file.
    /// `docs/durability.md` §11.2 makes a physical schema change compatible only
    /// where it still reads older files, and the `PRAGMA table_info` probe is
    /// what makes it one. This is the list of them, so a fourth column added
    /// without a probe fails here rather than in somebody's journal.
    #[test]
    fn every_column_added_after_its_table_is_migrated_into_an_older_file() {
        let journal = include_str!("js/journal.ts");
        let migrated = function_body(journal, "migrated");
        for (table, column, kind) in [
            ("executions", "callback", "TEXT"),
            ("executions", "traceparent", "TEXT"),
            ("effects", "refused", "INTEGER NOT NULL DEFAULT 0"),
            ("deliveries", "trigger_kind", "TEXT"),
            ("deliveries", "kind", "TEXT"),
        ] {
            assert!(
                migrated.contains(&format!("column[\"name\"] === \"{column}\"")),
                "`{table}.{column}` is in the schema and nothing probes for it, so a journal \
                 written before it would refuse every write that names it \
                 (`docs/durability.md` §11.2)"
            );
            assert!(
                migrated.contains(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind};")),
                "`{table}.{column}` is probed for and not added, or added under another type"
            );
        }
    }

    /// The journaled seams the walk below starts from, by the name each is
    /// declared under.
    ///
    /// A subset of the nine sites the inventory above enumerates, and the
    /// subset is what this test needs rather than a second opinion about what
    /// a seam is: a site earns a place here when something it reaches calls
    /// the world. `callModule` hands a composition's own file the effects it
    /// makes, which are that file's rather than this runtime's, and so reaches
    /// no primitive of these modules. `runCoder` earned one at PRD resolved
    /// q61: a `workspace: fresh` run has the runtime make its directory, which
    /// is a call on the world inside the coder adapter for the first time —
    /// the harness SDKs having always made theirs behind the driver seam, in a
    /// module this test does not read.
    const SEAMS: [&str; 8] = [
        "callModel",
        "runExec",
        "runBuiltin",
        "runHttp",
        "callFunction",
        "runHuman",
        "runCoder",
        "runStoreOp",
    ];

    /// **Nothing calls the world except through one of the seven.**
    ///
    /// The sibling above reads the inventory from the top down: the seven
    /// functions `docs/durability.md` §3 names reach the journal, and no
    /// eighth does. That direction cannot see the failure its own docstring
    /// calls load-bearing — a *new* surface, `runGrpc` say, that spawns a
    /// process or opens a socket and never reaches the journal at all. It is
    /// not one of the seven, so no per-site assertion applies to it; it contains
    /// no `journaled(` and no `.claim(`, so the count still answers seven; the
    /// table is untouched, so the documentation assertions pass. `cargo test`
    /// is green and every replay past that node issues its call again.
    ///
    /// So this reads it from the bottom up instead, off the **primitives**
    /// rather than off the seams: every call to the world in **every** emitted
    /// constant module has to sit somewhere the seven can reach, or be named below
    /// as something that is not an effect the graph issues. `runGrpc` fails here
    /// on the first line of its body.
    ///
    /// **What counts as a primitive is derived rather than listed**, which is
    /// the difference between a rule and a spot check. A hand-written list of
    /// spellings is only as complete as the last person to extend it: `spawn(`
    /// does not match `spawnSync(`, `fs.writeFileSync` does not match
    /// `fs.appendFileSync` or `fs.promises.writeFile`, and a surface written
    /// with any call the list happens not to hold passes both directions of the
    /// inventory while replaying into a second side effect. So the primitives
    /// are read off each module's **imports**: a module specifier is either one
    /// whose surface is the world ([`WORLD_MODULES`]) — in which case *every*
    /// binding it introduces is a primitive, whatever member of it is called —
    /// or one that reaches nothing outside the process ([`INERT_MODULES`]), and
    /// a specifier in neither list fails this test until somebody says which it
    /// is. What arrives with no import to derive it from is [`UNIMPORTED`].
    ///
    /// Reachability is transitive because the seams are thin: `runExec` calls
    /// `runExecLive`, which calls `spawn`; `runStoreOp` calls `perform`, which
    /// calls `open`, `transact`, `tabular`, `blobOp` and — through
    /// `runtime.callEmbeddings` — `send`, which is where a `vector` store's
    /// network call lives. Following the calls is what keeps the rule honest
    /// without pinning the shape of the code beneath each seam.
    ///
    /// **Every module and every declaration form**, because a blind spot in
    /// either is a place an unjournaled call can sit while this test answers
    /// green. `src/serve.ts` really holds one — the completion webhook — and a
    /// reading that took only `src/runtime.ts` and `src/stores.ts` could not see
    /// it; a reading that took only top-level `function` declarations could not
    /// see a `class HttpPool { async send() { await fetch(…) } }` or a
    /// `const runGrpc = async () => …` in the two modules it did read. So the
    /// file is **partitioned** by its column-zero declarations, whatever their
    /// keyword, and every primitive is attributed to the declaration it sits
    /// under.
    /// Module specifiers whose surface **is** the world.
    ///
    /// Every binding one of these introduces is a primitive: the whole of
    /// `node:fs` rather than the six calls of it this repository happens to
    /// make today, the whole of `node:child_process` rather than `spawn` alone.
    /// A module listed here is a decision that anything reached through it
    /// leaves the process.
    const WORLD_MODULES: [&str; 15] = [
        "node:fs",
        "node:fs/promises",
        "node:child_process",
        "node:net",
        "node:tls",
        "node:dgram",
        "node:dns",
        "node:http",
        "node:https",
        "node:http2",
        "node:cluster",
        "node:worker_threads",
        "node-sqlite3-wasm",
        // The two journal drivers, each reached only from the arm a target that
        // binds it gets (grammar §14.7, PRD resolved q62). They are the world by
        // the plainest reading of this list: a socket to a database is a socket.
        "pg",
        "mysql2/promise",
    ];

    /// Module specifiers that reach nothing outside the process, each with the
    /// reason it does not.
    ///
    ///  * `node:path`, `node:url`, `node:crypto` — string and byte arithmetic.
    ///  * `node:process` — `env`, `argv`, `exit` and the two standard streams.
    ///    Writing a diagnostic to `stderr` is not an effect a composition
    ///    issues, and no replay is "of" one.
    ///  * `@langchain/langgraph` — the graph engine the emitted nodes run
    ///    under. It calls the world only through the node bodies this compiler
    ///    emits, which are what the rest of this test reads.
    ///  * `fastify` — the app `serve` mounts. Its socket belongs to the
    ///    *command*, like `writeTrace`'s file: a recovered execution is replayed
    ///    into a server that is already listening, so nothing here is ever
    ///    reached a second time by a replay.
    const INERT_MODULES: [&str; 6] = [
        "node:path",
        "node:process",
        "node:url",
        "node:crypto",
        "@langchain/langgraph",
        "fastify",
    ];

    /// What calls the world with no import to derive it from.
    ///
    /// Two families. The **platform globals** — `fetch` and the transports a
    /// future surface would most plausibly reach for — are in the runtime rather
    /// than in a module, so no specifier introduces them. And the three journal
    /// **handles**: a constructor is a call an import's binding catches, but the
    /// handle a driver then runs every statement through is a *member* of a
    /// private field, so `#database.run(…)`, `#client.query(…)` and
    /// `#connection.query(…)` are watched as member prefixes rather than as
    /// calls. Without them the one place each arm really reaches its server
    /// would be the one place this walk could not see (grammar §14.7, PRD
    /// resolved q62).
    const UNIMPORTED: [&str; 11] = [
        "fetch(",
        "WebSocket(",
        "EventSource(",
        "XMLHttpRequest(",
        "navigator.sendBeacon(",
        "Bun.",
        "Deno.",
        "Database(",
        "database.",
        "#client.",
        "#connection.",
    ];

    #[test]
    fn nothing_in_the_emitted_runtime_calls_the_world_except_under_a_journaled_seam() {
        // What calls the world and is **not** an effect the graph issues, so is
        // not one a replay may perform twice. Each is named with its module,
        // because the same name in another module would be a different decision:
        //
        //  * `releaseExecution` — grammar 11.1's `scope: execution` lifetime,
        //    run by `runFlow` when a run ends. It removes what the run owned;
        //    running it twice removes it twice.
        //  * `releaseWorkspaces` — the same lifetime for the directory a
        //    built-in tool worked in where its binding wrote no `workspace:`
        //    (grammar 6.1, PRD resolved q54). Run by `runFlow` beside
        //    `releaseExecution`, and for its reason: it removes what the run
        //    owned, and a run whose journal row stays open keeps it.
        //  * `attemptDelivery` — one attempt at a lifecycle delivery: an `http`
        //    trigger's `callback:` webhook, or the settled trace a
        //    `trace_sink:` ships (grammar 13.3, 14.5, `docs/durability.md`
        //    §3.7). It is a journaled effect, and it is **not** one of the
        //    seven: a delivery is not something the graph dispatches, is not
        //    addressed by an instance path, and is never consumed by a replay —
        //    it is the *lifecycle* being reported, so it has a ledger of its own
        //    beside `effects`. What keeps a replay from making it twice is that
        //    ledger's own at-least-once discipline, which
        //    `a_delivery_is_journaled_before_it_is_attempted` reads off the
        //    same seam rather than leaving it to this exemption.
        //  * `writeTrace` — the run's own trace document, written by the command
        //    after the run (`docs/trace.md`). A resumed generation writes a fresh
        //    whole one, which is §9's promise rather than a repeat.
        //  * the journal's **own** storage: the three drivers, the open paths
        //    that create and migrate each backend's schema, the lock a killed
        //    writer leaves behind (§2), and `journalExists`. These are the record
        //    itself. A replay that "re-executed" them would be a replay reading
        //    its own journal, which is what a replay *is*. The two remote arms
        //    are read here like every other emitted module, so a driver call
        //    that ever escaped one of them fails this test rather than nothing.
        const NOT_AN_EFFECT: [(&str, &str); 13] = [
            ("src/stores.ts", "releaseExecution"),
            ("src/runtime.ts", "releaseWorkspaces"),
            ("src/delivery.ts", "attemptDelivery"),
            ("src/cli.ts", "writeTrace"),
            ("src/journal.ts", "SqliteDriver"),
            ("src/journal.ts", "openSqlite"),
            ("src/journal.ts", "breakStaleLock"),
            ("src/journal.ts", "migrated"),
            ("src/journal.ts", "journalExists"),
            ("src/journal.ts", "PostgresDriver"),
            ("src/journal.ts", "openPostgres"),
            ("src/journal.ts", "MysqlDriver"),
            ("src/journal.ts", "openMysql"),
        ];

        let modules = [
            ("src/runtime.ts", include_str!("js/runtime.ts")),
            ("src/stores.ts", include_str!("js/stores.ts")),
            ("src/journal.ts", include_str!("js/journal.ts")),
            // The two arms, under the path they are appended to: a project
            // whose target binds one really does carry it inside
            // `src/journal.ts` (grammar §14.7, PRD resolved q62).
            ("src/journal.ts", POSTGRES),
            ("src/journal.ts", MYSQL),
            ("src/serve.ts", include_str!("js/serve.ts")),
            ("src/delivery.ts", include_str!("js/delivery.ts")),
            ("src/otlp.ts", include_str!("js/otlp.ts")),
            ("src/cli.ts", include_str!("js/cli.ts")),
            ("src/cel.ts", include_str!("js/cel.ts")),
        ];
        // The primitives, derived. Each one *is* the effect — the moment a
        // process leaves itself — so a replay that reaches one has re-executed
        // something whatever the code around it is called.
        let mut primitives: BTreeSet<String> = UNIMPORTED.into_iter().map(str::to_string).collect();
        for (module, source) in modules {
            for statement in imports(source) {
                for specifier in specifiers(&statement) {
                    // Another emitted module, read in its own right by the loop
                    // this one is inside.
                    if specifier.starts_with('.') {
                        continue;
                    }
                    if INERT_MODULES.contains(&specifier.as_str()) {
                        continue;
                    }
                    assert!(
                        WORLD_MODULES.contains(&specifier.as_str()),
                        "`{module}` imports `{specifier}`, which this test cannot classify. Say \
                         which it is: `WORLD_MODULES` if anything reached through it leaves the \
                         process — every binding it introduces then has to sit under one of \
                         {SEAMS:?} — or `INERT_MODULES`, with the reason it reaches nothing \
                         outside this process."
                    );
                    // A type-only import introduces no value, so it calls
                    // nothing; the specifier is still classified above, because
                    // a module that arrives as a type today is one somebody
                    // imports for its functions tomorrow.
                    if statement.starts_with("import type ") {
                        continue;
                    }
                    primitives.extend(bindings(&statement));
                }
            }
        }

        let declared: Vec<(&str, String, String)> = modules
            .iter()
            .flat_map(|(module, source)| {
                declarations(source)
                    .into_iter()
                    .map(move |(name, body)| (*module, name, body))
            })
            .collect();

        for seam in SEAMS {
            assert!(
                declared.iter().any(|(_, name, _)| name == seam),
                "`{seam}` is declared in none of the emitted modules, so this test is reading \
                 an inventory that has moved"
            );
        }

        // Everything the seven reach, followed until it stops growing.
        let mut reached: BTreeSet<&str> = SEAMS.into_iter().collect();
        loop {
            let mut grew = false;
            for (_, name, body) in &declared {
                if !reached.contains(name.as_str()) {
                    continue;
                }
                for (_, called, _) in &declared {
                    if reached.contains(called.as_str()) || !calls(body, called) {
                        continue;
                    }
                    reached.insert(called.as_str());
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }

        let mut exempted: BTreeSet<(&str, &str)> = BTreeSet::new();
        for (module, name, body) in &declared {
            for primitive in &primitives {
                if !body.contains(primitive.as_str()) {
                    continue;
                }
                if NOT_AN_EFFECT.contains(&(module, name.as_str())) {
                    exempted.insert((module, name.as_str()));
                    continue;
                }
                assert!(
                    reached.contains(name.as_str()),
                    "`{name}` in `{module}` calls `{primitive}` and nothing journaled reaches \
                     it, so a replay past it issues that call a second time \
                     (`docs/durability.md` §3). Either route it through one of {SEAMS:?}, or — \
                     if it is not an effect the graph issues — say so in this test's \
                     `NOT_AN_EFFECT`."
                );
            }
        }
        assert_eq!(
            exempted,
            NOT_AN_EFFECT
                .into_iter()
                .collect::<BTreeSet<(&str, &str)>>(),
            "an exemption nothing uses is one nobody is checking: drop it"
        );
    }

    /// Every `import` of an emitted module, each flattened onto one line.
    ///
    /// Flattened because an import list is formatted across lines as soon as it
    /// is long enough, and a reading that took lines would see the specifier and
    /// the names it introduces as different statements. The **dynamic** ones are
    /// here too, as bare `import("…")` fragments: the SQLite driver arrives that
    /// way, and a module loaded at a call site is as much a module this project
    /// imports as one loaded at the top.
    fn imports(source: &str) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        let mut held: Option<String> = None;
        for line in source.lines() {
            if let Some(at) = line.find("import(") {
                found.push(line[at..].trim().to_string());
            }
            if held.is_none() && !line.starts_with("import ") {
                continue;
            }
            let mut statement = held.take().unwrap_or_default();
            if !statement.is_empty() {
                statement.push(' ');
            }
            statement.push_str(line.trim());
            if statement.contains("from \"") {
                found.push(statement);
            } else {
                held = Some(statement);
            }
        }
        found
    }

    /// The module specifiers one import statement names.
    fn specifiers(statement: &str) -> Vec<String> {
        let mut found = Vec::new();
        let mut rest = statement;
        for opener in ["from \"", "import(\""] {
            while let Some(at) = rest.find(opener) {
                let after = &rest[at + opener.len()..];
                let Some(end) = after.find('"') else { break };
                found.push(after[..end].to_string());
                rest = &after[end..];
            }
            rest = statement;
        }
        found
    }

    /// The primitives one import of a [`WORLD_MODULES`] specifier introduces.
    ///
    /// A **named** import is a call under its local name — `spawn` from
    /// `node:child_process` is `spawn(`, and `{ spawn as launch }` is `launch(`.
    /// A default or namespace import is the whole surface under its own name, so
    /// it is the member prefix `fs.` rather than any list of calls: that is what
    /// makes `fs.appendFileSync` and `fs.promises.writeFile` primitives without
    /// anybody having thought of them.
    fn bindings(statement: &str) -> Vec<String> {
        let body = statement
            .strip_prefix("import ")
            .unwrap_or(statement)
            .split(" from \"")
            .next()
            .unwrap_or_default()
            .trim();
        let (before, named) = match (body.find('{'), body.find('}')) {
            (Some(open), Some(close)) if open < close => (&body[..open], &body[open + 1..close]),
            _ => (body, ""),
        };
        let mut found: Vec<String> = named
            .split(',')
            .filter_map(|entry| entry.rsplit(" as ").next())
            .map(str::trim)
            .filter(|name| !name.is_empty() && *name != "type")
            .map(|name| format!("{name}("))
            .collect();
        // Whatever is left of the braces: `fs`, `* as fs`, or `fs,`.
        let whole = before
            .trim()
            .trim_end_matches(',')
            .trim()
            .trim_start_matches('*')
            .trim()
            .trim_start_matches("as ")
            .trim();
        if !whole.is_empty() {
            found.push(format!("{whole}."));
        }
        found
    }

    /// Whether `body` calls `name`, as a call rather than as a longer word.
    ///
    /// `runtime.` is looked through, because `src/stores.ts` reaches
    /// `src/runtime.ts` through a namespace import and `runtime.callEmbeddings`
    /// is one of the edges this walk depends on.
    fn calls(body: &str, name: &str) -> bool {
        let wanted = format!("{name}(");
        let mut rest = body;
        while let Some(at) = rest.find(&wanted) {
            let before = rest[..at].chars().next_back();
            let boundary = match before {
                None => true,
                Some('.') => rest[..at].ends_with("runtime."),
                Some(character) => {
                    !character.is_alphanumeric() && character != '_' && character != '$'
                }
            };
            if boundary {
                return true;
            }
            rest = &rest[at + wanted.len()..];
        }
        false
    }

    /// What the lines before an emitted module's first declaration are called.
    ///
    /// A primitive there is a module that calls the world **as it loads**, which
    /// no seam could reach and no exemption should quietly cover, so it is named
    /// rather than skipped.
    const MODULE_SCOPE: &str = "<module scope>";

    /// Every top-level declaration of an emitted module, by name and body.
    ///
    /// A **partition** rather than a brace match, and that is the point: each
    /// line at column zero that declares a name opens a region and the region
    /// runs to the next one, so a `class`, a `const … = async () =>` and a
    /// `function` are all read the same way and none of them can hide a call to
    /// the world by being the wrong shape. Matching a closing brace would have to
    /// know each form's, which is the reading that missed a class method.
    ///
    /// **Comment lines are dropped**, because both things this body is read for
    /// are about code: a primitive named in prose is not a call, and a function
    /// named in prose is not an edge in the reachability walk.
    fn declarations(source: &str) -> Vec<(String, String)> {
        let mut found: Vec<(String, String)> = Vec::new();
        let mut name = String::from(MODULE_SCOPE);
        let mut body = String::new();
        for line in source.lines() {
            if let Some(opened) = declared_name(line) {
                found.push((
                    std::mem::replace(&mut name, opened),
                    std::mem::take(&mut body),
                ));
            }
            if is_comment(line) {
                continue;
            }
            body.push_str(line);
            body.push('\n');
        }
        found.push((name, body));
        found
    }

    /// One top-level declaration's own lines, by name.
    ///
    /// [`declarations`] partitions a module at its column-zero declarations and
    /// drops the prose, so this is what a `const` really says rather than what
    /// the paragraph above it describes.
    fn declaration(source: &str, name: &str) -> String {
        declarations(source)
            .into_iter()
            .find(|(declared, _)| declared == name)
            .unwrap_or_else(|| panic!("the emitted module declares `{name}`"))
            .1
    }

    /// An emitted module's code, with its prose taken out.
    ///
    /// The difference between a rule that binds a statement and a rule its own
    /// doc comment satisfies. Two of the tests above were the latter — one was
    /// green with the `SET SESSION sql_mode` that makes every `"key"` a column
    /// replaced by a no-op, the other with the statement that shortens the
    /// guard's reap window deleted — because the identifier they matched on was
    /// still there, in the paragraph explaining why the deleted line mattered.
    fn code(source: &str) -> String {
        source
            .lines()
            .filter(|line| !is_comment(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// …and of one function of it, which is the pairing most of these rules
    /// want: a statement is sent from somewhere, and *where* is half the rule.
    fn function_code(source: &str, name: &str) -> String {
        code(&function_body(source, name))
    }

    /// Whether this line is comment or documentation rather than code.
    fn is_comment(line: &str) -> bool {
        let trimmed = line.trim_start();
        trimmed.starts_with("//") || trimmed.starts_with('*') || trimmed.starts_with("/*")
    }

    /// The name a column-zero declaration line opens, if it is one.
    ///
    /// Every keyword an emitted module declares something callable under, which
    /// is what makes the partition above cover a `class` and an arrow-function
    /// `const` as well as a `function`. A `type`, an `interface` and an `import`
    /// declare nothing that can call the world, so they open no region and their
    /// lines stay with the declaration above them.
    fn declared_name(line: &str) -> Option<String> {
        let rest = line.strip_prefix("export ").unwrap_or(line);
        let rest = rest.strip_prefix("async ").unwrap_or(rest);
        let rest = ["function ", "class ", "const ", "let ", "var "]
            .into_iter()
            .find_map(|keyword| rest.strip_prefix(keyword))?;
        let name: String = rest
            .chars()
            .take_while(|character| {
                character.is_alphanumeric() || *character == '_' || *character == '$'
            })
            .collect();
        if name.is_empty() {
            return None;
        }
        Some(name)
    }

    /// The body of one top-level function of an emitted module.
    ///
    /// The same reader `compose-core`'s `tests/trace_format_inventory.rs` uses,
    /// and for its reason: a rule about what one function does is only a rule if
    /// it is read off that function rather than off the file around it. Both
    /// emitted modules are formatted, so a top-level declaration opens at column
    /// zero and closes on a line that is exactly `}` — which is what makes a
    /// line scan enough, and a brace count wrong: a signature's own inline
    /// object type (`request: { … }`) opens a brace before the body does.
    ///
    /// The four spellings are read rather than the one, because whether a
    /// function is exported says nothing about what it does: the delivery seams
    /// are module-internal and are as much a rule as the exported ones. What
    /// follows the name is either `(` or `<` — a **generic** seam is still that
    /// seam, and `callModule` is one, so a reader that insisted on the
    /// parenthesis would report an effect site as missing rather than as
    /// unjournaled.
    fn function_body(source: &str, name: &str) -> String {
        let keywords = [
            "export async function ",
            "async function ",
            "export function ",
            "function ",
        ];
        let opens = |line: &str| {
            keywords.iter().any(|keyword| {
                line.strip_prefix(keyword)
                    .and_then(|rest| rest.strip_prefix(name))
                    .is_some_and(|rest| rest.starts_with('(') || rest.starts_with('<'))
            })
        };
        let mut lines = source.lines().skip_while(|line| !opens(line));
        let opened = lines
            .next()
            .unwrap_or_else(|| panic!("the emitted module declares `function {name}(…`"));
        let mut held = String::from(opened);
        for line in lines {
            held.push('\n');
            held.push_str(line);
            if line == "}" {
                return held;
            }
        }
        panic!("`{name}` has no closing brace in the first column")
    }
}
