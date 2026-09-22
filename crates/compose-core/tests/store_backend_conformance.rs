//! One contract, three backends: grammar 11.4's `kv` catalogue run against each
//! backend a deploy file can bind it to (grammar §14.3, PRD resolved q63).
//!
//! # Why this suite exists
//!
//! The ops in the emitted `src/stores.ts` are **one** implementation of grammar
//! 11.4 shared by SQLite, Postgres and MySQL; a dialled arm supplies a
//! connection and a schema and nothing else, and `runStoreOp` is the entry point
//! on every backend — which is what makes "a store-op node, a synthesized tool
//! and the trace record they file cannot tell which arm answered" true by
//! construction rather than by review.
//!
//! What is left is everything a **server** answers, and none of it is visible to
//! `tsc` or to a fake:
//!
//!  * a collation that folds case, where two `kv` keys differing in case are two
//!    keys and the primary key has to tell them apart;
//!  * one that **pads**, where `'a'` and `'a '` become one row — MySQL's older
//!    `utf8mb4_bin` is PAD SPACE, and the arm asks for `utf8mb4_0900_bin`
//!    precisely to be rid of it;
//!  * a column too small for a value a `value_schema` admits — MySQL's `TEXT` is
//!    64 KiB — or one that *normalizes* the JSON it was handed;
//!  * the key order a `list` answers in, which is the column's collation
//!    answering and has to be the UTF-8 byte order every other arm gives;
//!  * a key column bounded where another backend's is not, so a key or a
//!    session key past that bound is a green `store set` on two arms and a
//!    refusal on the third;
//!  * a DDL that is not idempotent, so a second open of a store this build
//!    already created fails rather than doing nothing — or one that is not
//!    idempotent *concurrently*, which is the case a store has and the journal
//!    does not: grammar 14.1 rule 5 admits a dialled store from a placement, so
//!    several first opens against one fresh database is the ordinary start-up;
//!  * a unique index and a conflict-ignore that really make a double-delivered
//!    write land **once** (grammar 9.4, PRD 5.8) — which is the clause resolved
//!    q63 moved from the caller's promise to the backend's;
//!  * a placeholder dialect — `?` against `$1` — and an upsert spelled two ways;
//!  * a connection the **server** takes away under the op that was using it,
//!    which is the one failure a `serve` cannot be restarted out of. See
//!    [`TRUE_WHEN_DIALLED`], which is the half of the contract only a backend
//!    with a socket has.
//!
//! # How it runs, and what it does when a server is absent
//!
//! Every backend is **built and type-checked** on every run, server or no
//! server: neither needs one, and the MySQL store arm has no golden carrying it,
//! so this is the only place `src/codegen/js/stores-mysql.ts` meets `tsc` on a
//! machine with no database.
//!
//! A **fourth** project is built and type-checked beside those three, and it
//! binds one store to Postgres and another to MySQL. `src/stores.ts` is
//! assembled from the invariant half plus one arm per bound provider, so a
//! project per provider never compiles the *union* of the arms — and the union
//! is where two arms declaring one name become a duplicate top-level
//! declaration that `tsc` refuses and Node will not even load. See
//! [`a_target_binding_both_dialled_arms_is_one_module_that_compiles`].
//!
//! The **cases** then run where there is something to run them against. SQLite
//! always: it needs nothing but the pinned driver, so a developer machine runs
//! the same cases CI does. Each dialled provider runs when its environment
//! variable names a reachable server — [`POSTGRES_URL`] and [`MYSQL_URL`], the
//! same two the journal suite reads, because one CI service pair serves both —
//! and is **skipped loudly** when it does not, naming the variable to look at.
//! Loudly means through [`notice`] rather than `eprintln!`, which libtest
//! swallows for a test that passes. Never silently green.
//!
//! The two suites share those servers and never share a row: the journal's
//! tables are `executions`, `effects`, `deliveries` and `dispatches`, and a
//! store's are `store_entries` and `store_applied`. Inside those, every case
//! here keys on a store name minted per run by the runner, so two runs against
//! one server — CI's and a developer's second `cargo test` — never read each
//! other's rows either.

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

