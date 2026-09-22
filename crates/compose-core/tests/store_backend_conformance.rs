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
//!  * a DDL that is not idempotent, so a second open of a store this build
//!    already created fails rather than doing nothing;
//!  * a unique index and a conflict-ignore that really make a double-delivered
//!    write land **once** (grammar 9.4, PRD 5.8) — which is the clause resolved
//!    q63 moved from the caller's promise to the backend's;
//!  * a placeholder dialect — `?` against `$1` — and an upsert spelled two ways.
//!
//! # How it runs, and what it does when a server is absent
//!
//! Every backend is **built and type-checked** on every run, server or no
//! server: neither needs one, and the MySQL store arm has no golden carrying it,
//! so this is the only place `src/codegen/js/stores-mysql.ts` meets `tsc` on a
//! machine with no database.
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

/// The variable the emitted project's store binding names its address in.
///
/// Named here rather than reused from the ambient environment: the emitted
/// `src/graph.ts` carries the *name* the deploy file wrote, so the runner's
/// process has to hold that name — and holding the suite's own variable under it
/// keeps the deploy file honest about being an `${ENV}` reference rather than an
/// address.
const STORE_URL: &str = "AGENT_COMPOSE_STORE_URL";

/// …and the second name for the same server, which is how the runner's
/// interleaved-writer case becomes two connections rather than one.
///
/// `src/stores.ts` caches a dialled connection per provider and variable name,
/// so two bindings naming one variable share a socket by design — which is right
/// for a graph and useless for a case about two writers.
const OTHER_STORE_URL: &str = "AGENT_COMPOSE_STORE_URL_B";

/// Build one project whose store binds `provider`, and answer where it landed.
///
/// The area is a directory of its own per provider, because libtest runs the
/// tests of one binary in **parallel threads**: two tests both building under
/// one path would be one `remove_dir_all` racing the other's `bun`.
fn built(root: &Path, provider: &str) -> PathBuf {
    let source = root.join("projects").join("store-sources").join(provider);
    let _ = fs::remove_dir_all(&source);
    fs::create_dir_all(source.join("deploy")).expect("the scratch area is writable");
    fs::write(source.join("main.yml"), COMPOSITION).expect("the entrypoint is writable");
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
    let destination = root.join("projects").join("store").join(provider);
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
    let checked = bun()
        .args(["run", "typecheck"])
        .current_dir(&project)
        .output()
        .expect("bun runs");
    assert!(
        checked.status.success(),
        "the `{}` store project does not type-check:\n{}\n{}",
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
        // Two names, one server. See [`OTHER_STORE_URL`].
        command.env(STORE_URL, &address);
        command.env(OTHER_STORE_URL, &address);
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
    // Multi-writer by design, and per key last write wins.
    "a_second_writer_reads_what_the_first_wrote",
    "two_writers_share_one_store_and_the_last_write_wins",
    // The DDL is idempotent and what was written is still there.
    "a_second_open_finds_what_the_first_wrote",
    // PRD 5.8's replay discipline, unchanged by the arms underneath it.
    "a_replayed_read_answers_out_of_the_record",
    "a_replayed_write_is_not_applied_again",
];

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
