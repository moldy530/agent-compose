//! `src/stores.ts`: the backends every `store.*` runs on.
//!
//! [`super::graph`] emits one `StoreBinding` per `store.*` — its kind, its
//! scope, whether it declares a `metadata_schema:`, its `embed:` connection and
//! the backend the active target resolved — and this module emits what those
//! bindings *do*: the SQLite tables `kv` and `vector` live in, the directory of
//! files a `blob` is, the scope partitions, the idempotency ledger, and the
//! synthesized-tool entry point of grammar 11.5.
//!
//! Its invariant half is a **constant**, for the reasons [`super::runtime`] is
//! one: a golden diff stays about the composition, and the file is real
//! TypeScript in the compiler's own tree (`src/codegen/js/stores.ts`) rather
//! than a Rust string literal.
//!
//! It is **assembled** the way [`super::journal`] is, and for that module's
//! reason (PRD resolved q63): the invariant half, plus one arm per dialled
//! backend this composition's stores bind — `postgres` and `mysql` for the `kv`
//! kind (grammar §14.3). A project whose stores are all local, which is every
//! project built for `--target local`, gets the invariant half alone: the same
//! bytes every project has always had, and neither the network driver nor the
//! code that would import one. That is "emit only the drivers a composition
//! uses", read one construct along from the harness SDKs and the journal.
//!
//! # Why a WebAssembly SQLite
//!
//! PRD 5.8's zero-infra guarantee is that `--target local` substitutes
//! "SQLite/local disk for every store unconditionally", and PRD §9.18 says
//! generated code carries no runtime-specific API — no `bun:` specifier, which
//! rules out `bun:sqlite`, and gate 14 of `tests/generated_code_gates.rs`
//! enforces it over the whole golden corpus. `node:sqlite` is the other obvious
//! candidate and is not portable either: Bun does not implement it (Bun 1.3
//! answers `Could not resolve: "node:sqlite"`), so an emitted project would run
//! on the fallback runtime and fail on the default one.
//!
//! `node-sqlite3-wasm` is what is left and what is pinned: one dependency, no
//! dependencies of its own, no native build and no install script — a
//! WebAssembly build of SQLite over `node:fs`, which both supported runtimes
//! resolve identically. The trade it makes is written down rather than
//! discovered: a WebAssembly SQLite is slower than a native one and has **no
//! cross-process locking**, so the local backends are single-process. That is
//! the same boundary PRD 5.10 already draws for `--target local` — one process —
//! and a deployment that really has two binds a dialled `kv` backend instead
//! (PRD resolved q63).

use std::collections::BTreeSet;

use crate::ast::deploy::BackendProvider;
use crate::ir::Ir;
use crate::ir::definition::DefinitionBody;

use super::drivers::RemoteDriver;

/// The invariant half: the local backends, the op catalogue, and the seam a
/// dialled arm answers through.
const SOURCE: &str = include_str!("js/stores.ts");

/// The Postgres `kv` arm, emitted where a store binds one.
const POSTGRES: &str = include_str!("js/stores-postgres.ts");

/// …and the MySQL one.
const MYSQL: &str = include_str!("js/stores-mysql.ts");

/// The arm one dialled provider is, or `None` for a backend opened in process.
#[must_use]
const fn arm_of(provider: BackendProvider) -> Option<&'static str> {
    match provider {
        BackendProvider::Postgres => Some(POSTGRES),
        BackendProvider::Mysql => Some(MYSQL),
        _ => None,
    }
}

/// The driver one dialled provider is reached through (PRD resolved q63).
///
/// The pins are [`super::drivers`]'s, shared with the journal arms that dial the
/// same two servers: a target whose journal and whose store both bind Postgres
/// declares `pg` **once**.
#[must_use]
pub const fn driver_of(provider: BackendProvider) -> Option<RemoteDriver> {
    match provider {
        BackendProvider::Postgres => Some(RemoteDriver::Pg),
        BackendProvider::Mysql => Some(RemoteDriver::Mysql2),
        _ => None,
    }
}