/// Where a reachable Postgres for the dialled arm is named.
///
/// The journal suite's variable, deliberately: one `postgres:17` service in
/// `.github/workflows/ci.yml` serves both contracts, and a second name would be
/// a second thing for a workflow to forget to set.
const POSTGRES_URL: &str = "AGENT_COMPOSE_TEST_POSTGRES_URL";

/// …and a MySQL.
const MYSQL_URL: &str = "AGENT_COMPOSE_TEST_MYSQL_URL";

/// One backend the suite drives, and how it is reached.
struct Backend {
    /// The `provider:` keyword, as grammar §14.3 spells it.
    provider: &'static str,
    /// The variable holding a reachable server, or `None` for the one that opens
    /// a file and needs nothing.
    variable: Option<&'static str>,
}

/// Every backend a `kv` store can be bound to and this release opens.
///
/// `memory` is the fourth `kv` provider this release implements and is not here:
/// it is a heap map that dies with the process, so a contract case about what a
/// second open finds, or about what another execution sees, would be asserting
/// the absence the provider is named for. It is driven by gate 16 of
/// `generated_code_gates`, beside the other local backends.
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
/// Deliberately the smallest one that builds a project with a `kv` store in it:
/// what is under test is the backend, and every case the runner drives reaches
/// `runStoreOp` directly rather than through a graph. The store's `backend:`
/// alias is what the deploy file below binds, which is what decides the arm
/// `build` emits into `src/stores.ts`.
const COMPOSITION: &str = "version: \"0.1\"\n\
store.contract:\n  \
  kind: kv\n  \
  scope: global\n  \
  description: What the conformance suite writes.\n  \
  backend: contract_db\n  \
  value_schema:\n    \
    text: { type: string }\n\
flow.probe:\n  \
  outputs: {}\n  \
  nodes:\n    \
    step: { exec: { command: \"true\" } }\n  \
  edges:\n    \
    - { from: start, to: step }\n    \
    - { from: step, to: end }\n";

/// …and the composition that binds **two** stores, for the arms-together case
/// below.
///
/// Two `kv` stores rather than one, because a deploy file binds a backend to an
/// alias and a store names one alias: two providers in one project is two
/// stores. Otherwise it is [`COMPOSITION`] — the same probe flow, which is there
/// so that a project is a project rather than because anything runs it. Nothing
/// ever does: see
/// [`a_target_binding_both_dialled_arms_is_one_module_that_compiles`].
const BOTH_ARMS: &str = "version: \"0.1\"\n\
store.prefs:\n  \
  kind: kv\n  \
  scope: global\n  \
  description: What the Postgres arm answers for.\n  \
  backend: prefs_db\n  \
  value_schema:\n    \
    text: { type: string }\n\
store.notes:\n  \
  kind: kv\n  \
  scope: global\n  \
  description: …and the MySQL one.\n  \
  backend: notes_db\n  \
  value_schema:\n    \
    text: { type: string }\n\
flow.probe:\n  \
  outputs: {}\n  \
  nodes:\n    \
    step: { exec: { command: \"true\" } }\n  \
  edges:\n    \
    - { from: start, to: step }\n    \
    - { from: step, to: end }\n";

/// The variable the emitted project's store binding names its address in.
///
/// Named here rather than reused from the ambient environment: the emitted
/// `src/graph.ts` carries the *name* the deploy file wrote, so the runner's
/// process has to hold that name — and holding the suite's own variable under it
/// keeps the deploy file honest about being an `${ENV}` reference rather than an
/// address.
const STORE_URL: &str = "AGENT_COMPOSE_STORE_URL";

/// …and four more names for the same server, which is how the runner's
/// interleaved-writer, concurrent-open and killed-connection cases become
/// several connections rather than one.
///
/// `src/stores.ts` caches a dialled connection per provider and variable name,
/// so two bindings naming one variable share a socket by design — which is right
/// for a graph and useless for a case about two writers, about four processes of
/// a placement reaching one fresh database at the same moment, or about a
/// connection the server takes away: the last of those has to be a socket no
/// other case is holding.
const OTHER_STORE_URLS: &[&str] = &[
    "AGENT_COMPOSE_STORE_URL_B",
    "AGENT_COMPOSE_STORE_URL_C",
    "AGENT_COMPOSE_STORE_URL_D",
    "AGENT_COMPOSE_STORE_URL_E",
];

