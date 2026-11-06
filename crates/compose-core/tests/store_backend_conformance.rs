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
//!  * a schema an **earlier release** created, which every op of this one has
//!    to find brought forward at its open (PRD resolved q68's `version`
//!    column) — and a DDL that brings it forward by locking out every reader
//!    of the table on every open, which a server answers and a file does not;
//!  * a write's isolation level, which decides whether first writes into two
//!    **distinct** partitions can deadlock on each other's gap locks;
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
//! store's are `store_entries`, `store_applied` and `store_sequences`. Inside
//! those, every case here keys on a store name minted per run by the runner, so
//! two runs against one server — CI's and a developer's second `cargo test` —
//! never read each other's rows either. The one case that needs tables of its
//! own — an earlier release's schema, laid down by hand — creates a database
//! named for the run and drops it when it is done.

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
        // connection to lose and no open to redial, so these are the dialled
        // arms' own half of the contract rather than a row of grammar 11.4's
        // catalogue.
        if backend.variable.is_some() {
            for case in TRUE_WHEN_DIALLED {
                assert_eq!(
                    answered.get(*case).and_then(Value::as_bool),
                    Some(true),
                    "the `{provider}` store fails `{case}`, which is what a dialled backend \
                     promises an operator about the connections it opens and the ones its \
                     server takes away (PRD resolved q63, `storeConnectionLost` in \
                     `src/stores.ts`): {answered:#?}"
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

        // PRD resolved q70: the walk was the ruling's size on this arm, and a
        // mixed set walked three keys at a time comes back in byte order — the
        // cursor's comparison agreeing with the column's sort, on every arm.
        assert_eq!(
            answered.get("walk_keys").and_then(Value::as_u64),
            Some(100_000),
            "the `{provider}` walk seeded another number of keys than the ruling's 100,000"
        );
        assert_eq!(
            answered.get("walk_pages").and_then(Value::as_u64),
            Some(100),
            "the `{provider}` store took another number of 1000-key pages to walk 100,000 keys"
        );
        let paged: Vec<Value> = PAGED_ORDER.iter().map(|key| Value::from(*key)).collect();
        assert_eq!(
            answered.get("paged_order").and_then(Value::as_array),
            Some(&paged),
            "the `{provider}` store pages a mixed set in another order than UTF-8 byte order, so \
             two backends would disagree about which keys a page holds (grammar 11.4, PRD \
             resolved q70)"
        );
        assert_eq!(
            answered
                .get("after_a_key_the_store_does_not_hold")
                .and_then(Value::as_array),
            Some(&vec![
                Value::from("aB"),
                Value::from("ab"),
                Value::from("b")
            ]),
            "the `{provider}` store answers a cursor it does not hold as a key with the wrong page"
        );
        assert_eq!(
            answered.get("get_many_refusal").and_then(Value::as_str),
            Some(GET_MANY_REFUSAL),
            "the `{provider}` store refuses a `get_many` of 1001 keys with another message"
        );
        notice(&format!(
            "note: the `{provider}` store seeded {} keys in {} ms and walked them in {} ms",
            answered.get("walk_keys").unwrap_or(&Value::Null),
            answered.get("walk_seed_ms").unwrap_or(&Value::Null),
            answered.get("walk_ms").unwrap_or(&Value::Null),
        ));
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
    // PRD resolved q68 (a): every landed write takes the partition's next
    // version, `get` answers it, and a sequence rather than a per-key counter
    // means a key deleted and set again never repeats one.
    "a_set_answers_applied_and_the_version_it_took",
    "a_get_answers_the_version_its_key_holds",
    "a_miss_answers_no_version",
    "versions_rise_with_every_write_to_a_partition",
    "a_version_is_never_reused_across_a_delete",
    // (b) conditions: a condition that does not hold is `applied: false` with
    // the key's current version, and writes nothing.
    "if_absent_applies_once_and_then_declines",
    "if_present_declines_an_absent_key_with_no_version",
    "if_present_applies_to_a_key_that_holds_a_value",
    "if_version_applies_when_the_version_matches",
    "if_version_declines_a_stale_version_and_answers_the_current_one",
    "if_version_declines_an_absent_key",
    "a_conditional_delete_declines_a_stale_version_and_applies_a_current_one",
    "a_declined_delete_answers_the_version_the_key_holds",
    // (d) the ledger answers a retried write with its first attempt's whole
    // answer, on one connection and on two at once.
    "a_retried_claim_answers_applied_and_deduped",
    "a_concurrently_retried_claim_is_applied_once_and_answered_twice",
    // …and a retried write whose first attempt declined answers that decline,
    // even once the key is free and the same request made fresh would apply.
    "a_retried_declined_write_answers_its_first_decline",
    "a_retried_declined_partial_update_writes_nothing",
    // (c) the partial updates: from an absent key, on a held one, and against
    // the schema — a result it refuses fails naming the field and changes
    // nothing, a stored value under a changed schema included.
    "a_merge_creates_a_value_that_satisfies_the_schema",
    "an_append_adds_one_item_to_the_end",
    "a_unique_append_of_a_held_item_changes_nothing",
    "an_incr_adds_by_to_the_field",
    "a_merge_keeps_every_field_it_does_not_name",
    "an_incr_from_absent_starts_at_zero",
    "an_append_from_absent_starts_from_an_empty_array",
    "a_merge_the_schema_refuses_fails_naming_the_field",
    "a_merge_the_schema_refuses_leaves_the_row_unchanged",
    "a_merge_on_an_absent_key_must_satisfy_the_schema_whole",
    "a_refused_merge_leaves_no_ledger_row",
    // Decision D119's split at the op: the same refusal fails a node and goes
    // back to a model, by class.
    "a_refused_partial_update_fails_a_node_and_goes_back_to_a_model",
    "a_partial_update_under_a_changed_schema_fails_naming_the_field",
    "a_get_under_a_changed_schema_answers_the_stored_value",
    "two_connections_appending_at_once_keep_every_item",
    // …and on a server whose default isolation is stricter than the
    // `READ COMMITTED` every dialled arm asserts on its own session: the
    // writes queue behind each other rather than failing (PRD resolved q63,
    // q68).
    "writes_serialize_whatever_isolation_the_server_defaults_to",
    // …and the half of that level a single key cannot see: first writes into
    // eight distinct partitions at once, which under InnoDB's default
    // `REPEATABLE READ` take gap locks across partitions and deadlock writes
    // that share nothing. The MySQL arm's asserted `READ COMMITTED` is what
    // this pins.
    "concurrent_first_writes_into_distinct_partitions_all_land",
    // A store the release before PRD resolved q68 created — no `version`
    // column, no sequence table — is brought forward at its open, from four
    // openers at once: its rows answer version `0`, its ledger still dedupes,
    // `if_version: 0` names such a key, and the sequence starts past it.
    "an_earlier_releases_store_takes_every_opener_at_once",
    "a_key_an_earlier_release_wrote_answers_version_zero",
    "a_write_an_earlier_release_recorded_still_dedupes",
    "if_version_zero_names_a_key_an_earlier_release_wrote",
    "an_upgraded_partitions_sequence_starts_past_what_it_holds",
    // …and the ledger a release before PRD resolved q73 created, brought
    // forward the same way: an `expires_at` on every arm, `NULL` on every row
    // it held, and an authored write that dedupes over it.
    "a_store_from_before_authored_keys_takes_every_opener_at_once",
    "a_ledger_from_before_authored_keys_is_migrated_on_open",
    // The trace: a write record carries `applied` and `version`, and a tool's
    // carries neither an idempotency key nor `deduped`.
    "a_node_write_record_carries_applied_version_and_its_key",
    "a_tool_write_record_carries_applied_and_version_but_no_key",
    // …and the clause the ruling was asked for, read where it means anything:
    // two **processes**, each with its own SQLite handle or its own server
    // connection, writing one store from one agreed instant
    // (`store-race.mjs`).
    "two_processes_claiming_one_key_get_exactly_one_applied",
    "two_processes_appending_at_once_keep_every_item",
    "two_processes_incrementing_at_once_lose_no_increment",
    "two_processes_merging_different_fields_keep_both",
    "two_processes_never_take_one_version",
    // …and the window those races almost never land in, held open by a
    // second process on purpose (`store-window.mjs`): a write arriving inside
    // another write's read-modify-write waits for its commit and builds on it.
    "the_window_was_open_when_the_second_write_arrived",
    "a_write_inside_another_writes_window_waits_and_builds_on_it",
    // PRD resolved q69 (a)–(b), expiry, with the store's clock moved by the
    // module's own offset rather than slept through: a key with a `ttl` is
    // read until its expiry and is absent from it on, to every read and every
    // condition — so `if: absent` takes an expired key, which is a lease, and
    // expiry consumes no version.
    "a_key_with_a_ttl_is_readable_before_it_expires",
    "an_expired_key_is_absent_to_get_and_list",
    // …and to PRD resolved q70's reads alike, through the one liveness
    // fragment every read of many keys applies: a `count` stops counting it, a
    // `get_many` answers it `found: false`, and a page skips it — the page's
    // `next` included, which a page reading the expired row as its "one more"
    // would still answer. The row is still stored while they read past it, so
    // it is each read's own filter that keeps it out, not a sweep.
    "an_expired_key_is_uncounted_by_count",
    "an_expired_key_is_not_found_by_get_many",
    "an_expired_key_is_skipped_by_a_page_and_its_next",
    "the_expired_key_was_still_stored_when_those_reads_skipped_it",
    "if_absent_declines_a_key_that_has_not_expired",
    "if_present_and_if_version_decline_an_expired_key_with_no_version",
    "if_absent_applies_on_an_expired_key",
    "a_key_set_again_after_expiring_takes_the_next_version",
    // …and so they do while the expired key is still a row: behind a backlog
    // of 400 expired fillers, each write's bounded sweep takes a hundred of
    // them and leaves the key stored, so what each condition reads is an
    // expired row rather than a missing one.
    "if_present_and_if_version_decline_an_expired_key_a_sweep_has_not_reached",
    "if_absent_takes_an_expired_key_a_sweep_has_not_reached",
    // …a write refreshes the expiry to its own `ttl`, else the store's
    // `default_ttl:`, else clears it; `ttl: never` beats a default; and a
    // declined write changes nothing.
    "a_write_refreshes_expiry_to_its_own_ttl",
    "a_write_with_no_ttl_and_no_default_clears_expiry",
    "a_declined_write_leaves_expiry_unchanged",
    "a_write_with_no_ttl_takes_the_store_default",
    "ttl_never_clears_a_store_default",
    "a_writes_own_ttl_beats_the_store_default",
    "a_unique_append_that_changes_nothing_still_refreshes_expiry",
    "merge_append_and_incr_each_take_a_ttl",
    // (c) the cleanup schedule: invisible at once, out of storage at the next
    // write to the partition — at most 100 rows a write — or the next process
    // start, whichever comes first: a process whose first op on the store is a
    // read, and a process start that makes no op on the store at all.
    "an_expired_key_stays_invisible_until_a_write_removes_it",
    "the_write_sweep_removes_at_most_100_rows",
    "an_expired_key_leaves_storage_at_the_next_writes_to_its_partition",
    // …and that write's sweep is its own partition's alone: two partitions of
    // 50 expired keys each, fewer than one write's hundred together, and a
    // write to one leaves the other's every row — what keeps a write's
    // `DELETE` off rows another partition's writer is about to reach for
    // (Decision D153).
    "a_writes_sweep_takes_no_other_partitions_rows",
    "an_expired_key_leaves_storage_when_a_process_next_opens_the_store",
    "an_expired_key_leaves_storage_when_a_process_starts_without_reaching_the_store",
    // …where that open's sweep runs behind the process's first op rather than
    // in front of it — the op answers with the backlog still stored — and has
    // removed all of it by the time the process lets go of the store.
    "a_first_op_answers_before_the_start_sweep_has_finished",
    "the_start_sweep_has_removed_the_backlog_when_the_process_lets_go",
    // …in batches of at most 1000 rows, counted by a second process **between**
    // two batches (`store-sweep-batches.mjs`), so that a sweep of one unbounded
    // `DELETE` — which the two cases above would both pass — is caught: no op
    // waits for more than the one batch in progress however large the backlog.
    "the_start_sweep_takes_at_most_1000_rows_a_batch",
    "a_malformed_clock_offset_is_refused_by_name",
    // (d) the ledger a settled execution leaves is pruned, through the
    // runtime's own settle: N executions leave a bounded ledger and keep their
    // data, an unsettled one keeps its rows and its resume still dedupes, and
    // a child's rows go when the child settles.
    "an_open_executions_writes_hold_their_ledger_rows",
    "settled_executions_leave_no_ledger_rows",
    "a_settled_executions_data_stays",
    "an_unsettled_executions_ledger_rows_are_kept",
    "a_resume_of_an_unsettled_execution_still_dedupes",
    "a_prune_reaches_only_its_own_executions_prefix",
    "a_childs_ledger_rows_go_when_the_child_settles",
    "a_parents_ledger_rows_go_when_the_parent_settles",
    // …the two outcomes `settleExecution` leaves open — a parked `human`
    // pause and a replay divergence — keep every row through the settlement
    // itself, and the resume of each is answered from them.
    "a_parked_executions_settlement_keeps_its_ledger_rows",
    "a_diverged_executions_settlement_keeps_its_ledger_rows",
    // …what is pruned is every store the composition binds, so a keyed write a
    // second process made (a placement's worker) goes when the process that
    // settles prunes, though it never saw the write.
    "a_keyed_write_another_process_made_is_pruned_at_settlement",
    // …and a detached delivery still running at settlement writes under the
    // execution's id: its rows stay until it has finished, so a retry inside
    // it after the run settled is answered rather than applied twice.
    "a_detached_delivery_running_at_settlement_keeps_its_ledger_rows",
    "a_retry_inside_a_detached_delivery_after_settlement_still_dedupes",
    "a_detached_deliverys_rows_go_once_it_has_finished",
    // …unless the command lets go of its stores first, as `agent-compose run`
    // does without waiting for its deliveries: the exit starts the prune the
    // delivery was holding back, and the delivery — still running in the
    // process until it ends — is refused a keyed write from then on rather
    // than applying one its ledger can no longer answer.
    "a_command_letting_go_prunes_what_a_running_delivery_held_back",
    "a_delivery_cut_off_by_its_command_is_refused_a_keyed_write",
    // …but not a write under an authored key, whose row no prune removes
    // (PRD resolved q73).
    "a_delivery_cut_off_by_its_command_still_makes_an_authored_write",
    // …and so does a `serve` when it is stopped, the one exit it has: nothing
    // comes back for a settled execution afterwards, so the stop starts the
    // prune and refuses the delivery's keyed write the same way — while
    // closing nothing under an execution still open, which writes on, is
    // answered, and keeps its rows for the start that recovers it.
    "a_stopped_serve_prunes_what_a_running_delivery_held_back",
    "a_delivery_cut_off_by_a_stopped_serve_is_refused_a_keyed_write",
    "an_open_execution_still_writes_while_a_serve_stops",
    // …and nothing is pruned under a placed node a worker may still be
    // running: a supersession ends the attempt and not the worker's runner,
    // which writes under the same keys as the retry that replaced it, so an
    // execution whose dispatch a worker was handed and no result ended keeps
    // every row, and the runner's late write after settlement is answered from
    // one. A dispatch a result settled, or one no worker took, holds nothing.
    "a_superseded_placed_dispatch_keeps_its_executions_ledger_rows",
    "a_superseded_runners_late_write_after_settlement_still_dedupes",
    "a_placed_dispatch_still_held_at_settlement_keeps_its_executions_ledger_rows",
    "a_placed_dispatch_a_result_settled_does_not_hold_the_prune",
    "a_placed_dispatch_no_worker_took_does_not_hold_the_prune",
    // …and in a process that hands placed nodes out, the prune a delivery held
    // back asks again as it starts, so a node that delivery handed to a worker
    // after the settlement holds it the same way.
    "a_node_a_delivery_hands_out_after_settlement_holds_the_prune_when_superseded",
    "a_node_a_delivery_hands_out_after_settlement_is_pruned_behind_its_result",
    // PRD resolved q70: a 100,000-key store walked page by page, bulk-seeded
    // beside the store rather than written through it. The pages laid end to
    // end are the sorted key set — gapless, duplicate-free and in byte order —
    // with and without a prefix; every page but the last is full and its
    // `next` is its own last key, and the last answers none even when it is
    // full; `count` agrees with the walk.
    "a_fresh_store_counts_nothing",
    "a_walk_sees_every_key_exactly_once_in_byte_order",
    "a_walk_answers_no_key_twice",
    "every_page_but_the_last_is_full_and_its_next_is_its_last_key",
    "the_last_page_answers_no_next",
    "a_full_last_page_answers_no_next",
    "a_page_without_include_values_answers_no_entries",
    "a_walk_under_a_prefix_sees_every_prefixed_key_exactly_once",
    "a_prefixed_walks_last_page_is_short_and_answers_no_next",
    "count_agrees_with_the_walk",
    "count_answers_an_integer",
    // `include_values: true` carries each key's value and version, in order.
    "include_values_entries_carry_each_keys_value_and_version",
    "a_valued_walk_sees_every_key_once",
    // `get_many` of 1000 keys answers one entry per key asked, in the order
    // asked, with `found` and — only where found — `value` and `version`;
    // 1001 fails the op before anything is read or recorded.
    "get_many_of_1000_keys_answers_one_entry_per_key_in_order",
    "get_many_answers_found_with_value_and_version_or_neither",
    "get_many_answers_a_key_asked_twice_twice",
    "get_many_of_no_keys_answers_no_entries",
    "a_refused_get_many_files_no_record",
    // …and the walk hands its hundred thousand rows back when it is done — the
    // server a developer points this suite at keeps whatever a run leaves, and
    // a run-unique store nothing reads again is still a disk filling up.
    "the_walk_gives_its_rows_back",
    // A keyset cursor: a write landing mid-walk moves no page still to come.
    "after_the_last_key_answers_an_empty_last_page",
    "a_write_mid_walk_moves_no_later_page",
    // The trace and the replay: all three are reads — recorded with their
    // answer, carrying no key, no idempotency key and no `deduped`, writing no
    // ledger row — and a resumed generation answers them out of the record.
    "a_list_page_record_is_a_read_with_no_key",
    "a_count_record_is_a_read_with_no_key",
    "a_get_many_record_is_a_read_with_no_key",
    "the_reads_wrote_no_ledger_row",
    "a_replayed_count_answers_out_of_the_record",
    "a_replayed_get_many_answers_out_of_the_record",
    "a_replayed_page_answers_out_of_the_record",
    // PRD resolved q71: a `batch` is one transaction on every arm — a set, a
    // set and a delete land together at consecutive versions; a condition
    // failing at index 2 takes indexes 0 and 1 back and answers `failed: 2`; a
    // `set` entry's value is checked as a single `set`'s is (q68: CEL typing
    // at `validate`, no run-time Zod for a node), so a value the emitted Zod
    // would refuse is written by both alike; later entries read what earlier
    // ones left; each entry's `ttl` and the store's default apply as on one
    // write.
    "a_batch_of_set_set_delete_lands_all_three_with_consecutive_versions",
    "a_failed_condition_at_index_2_leaves_indexes_0_and_1_unapplied",
    "a_batch_set_entry_writes_the_value_a_single_set_writes",
    "a_duplicate_key_within_a_batch_applies_in_order",
    "an_entry_condition_reads_what_the_entries_before_it_left",
    "each_batch_entrys_ttl_and_the_store_default_apply",
    "a_batch_of_101_entries_is_refused_by_name",
    // PRD resolved q69 of a batch, which is one write: it sweeps its partition
    // once — at most 100 expired rows, not none and not 100 per entry — and
    // every entry's condition reads an expired key as absent behind a backlog
    // that sweep has not reached, so `if: absent` takes it at the next version
    // and `if: present` and `if_version:` decline it, consuming none.
    "a_batch_sweeps_at_most_100_expired_rows_of_its_partition_once",
    "an_expired_key_leaves_storage_at_the_next_batch_to_its_partition",
    "a_batch_entrys_condition_declines_an_expired_key_a_sweep_has_not_reached",
    "a_batch_entry_with_if_absent_takes_an_expired_key_a_sweep_has_not_reached",
    // One idempotency key, one ledger row holding the whole answer: a retried
    // batch dedupes whole, and so does a retried decline, which is an answer
    // too — while the same entries under a fresh key apply.
    "a_retried_batch_dedupes_whole",
    "a_retried_declined_batch_answers_its_decline_deduped",
    "the_same_entries_under_a_fresh_key_apply_once_the_condition_holds",
    // The trace: a batch is one write addressing no single key, its keys and
    // versions in the answer it carries.
    "a_batch_record_is_one_write_with_no_key_and_its_answer",
    // All or nothing under a real crash: the writing process killed with
    // `SIGKILL` between two entry writes — its first entry provably written
    // inside the open transaction — leaves the partition as it was, and the
    // retry of that effect applies, once.
    "the_batch_was_mid_transaction_when_it_was_killed",
    "the_killed_batch_had_written_its_first_entry",
    "a_batch_killed_mid_transaction_leaves_none_of_it",
    "a_batch_retried_after_its_crash_applies_once",
    // …and one that fails at its ledger row, the statement after its last
    // entry, which no crash above reaches: the row commits in the batch's own
    // transaction, so a batch that fails there — its session ended by the
    // server while its insert waits on a ledger row held under its key, or
    // the row refused by a trigger in the store's file — leaves none of its
    // entries, no version and no ledger row, and its retry applies once and
    // answers the retry after it. A ledger row written after the `COMMIT`
    // fails the first: every entry would be there with nothing recording it.
    "a_batch_that_fails_at_its_ledger_row_leaves_none_of_it",
    "a_batch_that_failed_at_its_ledger_row_is_retried_once",
    // Two processes racing batches over the same six keys in opposite orders
    // (`store-batch-race.mjs`): the partition's sequence is locked before any
    // key, so the two wait for each other rather than in a circle — every
    // batch applies, no version is taken twice and each batch's are one
    // consecutive run, a reader in a third process never finds two batches'
    // values among the six, and the six end holding the last batch's. The
    // same writers then race fresh keys into their own adjacent partitions.
    "two_processes_racing_batches_really_overlapped",
    "two_processes_racing_batches_over_shared_keys_in_opposite_orders_all_apply",
    "two_processes_racing_batches_never_take_one_version",
    "every_key_two_racing_batches_wrote_holds_the_last_batchs_value",
    "a_reader_beside_two_racing_batches_never_finds_one_half_landed",
    "two_processes_racing_batches_into_adjacent_partitions_all_apply",
    // PRD resolved q73: an authored idempotency key — `author/<store>/<key>`,
    // a business key two executions share. The second execution's write is
    // answered with the first's whole row and writes nothing; the namespace
    // carries the store, and is disjoint from every derived key; it is a
    // store's and not a partition's; two processes racing one key apply it
    // once; the settlement prune never takes it; it lives as long as the
    // store's `default_ttl:`, and for good without one; a batch dedupes whole;
    // and what it refuses is refused by name.
    "a_second_execution_under_an_authored_key_is_answered_with_the_firsts_row",
    "an_authored_key_holds_the_first_executions_value_and_version",
    "a_retry_under_an_authored_key_dedupes_the_same_way",
    "an_authored_write_record_carries_the_ledger_key_as_carried",
    "two_different_authored_keys_both_apply",
    "the_same_authored_key_on_two_stores_applies_on_both",
    "a_derived_and_an_authored_write_are_independent_ledger_rows",
    "a_delete_under_an_authored_key_a_set_took_fails_naming_both",
    "an_authored_key_past_ascii_applies_and_dedupes",
    "two_session_partitions_sharing_an_authored_key_dedupe_against_each_other",
    "two_partitions_racing_one_authored_key_apply_it_once",
    "two_processes_writing_one_authored_key_apply_it_once",
    "two_processes_in_two_partitions_writing_one_authored_key_apply_it_once",
    "a_settled_executions_prune_takes_its_derived_rows_and_leaves_its_authored_one",
    "an_authored_key_answers_within_the_stores_default_ttl",
    "an_authored_key_past_the_stores_default_ttl_applies_again",
    "an_expired_authored_row_leaves_storage_at_the_next_authored_write",
    "an_expired_authored_row_leaves_storage_when_a_process_next_starts",
    "an_authored_row_on_a_store_without_default_ttl_answers_after_any_wait",
    // …and the default is the store's, never the write's (D157): a row written
    // with a ten-minute `ttl:` still answers at eleven, and one written
    // `ttl: never` stops answering with the store's hour.
    "an_authored_row_outlives_a_shorter_ttl_its_write_gave_the_key",
    "an_authored_row_written_with_ttl_never_expires_with_the_stores_default",
    // …and the ledger's cleanup is bounded as a key's is: a write under an
    // authored key removes at most 100 of the store's expired ledger rows —
    // inside a transaction that, on a server, holds the store's authored lock
    // — and the start sweep takes them at most 1000 a batch, counted between
    // batches (`store-sweep-batches.mjs` with `ledger`).
    "an_authored_writes_ledger_sweep_removes_at_most_100_rows",
    "an_expired_ledger_backlog_leaves_storage_at_the_next_authored_writes",
    "the_start_sweep_takes_at_most_1000_ledger_rows_a_batch",
    // …while a derived row on a store with `default_ttl:` never expires: a
    // retry of an open execution's write, long past the default, is still
    // answered by its row, which carries no expiry (PRD resolved q69(d)).
    "a_derived_row_on_a_store_with_default_ttl_never_expires",
    // Two nodes naming one authored key with different ops: the later is
    // answered with the first's row wherever that row holds every field its
    // own output declares, and fails naming both where it does not.
    "partial_updates_under_one_authored_key_answer_one_another",
    "a_set_after_a_partial_update_under_one_authored_key_is_answered_without_its_value",
    "a_partial_update_after_a_set_under_one_authored_key_fails_naming_both",
    "a_batch_under_an_authored_key_dedupes_whole",
    "an_authored_key_on_an_execution_scoped_store_is_refused",
    "an_execution_id_in_the_authored_namespace_is_refused_by_the_store_and_the_journal",
    "every_derived_child_execution_id_is_outside_the_authored_namespace",
];

/// The order every arm pages a mixed set in: UTF-8 byte order, which Rust's
/// own `str` ordering is — so [`the_paged_order_is_byte_order`] holds the
/// constant to the definition rather than to a second hand-sorted copy.
const PAGED_ORDER: &[&str] = &[
    "A",
    "B",
    "Z",
    "a",
    "a b",
    "aB",
    "ab",
    "b",
    "z",
    "É",
    "ß",
    "é",
    "Ω",
    "中",
    "＀",
    "\u{1F600}",
];

/// The refusal a `get_many` of 1001 keys fails its node with, word for word,
/// the store's address aside (PRD resolved q70, grammar 11.4).
const GET_MANY_REFUSAL: &str = "a `get_many` on `<store>` was asked for 1001 keys, and one \
     `get_many` reads at most 1000 (grammar 11.4, PRD resolved q70): read them in slices of 1000 \
     or fewer, or walk the store a page at a time with `list` and `after:`";

/// [`PAGED_ORDER`] is sorted the way the arms are promised to sort, which is
/// byte order — not the order a UTF-16 or a case-folding comparison answers.
#[test]
fn the_paged_order_is_byte_order() {
    let mut sorted = PAGED_ORDER.to_vec();
    sorted.sort_unstable();
    assert_eq!(sorted, PAGED_ORDER);
    let utf16 = {
        let mut keys = PAGED_ORDER.to_vec();
        keys.sort_by(|left, right| left.encode_utf16().cmp(right.encode_utf16()));
        keys
    };
    assert_ne!(
        utf16, PAGED_ORDER,
        "the set is chosen so that a UTF-16 comparison orders it differently"
    );
}

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
///
/// The redial is also why the first case here is in this list: a dialled arm
/// runs its schema at **every** open, so an open that waits on a reader is a
/// `serve` that cannot come back for as long as a `pg_dump` or an operator's
/// open transaction lasts — and a local file has neither a server-side lock
/// queue nor a redial.
const TRUE_WHEN_DIALLED: &[&str] = &[
    // The schema an open runs takes no lock a reader holds against it — so no
    // bare `ALTER TABLE` on every open, which Postgres runs under
    // `ACCESS EXCLUSIVE` whether or not there is anything to add.
    "an_open_does_not_wait_on_a_reader",
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
    // PRD resolved q69 (a): a dialled store's expiry is the **server's** clock
    // plus the `ttl`, so writers whose own clocks disagree agree about which
    // keys are alive — the row holds the server's instant with this process's
    // clock ten days off.
    "expiry_is_computed_by_the_servers_clock",
    // …and every read judges that row by the same clock: with this process's
    // clock ten days ahead or behind, a `get`, a `list`, a `count` and a
    // `get_many` find the key while the server's clock holds it alive and miss
    // it once that clock (moved by the store offset) is past its expiry.
    "a_dialled_read_finds_a_key_the_servers_clock_holds_alive",
    "a_dialled_read_misses_a_key_the_servers_clock_holds_expired",
    // …and so does the store-wide sweep a connection's first op starts: a
    // process whose clock runs ten days fast takes no key the server's clock
    // holds alive, and one ten days slow still takes a key it holds expired.
    "a_store_wide_sweep_keeps_a_key_the_servers_clock_holds_alive",
    "a_store_wide_sweep_takes_a_key_the_servers_clock_holds_expired",
    // PRD resolved q73: an authored ledger row's expiry is the server's clock
    // plus the store's `default_ttl:` too — the instant the key beside it holds
    // — so writers whose own clocks disagree agree about when a business key
    // stops deduping; and a redelivery with this process's clock ten days the
    // other way is still answered by the row, which the ledger read judges by
    // the server's clock.
    "an_authored_rows_expiry_is_computed_by_the_servers_clock",
    "a_dialled_ledger_read_answers_an_authored_row_the_servers_clock_holds_alive",
    // PRD resolved q69 (c): the two halves of the cleanup schedule — a write's
    // sweep of its partition and the store-wide sweep a connection's first op
    // starts — run at once in two processes over one backlog without either
    // failing as a deadlock, the sweep takes no key a write refreshed, and the
    // backlog is gone once both processes have let go of the store. Each of the
    // three also holds only where the store-wide sweep really ran during the
    // writer's run: the backlog carries rows only that sweep can take.
    "a_store_wide_sweep_and_a_write_never_deadlock",
    "a_store_wide_sweep_takes_no_key_a_write_refreshed",
    "a_store_wide_sweep_removes_the_whole_backlog",
    // …and each batch of that sweep takes only the rows of the one partition
    // whose sequence lock it holds — the store's oldest expired row's — and
    // none of another's, counted between batches as above: a batch with no
    // partition in its `DELETE` would hold rows a write to another partition
    // reaches for under a lock that write does not take, which is the deadlock
    // the lock exists to rule out (`StoreDialect.sweepPartition`, D153). The
    // race above cannot tell the two apart: its backlog is one partition's.
    "a_store_wide_sweep_batch_takes_only_its_own_partitions_rows",
    // PRD resolved q69 (d): a settlement prunes every store the composition
    // binds, so a bound store whose server accepts a connection and never
    // answers is one every settlement meets — and it delays neither the
    // settlement nor any other store's prune, and holds a command's exit for
    // the dial deadline and no longer.
    "a_store_that_never_answers_delays_no_settlement",
    "a_store_that_never_answers_delays_no_other_stores_prune",
    "a_store_that_never_answers_holds_a_command_for_its_dial_deadline_alone",
    // PRD resolved q71 on the dialled arms' own failure: a batch whose session
    // the server ends between two entry writes — caught waiting on a row lock
    // after its first entry is written, which an uncommitted insert of that
    // key proves — is refused by name, leaves none of its entries, no version
    // and no ledger row, and its retry applies once.
    "the_batch_was_caught_between_two_entries",
    "the_ended_batch_had_written_its_first_entry",
    "a_batch_whose_connection_the_server_ends_mid_transaction_leaves_none_of_it",
    "a_batch_whose_connection_ended_is_retried_once",
    // …and the premise of the ledger-row case above on a server: the batch
    // was caught waiting on the ledger row this runner's connection holds
    // under its key, with every one of its entries still uncommitted — an
    // insert of each key waits on it — so the session the server ends is one
    // whose entries and ledger row are one transaction.
    "the_batch_was_caught_waiting_on_its_ledger_row",
    "the_batch_caught_at_its_ledger_row_held_every_entry_uncommitted",
    // A server keeps what a run leaves on it, so the run takes every row of its
    // own run-unique stores back off it at the end (the large walk's hundred
    // thousand go first, in that case's own `finally`: `TRUE_EVERYWHERE`).
    "the_run_gives_its_rows_back",
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