/// Every backend this composition's stores resolve to under the active target.
///
/// Sorted and deduplicated, because what reads it is an emitter: two stores on
/// one provider are one arm and one dependency entry.
#[must_use]
pub fn bound(ir: &Ir) -> BTreeSet<BackendProvider> {
    ir.definitions
        .values()
        .filter_map(|definition| match &definition.body {
            DefinitionBody::Store(store) => Some(crate::ir::deploy::backend_of(ir, store).provider),
            _ => None,
        })
        .collect()
}

/// The packages this composition's store bindings bring, pinned.
///
/// Empty for a project whose stores are all local — which is every project built
/// for `--target local`, since that target substitutes local storage for every
/// store unconditionally (PRD 5.8, Decision D87).
#[must_use]
pub fn pins(ir: &Ir) -> Vec<(&'static str, &'static str)> {
    bound(ir)
        .into_iter()
        .filter_map(driver_of)
        .flat_map(super::drivers::pins_of)
        .copied()
        .collect()
}

/// …and the development ones.
#[must_use]
pub fn development_pins(ir: &Ir) -> Vec<(&'static str, &'static str)> {
    bound(ir)
        .into_iter()
        .filter_map(driver_of)
        .flat_map(super::drivers::development_pins_of)
        .copied()
        .collect()
}