/// Build the project `composition` and `deploy` describe, and answer where the
/// emitted TypeScript landed.
///
/// `area` names a scratch directory of its own, because libtest runs the tests
/// of one binary in **parallel threads**: two tests both building under one path
/// would be one `remove_dir_all` racing the other's `bun`.
fn build_project(root: &Path, area: &str, composition: &str, deploy: &str) -> PathBuf {
    let source = root.join("projects").join("store-sources").join(area);
    let _ = fs::remove_dir_all(&source);
    fs::create_dir_all(source.join("deploy")).expect("the scratch area is writable");
    fs::write(source.join("main.yml"), composition).expect("the entrypoint is writable");
    fs::write(source.join("deploy/remote.yml"), deploy).expect("the deploy file is writable");

    let entrypoint = source.join("main.yml");
    let resolution = compose_core::resolve_with_target(&entrypoint, "remote");
    assert!(
        resolution.diagnostics.is_empty(),
        "the `{area}` fixture does not resolve: {:#?}",
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "the `{area}` fixture does not validate: {diagnostics:#?}"
    );

    let authored = compose_core::Authored::read(&ir, &source)
        .expect("the fixture references no module binding");
    let destination = root.join("projects").join("store").join(area);
    let _ = fs::remove_dir_all(&destination);
    for file in compose_core::emit(&ir, &authored).files() {
        let target = destination.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(target.parent().expect("an emitted path has a parent"))
            .expect("the scratch area is writable");
        fs::write(&target, &file.contents).expect("an emitted file is writable");
    }
    destination
}

/// Build one project whose store binds `provider`, and answer where it landed.
fn built(root: &Path, provider: &str) -> PathBuf {
    // `local` substitutes local storage for every store unconditionally and
    // consults no alias at all (Decision D87), so the target has a name — which
    // is also the shape an operator writes, since a store on a server is a
    // deployment rather than a laptop.
    let deploy = if provider == "sqlite" {
        format!(
            "version: \"0.1\"\nstorage_backends:\n  aliases:\n    contract_db: {{ provider: {provider} }}\n"
        )
    } else {
        format!(
            "version: \"0.1\"\nstorage_backends:\n  aliases:\n    contract_db: {{ provider: {provider}, url: \"${{{STORE_URL}}}\" }}\n"
        )
    };
    build_project(root, provider, COMPOSITION, &deploy)
}

/// Type-check one built project, naming it in the failure.
fn type_check(project: &Path, what: &str) {
    let checked = bun()
        .args(["run", "typecheck"])
        .current_dir(project)
        .output()
        .expect("bun runs");
    assert!(
        checked.status.success(),
        "the `{what}` store project does not type-check:\n{}\n{}",
        String::from_utf8_lossy(&checked.stdout),
        String::from_utf8_lossy(&checked.stderr),
    );
}

/// Say something a plain `cargo test` really prints.
///
/// libtest captures a **passing** test's output, so a skip notice written with
/// `eprintln!` reaches nobody: `cargo test` prints `1 passed` and nothing else,
/// and a laptop run that drove one of three backends reads exactly like CI's run
/// that drove all three.
fn notice(message: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{message}");
    let _ = stderr.flush();
}

/// Drive the contract runner against one built project, or `None` where the
/// backend's server is absent.
fn contract(root: &Path, backend: &Backend) -> Option<BTreeMap<String, Value>> {
    // **Built and type-checked before the server is asked for**, because neither
    // needs one and what would go unchecked is not small: a dialled arm is
    // emitted only into a project that pins its driver, and this is where that
    // pairing is checked for the arm no golden carries. `triage-fanout-staging`
    // carries the Postgres one; **MySQL has no golden at all**, so a wrong
    // `RowDataPacket` generic or a renamed export from `mysql2/promise` would
    // pass `cargo fmt`, `cargo clippy` and `cargo test --workspace` on a machine
    // with no MySQL and fail only in CI. What an absent server skips is the
    // **run**, below.
    let project = built(root, backend.provider);
    type_check(&project, backend.provider);

    let address = match backend.variable {
        None => None,
        Some(variable) => match std::env::var(variable) {
            Ok(value) if !value.is_empty() => Some(value),
            _ => {
                assert!(
                    !required(),
                    "`{variable}` is required in CI: `.github/workflows/ci.yml` runs a `{}` \
                     service container for it, and this suite is what makes the `{}` `kv` \
                     backend a checked promise rather than a paragraph in the grammar (PRD \
                     resolved q63)",
                    backend.provider,
                    backend.provider,
                );
                notice(&format!(
                    "warning: skipping the `{}` store conformance cases — `{variable}` is unset, \
                     so there is no server to drive them against. The project was still built and \
                     type-checked. It is set in CI (see .github/workflows/ci.yml); set it locally \
                     to run them.",
                    backend.provider
                ));
                return None;
            }
        },
    };

    // A data directory of its own per backend: the SQLite arm writes its store
    // there, and every arm writes the journal the replay cases read.
    let data = root.join("projects").join("store").join(backend.provider);
    let data = data.join("data");
    let _ = fs::remove_dir_all(&data);

    let mut command = runner("store-contract.mjs");
    command.arg(&project).arg(backend.provider).arg(&data);
    if let Some(address) = address {
        // Four names, one server. See [`OTHER_STORE_URLS`].
        command.env(STORE_URL, &address);
        for variable in OTHER_STORE_URLS {
            command.env(variable, &address);
        }
    }
    let output = command.output().expect("bun runs");
    assert!(
        output.status.success(),
        "the `{}` store did not answer the contract:\n{}",
        backend.provider,
        String::from_utf8_lossy(&output.stderr),
    );
    Some(serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object"))
}

/// **The store contract, against every backend a `kv` store can be bound to.**
///
/// One set of cases and one set of expectations, because there is one contract:
/// grammar 11.4's catalogue is backend-invariant, and a case that answered
/// differently on two backends would be that table's promise broken on one of
/// them. The provider is in the failure message rather than in the expectation.
#[test]
fn every_store_backend_answers_the_same_contract() {
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

        for case in TRUE_EVERYWHERE {
            assert_eq!(
                answered.get(*case).and_then(Value::as_bool),
                Some(true),
                "the `{provider}` store fails `{case}`, which grammar 11.4 and PRD 5.8 promise \
                 of every backend: {answered:#?}"
            );
        }

        // …and the cases only a backend with a socket has. A local store has no
        // connection to lose, so these are the dialled arms' own half of the
        // contract rather than a row of grammar 11.4's catalogue.
        if backend.variable.is_some() {
            for case in TRUE_WHEN_DIALLED {
                assert_eq!(
                    answered.get(*case).and_then(Value::as_bool),
                    Some(true),
                    "the `{provider}` store fails `{case}`, which is what a dialled backend \
                     promises an operator about a connection its server takes away (PRD \
                     resolved q63, `storeConnectionLost` in `src/stores.ts`): {answered:#?}"
                );
            }
        }

        // `list` answers in UTF-8 byte order on every backend, which on a server
        // is the column's collation answering: U+FF00 is `EF BC 80` and U+1F600
        // is `F0 9F 98 80`, so bytes put U+FF00 first while a UTF-16 comparison
        // puts the surrogate first. With the `limit:` grammar 11.4 requires,
        // that is not a different order but a different answer.
        assert_eq!(
            answered.get("list_order").and_then(Value::as_array),
            Some(&vec![
                Value::from("zz"),
                Value::from("＀"),
                Value::from("\u{1F600}"),
                Value::from("😀alpha"),
                Value::from("😀beta"),
            ]),
            "the `{provider}` store answers `list` in another order, so two backends disagree \
             about which keys a `limit:` returns at all (grammar 11.4)"
        );
        assert_eq!(
            answered.get("list_limited").and_then(Value::as_array),
            Some(&vec![Value::from("zz"), Value::from("＀")]),
            "the `{provider}` store's `limit:` takes another prefix of that order"
        );
        // …and a `prefix:` outside the BMP, which the two sides of the
        // comparison count differently: the servers count characters in
        // `SUBSTR` and a JavaScript `.length` counts UTF-16 code units.
        assert_eq!(
            answered.get("prefix_keys").and_then(Value::as_array),
            Some(&vec![
                Value::from("😀"),
                Value::from("😀alpha"),
                Value::from("😀beta"),
            ]),
            "the `{provider}` store answers an astral `prefix:` with the wrong keys, which is a \
             legal read answered wrongly and silently (grammar 11.4)"
        );
        assert_eq!(
            answered.get("deeper_prefix_keys").and_then(Value::as_array),
            Some(&vec![Value::from("😀alpha")]),
            "the `{provider}` store answers a longer astral `prefix:` with the wrong keys"
        );
    }
    assert!(
        driven.contains(&"sqlite"),
        "SQLite needs no server and is always driven; a run that skipped it read nothing"
    );
    notice(&format!(
        "note: the store contract ran against {driven:?}; every backend was built and type-checked"
    ));
}