/// `src/stores.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    // The arms this composition's stores bind, appended. Each assigns itself
    // into `STORE_BACKENDS`, which is what makes the dispatch in the invariant
    // half a lookup rather than a `switch` naming providers this project has no
    // driver for — and what keeps a project whose stores are all local carrying
    // neither the code nor the dependency (PRD resolved q63).
    for provider in bound(ir) {
        if let Some(arm) = arm_of(provider) {
            contents.push('\n');
            contents.push_str(arm);
        }
    }
    super::GeneratedFile {
        path: "src/stores.ts".to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn the_module_is_emitted_verbatim_under_the_header() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.starts_with("// This file was generated by agent-compose"));
        assert!(emitted.contains("export async function runStoreOp("));
        assert!(
            emitted.ends_with(SOURCE),
            "the backends are emitted verbatim"
        );
    }

    /// The backends are the same bytes for every composition **under one set of
    /// bindings**, like the journal beside them.
    ///
    /// The qualifier is grammar §14.3's and is the whole of what resolved q63
    /// changed here: what a project's store module holds is a function of the
    /// backends its stores resolve to, which under `--target local` is always
    /// the local ones — so two compositions built for `local` are byte-identical
    /// and the composition still says nothing about where its data lives.
    #[test]
    fn every_composition_gets_the_same_backends() {
        let empty = module(&ir_of("version: \"0.1\"\n")).contents;
        let full = module(&ir_of(crate::codegen::test_support::EVERY_FORM)).contents;
        assert_eq!(empty.replace("main.yml", ""), full.replace("main.yml", ""));
        assert!(
            !full.contains(POSTGRES) && !full.contains(MYSQL),
            "a `local` build carries a dialled arm, so the zero-infra project imports a driver \
             its `package.json` does not pin (PRD 5.8, Decision D87)"
        );
    }

    /// **The arm each store's backend bound, and no other** (grammar §14.3, PRD
    /// resolved q63).
    ///
    /// The third claim is the one a reader of the ruling should be able to
    /// check: a composition whose stores are local gets the bytes every project
    /// has always had — no `pg`, no `mysql2`, and no code that would import
    /// either.
    #[test]
    fn a_project_carries_the_store_arm_each_binding_needs_and_no_other() {
        let composition = "version: \"0.1\"\n\
             store.prefs:\n  kind: kv\n  scope: global\n  backend: prefs_db\n  \
             value_schema:\n    theme: { type: string }\n\
             store.notes:\n  kind: kv\n  scope: global\n  backend: notes_db\n  \
             value_schema:\n    text: { type: string }\n";
        let deploy = "version: \"0.1\"\nstorage_backends:\n  aliases:\n    \
             prefs_db: { provider: postgres, url: \"${PREFS_URL}\" }\n    \
             notes_db: { provider: mysql, url: \"${NOTES_URL}\" }\n";
        let ir = crate::codegen::test_support::ir_of_mesh(composition, deploy);
        let emitted = module(&ir).contents;
        assert!(
            emitted.contains(SOURCE),
            "the invariant half is still there"
        );
        assert!(
            emitted.contains(POSTGRES) && emitted.contains(MYSQL),
            "a target binding both dialled providers gets both arms, or an op on one of them \
             meets an empty `STORE_BACKENDS`"
        );
        assert_eq!(
            pins(&ir),
            vec![("pg", "8.23.0"), ("mysql2", "3.24.4")],
            "the drivers go exactly where the arms do"
        );
        assert_eq!(development_pins(&ir), vec![("@types/pg", "8.23.1")]);

        // …and one provider alone brings one arm.
        let only_postgres = crate::codegen::test_support::ir_of_mesh(
            "version: \"0.1\"\nstore.prefs:\n  kind: kv\n  scope: global\n  backend: prefs_db\n  \
             value_schema:\n    theme: { type: string }\n",
            deploy,
        );
        let emitted = module(&only_postgres).contents;
        assert!(emitted.contains(POSTGRES));
        assert!(
            !emitted.contains(MYSQL),
            "a project carries an arm it never dispatches to, and the driver that arm imports \
             is one its `package.json` does not pin"
        );
    }

    /// **The halves are appended into one module, so they share no top-level
    /// name.**
    ///
    /// [`module`] concatenates the invariant half with one arm per bound
    /// provider, which makes the top level of an emitted `src/stores.ts` the
    /// *union* of the three files' top levels rather than any one of them. A
    /// name declared in two of them is therefore a duplicate declaration in
    /// every project that binds both: `tsc` refuses it (TS2393 for a function,
    /// TS2451 for a binding) and Node refuses to load the module at all, because
    /// a top-level `function` in an ES module is lexically declared — so every
    /// command of that build would die at import, before a single store op. Bun
    /// is lenient and silently keeps the last declaration, which is worse: a
    /// MySQL store would raise the Postgres arm's wording.
    ///
    /// Checked here, in Rust, rather than only by the `tsc` gate in
    /// `tests/store_backend_conformance.rs`, because this one needs no
    /// toolchain: it runs on a `cargo test` with no Bun, where that suite stands
    /// down.
    ///
    /// Names in **two different files** are what this flags. A name declared
    /// twice inside one file is that file's business — TypeScript's overload
    /// signatures and interface merging are both spelled that way, and both are
    /// as legal in the emitted module as in the source.
    #[test]
    fn the_halves_of_the_module_share_no_top_level_name() {
        use std::collections::BTreeMap;

        /// Every name a file declares at its top level — column zero, since
        /// these three are formatted files and nothing else is out there.
        fn declared(module: &str) -> Vec<&str> {
            module
                .lines()
                .filter(|line| !line.starts_with(char::is_whitespace))
                .filter_map(|line| {
                    let rest = line.strip_prefix("export ").unwrap_or(line);
                    let rest = rest.strip_prefix("declare ").unwrap_or(rest);
                    let rest = rest.strip_prefix("async ").unwrap_or(rest);
                    let rest = [
                        "function ",
                        "const ",
                        "let ",
                        "var ",
                        "class ",
                        "type ",
                        "interface ",
                        "enum ",
                    ]
                    .into_iter()
                    .find_map(|keyword| rest.strip_prefix(keyword))?;
                    let name = rest
                        .split(|character: char| {
                            !(character.is_alphanumeric() || character == '_' || character == '$')
                        })
                        .next()?;
                    (!name.is_empty()).then_some(name)
                })
                .collect()
        }

        let mut declarers: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (file, module) in [
            ("stores.ts", SOURCE),
            ("stores-postgres.ts", POSTGRES),
            ("stores-mysql.ts", MYSQL),
        ] {
            for name in declared(module) {
                declarers.entry(name).or_default().insert(file);
            }
        }
        let collisions: Vec<_> = declarers
            .iter()
            .filter(|(_, files)| files.len() > 1)
            .collect();
        assert!(
            collisions.is_empty(),
            "two halves of `src/stores.ts` declare one name, so a target that binds both arms \
             emits a module with a duplicate top-level declaration — `tsc` refuses it and Node \
             refuses to load it, which kills every command of that build at import (PRD resolved \
             q63). Give each arm its own name, or hoist the shared one into the invariant half: \
             {collisions:#?}"
        );

        // …and the scan really sees declarations, rather than passing because it
        // matched nothing: the seam an arm answers through is in the invariant
        // half, and each arm has a driver class of its own.
        assert!(declared(SOURCE).contains(&"STORE_BACKENDS"));
        assert!(declared(POSTGRES).contains(&"PostgresStoreDriver"));
        assert!(declared(MYSQL).contains(&"MysqlStoreDriver"));
    }

    /// **The MySQL arm's version floor is its upsert's, and it is asserted
    /// before the DDL** (PRD resolved q63).
    ///
    /// `duplicateKeyOverwrite` spells a `store set` with the `AS excluded` row
    /// alias, which MySQL took in **8.0.19**, while the `utf8mb4_0900_bin` the
    /// schema asks for has been there since 8.0. So the DDL is not what stops an
    /// earlier 8.0 server: it creates both tables cleanly, and every keyed write
    /// afterwards is an `ER_PARSE_ERROR` at the `AS` while every `get`, `list`
    /// and `delete` keeps answering — for the life of the deployment, since
    /// nothing about it is transient. The open therefore reads the server's
    /// version and refuses it by name *before* running the schema, which is the
    /// ordering this holds.
    ///
    /// A drift test rather than a conformance case for the reason the journal's
    /// session tests are: every server CI dials is above the floor, so no case
    /// run against a real database can see this fail.
    #[test]
    fn the_mysql_store_arm_refuses_a_server_below_the_version_its_upsert_needs() {
        assert!(
            MYSQL.contains("AS excluded ON DUPLICATE KEY UPDATE"),
            "the MySQL `set` no longer upserts through the row alias, so the 8.0.19 floor the \
             open asserts is a requirement this arm no longer has — state the floor the \
             statements really need, in `MYSQL_STORE_SCHEMA` and in `docs/topics/stores.md` too"
        );
        let checked = MYSQL.find("mysqlIsBelowFloor(version)").expect(
            "`openMysqlStore` no longer reads the server's version, so a server between \
             `utf8mb4_0900_bin` and the row alias — MySQL 8.0.0 through 8.0.18 — creates this \
             store's tables and then refuses every `store set` with a raw syntax error",
        );
        let ddl = MYSQL
            .find("for (const statement of MYSQL_STORE_SCHEMA)")
            .expect("`openMysqlStore` no longer runs `MYSQL_STORE_SCHEMA`");
        assert!(
            checked < ddl,
            "`openMysqlStore` checks the server's version after creating its tables, so an \
             operator below the floor is left with a store whose schema exists and whose writes \
             never will, rather than with the sentence `storeServerTooOld` writes"
        );
        assert!(
            MYSQL.contains("8.0.19 or newer"),
            "the refusal no longer names the version to bind instead (PRD G3)"
        );
    }

    /// **The store's schema lock and the journal's writer guard are two
    /// different locks** (PRD resolved q63).
    ///
    /// The store arm takes `pg_advisory_xact_lock` — the **blocking** form —
    /// around its DDL, and the journal takes `pg_try_advisory_lock` on a
    /// *session* scope and holds it for as long as a `serve` lives. Postgres
    /// advisory locks share one namespace per database, so if the two key pairs
    /// ever coincided, a store opened against a database a `serve` is journaling
    /// into would block on that `serve`'s guard **forever**: no SQLSTATE, no
    /// timeout, no message — every store op of the deployment simply stops. And
    /// that deployment is the shipped example, not a hypothetical:
    /// `examples/triage-fanout/deploy/staging.yml` binds its store to the same
    /// server its `journal:` names.
    ///
    /// The two constants live in two files in two modules, so nothing but this
    /// compares them — which is why the distinctness is checked here rather than
    /// left to the doc comment in `stores-postgres.ts` that states it. It cannot
    /// be a conformance case either: reproducing it needs a Postgres journal and
    /// a Postgres store open against one server at once, which is a deployment
    /// CI's suites never stand up.
    #[test]
    fn the_store_schema_lock_is_not_the_journals_writer_guard() {
        /// The two `int4` words a `readonly [number, number]` constant holds.
        ///
        /// Read out of the source rather than mirrored in Rust, because a copy
        /// here would be the drift this test is about.
        fn advisory_pair(module: &str, file: &str, name: &str) -> (u32, u32) {
            let needle = format!("const {name}: readonly [number, number] = [");
            let start = module.find(&needle).unwrap_or_else(|| {
                panic!(
                    "`{file}` no longer declares `{name}` as a pair of `int4` words, so this \
                     test can no longer tell the store's schema lock from the journal's writer \
                     guard — and those two taking one key is an `openPostgresStore` that blocks \
                     on a live `serve` with no error at all (PRD resolved q63)"
                )
            });
            let rest = &module[start + needle.len()..];
            let end = rest
                .find(']')
                .unwrap_or_else(|| panic!("`{file}`'s `{name}` is an unterminated literal"));
            let words: Vec<u32> = rest[..end]
                .split(',')
                .map(|word| {
                    let digits = word.trim().trim_start_matches("0x").replace('_', "");
                    u32::from_str_radix(&digits, 16).unwrap_or_else(|_| {
                        panic!("`{file}`'s `{name}` holds `{word}`, which is not a hex `int4`")
                    })
                })
                .collect();
            assert_eq!(
                words.len(),
                2,
                "`{file}`'s `{name}` is no longer two words: {words:?}"
            );
            (words[0], words[1])
        }

        let journal = include_str!("js/journal.ts");
        let guard = advisory_pair(journal, "journal.ts", "WRITER_GUARD_KEYS");
        let schema = advisory_pair(POSTGRES, "stores-postgres.ts", "POSTGRES_STORE_SCHEMA_LOCK");
        assert_ne!(
            guard, schema,
            "the Postgres store's schema lock and the journal's writer guard are one key pair, \
             so a target whose `journal:` and whose store name one server has `openPostgresStore` \
             waiting on `pg_advisory_xact_lock` for a session guard the hub holds until it exits: \
             every store op of that deployment hangs with no SQLSTATE, no timeout and no message \
             (PRD resolved q63). Give the store its own low word"
        );

        // …and the halves of the argument the distinctness rests on: the store
        // waits, the journal does not, and a store that had copied the guard
        // rather than declared its own would read as distinct here while taking
        // the very same lock.
        assert!(
            POSTGRES.contains(
                "pg_advisory_xact_lock(${POSTGRES_STORE_SCHEMA_LOCK[0]}, \
                 ${POSTGRES_STORE_SCHEMA_LOCK[1]})"
            ),
            "the store arm no longer takes its schema lock by the constant this test reads, so \
             the pair it really locks on is not the pair checked above"
        );
        assert!(
            journal.contains("const WRITER_GUARD_KEYS") && !POSTGRES.contains("WRITER_GUARD_KEYS["),
            "the store arm reaches for the journal's `WRITER_GUARD_KEYS` instead of its own \
             constant, which is the collision this test exists to refuse"
        );
        assert!(
            !MYSQL.contains("GET_LOCK"),
            "the MySQL store arm now takes a named lock, so it can queue behind the journal's \
             `GET_LOCK` on a server both bind — state the name it takes and check it against \
             `MYSQL_GUARD_NAME` here, the way the Postgres pair is checked above"
        );
    }

    /// The driver is reached by the one specifier PRD §9.18 admits, and the two
    /// this project refuses are named nowhere in it.
    #[test]
    fn the_backends_name_no_runtime_specific_sqlite() {
        assert!(SOURCE.contains("await import(\"node-sqlite3-wasm\")"));
        assert!(
            !SOURCE.contains("\"bun:sqlite\"") && !SOURCE.contains("\"node:sqlite\""),
            "a generated module may not reach for an API one supported runtime lacks (PRD §9.18)"
        );
    }
}