/// The cases every backend must answer `true`.
///
/// One list, because there is one contract. Each names the promise it is about,
/// so a failure reads as the sentence of the grammar or the PRD it broke rather
/// than as an index into a runner.
const TRUE_EVERYWHERE: &[&str] = &[
    // Grammar 11.4's `kv` rows, and what a value column has to hold: the JSON
    // this runtime produced, back as it went in. A `JSONB`/`JSON` column would
    // normalize it, and MySQL's `TEXT` would refuse the large one.
    "round_trips_a_unicode_key_and_value",
    "round_trips_the_types_value_schema_admits",
    "round_trips_a_value_past_64_kib",
    "a_miss_answers_found_false_with_no_value",
    "a_delete_answers_once",
    // The key column's collation, in both of the ways it goes wrong.
    "keys_are_case_sensitive",
    "keys_keep_a_trailing_space",
    // …and its *bound*, which is the same "a write two arms take and one
    // refuses is not one contract" read on the column the key lives in. Grammar
    // 11.4 restricts a `kv` key not at all, and a partition name is the encoded
    // session key a trigger supplied rather than anything an author wrote.
    "round_trips_a_key_longer_than_512_characters",
    "two_long_keys_sharing_a_prefix_are_two_rows",
    "a_long_session_key_is_its_own_partition",
    // A fresh schema, created by several first opens at once — which grammar
    // 14.1 rule 5 makes the ordinary start-up of a placement rather than an edge
    // case, and which `CREATE TABLE IF NOT EXISTS` is not atomic against.
    "a_fresh_schema_takes_every_opener_at_once",
    "every_opener_wrote_through_its_own_connection",
    // Grammar 11.3's three scopes, keyed exactly as the local backend keys them
    // — which is what makes a graph moved between backends address the same
    // rows.
    "executions_do_not_share_an_execution_scoped_store",
    "an_execution_scoped_store_dies_with_the_run",
    "another_execution_keeps_its_own_partition",
    "one_session_is_shared_across_executions",
    "sessions_do_not_share_a_session_scoped_store",
    "a_global_store_is_shared_across_executions",
    // Grammar 9.4's idempotency key, enforced by the backend rather than
    // promised of the caller (PRD resolved q63).
    "a_double_delivered_write_lands_once",
    "the_second_delivery_is_recorded_as_deduped",
    "a_write_under_another_key_is_another_effect",
    // …and the clause "by the backend" read at the one moment it means
    // anything: the same key in flight on **two** connections at once. The three
    // above run over one binding, so one queue on one socket serializes them and
    // a dedupe that read the ledger before writing it would pass them all. Two
    // transactions racing can only be arbitrated by the ledger's primary key,
    // and exactly one of them may answer `deduped: false`.
    "a_concurrent_double_delivery_is_deduped_by_the_backend",
    "a_concurrent_double_delivery_leaves_the_applied_value",
    // Multi-writer by design, and per key last write wins.
    "a_second_writer_reads_what_the_first_wrote",
    "two_writers_share_one_store_and_the_last_write_wins",
    // The DDL is idempotent and what was written is still there.
    "a_second_open_finds_what_the_first_wrote",
    // PRD 5.8's replay discipline, unchanged by the arms underneath it.
    "a_replayed_read_answers_out_of_the_record",
    "a_replayed_write_is_not_applied_again",
];

/// The cases a backend with a **socket** must answer `true`, on top of those.
///
/// A local store has no connection to lose, so this is the dialled arms' own
/// half of the contract rather than a row of grammar 11.4's catalogue — and it
/// is the half a `serve` depends on most, because a `serve` is the deployment
/// with no next run to fix anything: a store whose connection is taken away by a
/// managed failover, a proxy's idle reaper or an operator's `KILL` has to come
/// back on the next op, or it is gone for the life of the process.
///
/// The runner kills the connection with a statement **in flight**, which is what
/// makes these cases about more than a driver's `error` event: `mysql2` reports
/// a socket that died under a command to that command alone and emits nothing,
/// so an arm that listened only to the event would leave a dead connection in
/// the dialled cache and refuse every later op with a raw
/// `Can't add new command when connection is in closed state`.
const TRUE_WHEN_DIALLED: &[&str] = &[
    // The premise of the two below: the kill really landed on a statement the
    // server could see waiting, rather than on an idle socket.
    "the_store_was_caught_mid_statement",
    // The op that met the loss is refused with the sentence that names the
    // variable an operator can change, rather than with the driver's own.
    "a_lost_connection_is_refused_by_name",
    // …and the next op runs over a connection dialled again, against the rows
    // the lost one had already committed.
    "a_lost_connection_is_redialled",
    "a_redialled_store_still_holds_what_it_wrote",
];

/// **A target that binds both dialled arms gets one module, and it compiles.**
///
/// `src/stores.ts` is **assembled** — the invariant half plus one arm per
/// provider this composition's stores bind (`codegen::stores`) — so the arms are
/// not three files a reader checks one at a time: the top level of an emitted
/// module is their *union*. Two arms that happen to declare one name are a
/// duplicate top-level declaration there, and that is not a style problem.
/// `tsc` refuses it (TS2393 for a function, TS2451 for a binding), so the
/// emitted project fails its own `typecheck` script; Node refuses to load the
/// module at all, because a top-level `function` in an ES module is lexically
/// declared, so every command of that build — `run`, `serve`, `resume` — dies at
/// import before a single store op; and Bun is lenient, silently keeping the
/// last declaration, which is worse than either, because a MySQL store would
/// then raise the Postgres arm's wording on the runtime the README makes
/// default.
///
/// The suite above builds one project **per provider** and would never see it,
/// and no golden carries two dialled arms either (`triage-fanout-staging` is
/// Postgres twice over, its store and its journal). This is the only place the
/// union is compiled.
///
/// No server, and none is skipped for: what is under test is the composition of
/// the arms, which `tsc` answers on a machine with no database.
#[test]
fn a_target_binding_both_dialled_arms_is_one_module_that_compiles() {
    let Some(root) = installed() else {
        return;
    };
    // Two variables, because two aliases naming one would be one connection —
    // fine for a graph, and beside the point here. Neither is read: nothing runs.
    let deploy = format!(
        "version: \"0.1\"\nstorage_backends:\n  aliases:\n    \
         prefs_db: {{ provider: postgres, url: \"${{{STORE_URL}}}\" }}\n    \
         notes_db: {{ provider: mysql, url: \"${{{}}}\" }}\n",
        OTHER_STORE_URLS[0]
    );
    let project = build_project(root, "both-arms", BOTH_ARMS, &deploy);

    // The project really carries both arms, so a green run cannot be one that
    // quietly compiled a single-arm module.
    let module = fs::read_to_string(project.join("src/stores.ts")).expect("the module is emitted");
    for registration in ["STORE_BACKENDS.postgres = ", "STORE_BACKENDS.mysql = "] {
        assert!(
            module.contains(registration),
            "the two-provider project's `src/stores.ts` carries no `{registration}`, so this case \
             type-checks a module that is not the union it exists to check (PRD resolved q63)"
        );
    }

    type_check(&project, "postgres+mysql");
}

/// **Every `kv` backend this release opens is driven, or says why it is not.**
///
/// The suite above quantifies over [`BACKENDS`], which is only the whole story
/// if that list is grammar §14.3's `kv` row minus the one this suite states it
/// leaves out. A provider made live in a later release and not added here would
/// be a backend this suite reports nothing about while reading as though it
/// covered them all.
#[test]
fn the_suite_drives_every_kv_backend_this_release_opens() {
    use compose_core::ast::definition::StoreKind;
    use compose_core::ast::deploy::BackendProvider;

    let live: Vec<&str> = BackendProvider::ALL
        .iter()
        .filter(|provider| provider.kind() == StoreKind::Kv && provider.implemented())
        .map(|provider| provider.as_str())
        // `memory` is stated as an exclusion on [`BACKENDS`] and is driven by
        // gate 16 of `generated_code_gates` instead.
        .filter(|name| *name != "memory")
        .collect();
    let driven: Vec<&str> = BACKENDS.iter().map(|backend| backend.provider).collect();
    assert_eq!(
        driven, live,
        "a `kv` backend this release opens is not driven by this suite, so grammar 11.4's \
         catalogue is unchecked on one of the backends a deploy file can name (grammar §14.3, \
         PRD resolved q63)"
    );

    // …and each dialled one names a variable of its own, which is what the skip
    // above prints and what `.github/workflows/ci.yml` sets.
    let workflow = fs::read_to_string(repository().join(".github/workflows/ci.yml"))
        .expect("the workflow is readable");
    for backend in BACKENDS {
        let opens_in_process = BackendProvider::ALL
            .iter()
            .find(|provider| provider.as_str() == backend.provider)
            .expect("every driven provider is one the grammar spells")
            .opens_in_process();
        assert_eq!(
            backend.variable.is_none(),
            opens_in_process,
            "`{}` opens in process: {opens_in_process}, and names a server variable: {}. A \
             provider that dials out needs one and a provider that opens a file has none",
            backend.provider,
            backend.variable.is_some()
        );
        let Some(variable) = backend.variable else {
            continue;
        };
        assert!(
            workflow.contains(variable),
            "`.github/workflows/ci.yml` sets no `{variable}`, so CI skips the `{}` store while \
             reporting the suite green — which is exactly what the service containers were \
             ratified to prevent (PRD resolved q62, q63)",
            backend.provider
        );
    }
}

/// **The runtime's list of implemented backends is the compiler's.**
///
/// Two readings of "which providers does this release open" exist by necessity:
/// `src/codegen/js/stores.ts` decides at the op whether to answer or to refuse,
/// and [`BackendProvider::implemented`] decides what
/// `agent-compose validate` offers as a repair a build of this release can run
/// (`check::placements`). A drift between them is silent in the worst direction
/// — a diagnostic recommending a backend the runtime throws on, which is the
/// exact failure that caveat was written to prevent.
#[test]
fn the_runtime_and_the_compiler_name_one_set_of_implemented_backends() {
    use compose_core::ast::deploy::BackendProvider;

    const MODULE: &str = include_str!("../src/codegen/js/stores.ts");
    let listed = |declaration: &str| -> Vec<String> {
        MODULE
            .split_once(&format!(
                "const {declaration}: readonly BackendProvider[] = "
            ))
            .unwrap_or_else(|| panic!("`src/codegen/js/stores.ts` declares `{declaration}`"))
            .1
            .split_once("];")
            .expect("the list is closed")
            .0
            .split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_string)
            .collect()
    };
    let mut runtime = listed("IN_PROCESS_PROVIDERS");
    runtime.extend(listed("DIALLED_KV"));
    runtime.sort_unstable();

    let mut compiler: Vec<String> = BackendProvider::ALL
        .iter()
        .filter(|provider| provider.implemented())
        .map(|provider| provider.as_str().to_string())
        .collect();
    compiler.sort_unstable();

    assert_eq!(
        runtime, compiler,
        "`src/codegen/js/stores.ts` answers one set of providers and `BackendProvider::\
         implemented` another, so `validate` offers a repair the runtime refuses at the first \
         store op — or refuses one it would have answered (PRD resolved q63)"
    );

    // …and the two halves of the runtime's own answer are the placement rule's:
    // what `opens_in_process` classifies is what `src/stores.ts` opens itself.
    let mut in_process = listed("IN_PROCESS_PROVIDERS");
    in_process.sort_unstable();
    let mut local: Vec<String> = BackendProvider::ALL
        .iter()
        .filter(|provider| provider.opens_in_process())
        .map(|provider| provider.as_str().to_string())
        .collect();
    local.sort_unstable();
    assert_eq!(
        in_process, local,
        "`src/stores.ts` opens a backend in process that grammar 14.1 rule 5 treats as dialled, \
         or the other way round — so a mesh `validate` admits would fork its state silently \
         (Decision D131)"
    );
}
