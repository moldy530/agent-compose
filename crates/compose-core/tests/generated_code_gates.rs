//! The generated-code checks of CLAUDE.md's *Validation strategy*, run against
//! the **real** pinned JavaScript toolchain — under **Bun**, which PRD §9.18
//! makes the default runtime and package manager of every emitted project.
//!
//! Twenty-two gates. The first four are in increasing strength, each one
//! existing because the one above it passes on code the one below it catches;
//! the fifth is about a construct whose guarantees are only observable from
//! inside the runtime; the next two are about the schemas rather than the graph;
//! the eighth is about a composition that has no generated project at all; the
//! next four are about what a binding does on the wire, which no amount of
//! type-checking or graph construction reaches; the three after those are about
//! the *other* runtime — the Node fallback the same decision keeps supported;
//! the sixteenth is about the storage underneath a `store.*`; the seventeenth is
//! about the argument parser every launch of an emitted project goes through;
//! the eighteenth is about what a failure *says*, which is the one subject here
//! that is a published surface rather than a behaviour; the three after that are
//! about a `human` pause — the board a run holds one on, the terminal it answers
//! one at, and the process's own standard input that terminal really is; and the
//! last is about the two built-in tools a model drives, read from inside one
//! call of them:
//!
//! 1. **`bun run typecheck`** — every golden project type-checks under its own
//!    strict `tsconfig.json`, against installed `@langchain/langgraph`,
//!    `@langchain/core` and `zod`. Not a stub, not a shim: the versions
//!    `compose_core::codegen::project::PINS` names, downloaded and resolved. The
//!    command is the emitted `README.md`'s own, run through the emitted
//!    `scripts.typecheck`, so a manifest that stopped declaring it fails here.
//!    It has a **negative** half, numbered `1b` because it is the same command
//!    read the other way: a staged golden whose emitted schema is moved under an
//!    authored `module:` implementation must **fail** to compile, naming the
//!    field and the file. Everything else here is positive, and a contract that
//!    stopped constraining would pass all of it — which is the one way PRD
//!    resolved q48's "`tsc` is the merge tool" could quietly stop being true.
//!    And a third half, `1c`, over the authored code **nobody wrote**: every
//!    `module:` implementation in the corpus is hand-authored, so the stub
//!    `build` scaffolds for an absent one is checked by nothing — and it is
//!    written once and never rewritten, so a stub that stopped compiling is a
//!    file the author already has and the compiler declines to replace. It
//!    builds `tests/projects/scaffolded-modules` the way the command does —
//!    scaffold, read back, emit — and type-checks the result.
//! 2. **Construction** — Bun runs the emitted TypeScript and builds a
//!    `StateGraph` over the state model. A channel spec LangGraph refuses is a
//!    green `tsc` and a runtime failure, so type-checking alone would not catch
//!    it. The channel names come back and are compared against the composition's
//!    own `state:` section, so a channel dropped between the IR and the emitted
//!    module fails here.
//! 3. **Reduction** — the same graph is *invoked*. Names say a channel exists;
//!    only running it says the reducer appends rather than prepends and that a
//!    `default:` became an initial value. A wrong reducer type-checks, constructs,
//!    and would be committed as a correct golden, so [`REDUCTIONS`] states what
//!    every golden's state holds after two rounds of writes.
//! 4. **The environment check** — `bun src/index.ts`, once with the
//!    composition's `${ENV}` variables removed and once with them supplied. The
//!    emitted `README.md` says loading the project is what checks its
//!    environment (PRD 5.9); a `readEnvironment` that was declared and never
//!    called would pass every gate above and make that sentence false.
//! 5. **The fan-out** — `map` dispatch, driven directly against a golden's own
//!    `src/runtime.ts`. Nine of grammar 8.6's guarantees are invisible from
//!    outside: how many instances were in flight at once — with and without a
//!    detached route beside them — which item a `fail` names when two of them
//!    fail, whether the join returned before a detached delivery did, what that
//!    delivery handed its sink, that an exhausted item retry resolves as `fail`
//!    does, how many attempts an item *made* when the node's deadline cut a
//!    backoff short, what a fan-out that failed still records, that a batch
//!    survives the **channel** its reducer belongs to and not only the reducer,
//!    and that the ordering holds under a completion order that is the reverse
//!    of the source's. A happy-path run produces the same outputs with every one
//!    of them broken. It also decides what a node's own `timeout:` does to a
//!    **subgraph** (grammar 9.2) — the one activity a deadline can stop rather
//!    than merely stop waiting for.
//! 6. **Schema-lowering agreement** — the corpus under
//!    `tests/fixtures/schema-lowering/` is validated twice: against the JSON
//!    Schema this compiler lowers to (Rust, the `jsonschema` crate) and against
//!    the Zod the same module emits (Bun here, and Node in gate 15, because that
//!    column's verdicts are a regex engine's). Grammar 3.8 is one table with two
//!    columns and this is what keeps them from drifting apart. Each document
//!    carries the verdict it *should* get, so two implementations agreeing on a
//!    wrong answer is still a failure — and where the two columns cannot agree,
//!    the document carries **both** verdicts and names one of [`DIVERGENCES`],
//!    so a difference is something a reader signed off on rather than something
//!    a thin corpus failed to notice.
//! 7. **What a provider would be handed** — `@langchain/core`'s own converter is
//!    run over the emitted schemas, because `withStructuredOutput` sends JSON
//!    Schema rather than the Zod it was given, and the conversion drops every
//!    check spelled `.refine`. That is why PRD 9.16 sends the compiler's own
//!    lowering instead, and this gate measures the road not taken — see
//!    `codegen::schema`'s *What a provider is handed*. Gate 6 is what makes the
//!    two columns agree; this is what says why a model is shown the one it is.
//! 8. **The refusal's evidence** — `compose_core::codegen::diagnostics` refuses to
//!    build a composition whose `state:` names a channel after a property every
//!    JavaScript object carries (`constructor`). That refusal is an
//!    over-refusal until something shows the runtime really cannot take the
//!    name, and no golden can show it, because the compiler will not emit one.
//!    So this gate builds the channel table itself and makes LangGraph fail on
//!    it — and asks the runtime for `Object.getOwnPropertyNames(
//!    Object.prototype)`, so the Rust-side list is checked against the object
//!    model rather than against a memory of it. Gate 13 asks Node the same
//!    question, because the list is the *engine's* and a refusal that held under
//!    one runtime and not the other would be a compiler constant that is wrong
//!    for half its users.
//! 9. **What a raw binding binds** — the single string-typed property of a
//!    `tool.*` takes *trimmed* raw stdout from an `exec:` implementation and the
//!    raw response text from an `http:` one, which is grammar 6.1 stating one
//!    exception once per surface with one word different between them. Both are
//!    driven out of a golden's own runtime, over a payload with whitespace at
//!    either end, so which reading this compiler took is a committed fact rather
//!    than an accident of a shared decoder.
//! 10. **A command that never reads its input** — grammar 8.2 writes a scalar
//!     `input:` to the child's stdin, and `printf` exits without draining it.
//!     The EPIPE that follows arrives as an `error` *event*, outside the promise
//!     the node's own error policy is built on, so an unhandled one aborts the
//!     whole process rather than failing the node. Nothing about a returned
//!     value is wrong there — no value is returned — which is why it is a gate
//!     and not an assertion.
//! 11. **A declared `Content-Type`** — header names are case-insensitive
//!     (grammar 6.1) and `fetch` composes its `Headers` by appending, so a
//!     binding's own media type would ride out beside the runtime's instead of
//!     replacing it. The server here is loopback and reports what it received.
//! 12. **A bound input object on a `GET`** — the other half of the same
//!     sentence: without `query:`/`body:`, the object goes out as query
//!     parameters rather than as a body. Which slot codegen fills is a golden's
//!     to commit; this is what the runtime does with what it was handed.
//! 13. **The Node fallback** — the whole of what PRD §9.18 promises a reader
//!     without Bun, done the way the emitted `README.md` says to do it: `npm ci`
//!     from the committed `package-lock.json`, `tsc --noEmit`, graph
//!     construction, a real invocation of the state model, and
//!     `node src/index.ts`. It also re-runs the three runners behind gates 9 to
//!     12, because those four verdicts are the *runtime's* rather than the
//!     emitted code's — an EPIPE's delivery, a `Headers` composition, a query
//!     string's spelling — and it asserts them with the very functions those
//!     gates use, so the two runtimes cannot come to different answers unnoticed.
//!     One golden, because what is in question is the runtime rather than any
//!     composition — every other gate above is what says the emitted code is
//!     right, and this is what says the second supported runtime can still run
//!     it. The `node` it finds is checked against the `engines.node` floor first:
//!     an older one cannot run a `.ts` file at all, and saying so is more use
//!     than a failure about a file extension.
//! 14. **No Bun-only API** — the same promise, statically and over the whole
//!     corpus. Gate 13 runs one golden, so a Bun-only call on a path that golden
//!     never takes would survive it; this reads every emitted module instead and
//!     refuses a `Bun` global, a `bun:` specifier, and any import that is not
//!     relative, a `node:` builtin, or one of the pinned packages — in every
//!     spelling an import can be written, the bare side-effect form included. A
//!     whitelist rather than a blacklist, so the next non-portable dependency
//!     fails too without anyone having thought of it first.
//! 15. **Both shared corpora, under the other engine** — gates 6 and the
//!     acceptance suite's CEL check answer their corpora with a *JavaScript*
//!     column, and what that column answers with belongs to the engine: an
//!     emitted `format:` is a `RegExp`, an emitted `max_length` is a code-point
//!     count, and `src/cel.ts` is `BigInt` arithmetic and `RegExp` matching
//!     throughout. So the schema corpus's Zod column and the whole CEL corpus are
//!     answered under Node too, with the same assertion code — otherwise a
//!     JavaScriptCore-only reading of a regex would decide a router or a parse
//!     differently for every reader on the fallback while every gate stayed
//!     green. Gate 13 loads, constructs, reduces and launches a golden under
//!     Node; it never validates a document or evaluates a guard.
//! 16. **The store backends** — `src/stores.ts`, driven directly. PRD 5.8's
//!     zero-infra guarantee is a promise about behaviour a golden diff cannot
//!     show: which partition a `scope:` addresses, whether a write's idempotency
//!     key is honoured, what a `list` answers a prefix with, and whether a key
//!     that would climb out of a directory becomes a file name that cannot. A
//!     composition using a store emits the same bytes with every one of those
//!     wrong. Gate 13 runs the same runner under Node, because a WebAssembly
//!     SQLite over `node:fs` is exactly the dependency that could answer the two
//!     engines differently.
//! 17. **The project's own command line** — `bun src/index.ts run … --fromat
//!     json` and `serve --prot 8787`: a flag neither verb declares, on the
//!     surface PRD 5.12's eject path leaves as the *only* way to launch the
//!     project. `agent-compose run` never sends one — clap refuses it first — so
//!     nothing else in the suite reaches this parser, and an option it accepted
//!     and never read would be a caller asking for the JSON record, getting
//!     human output, and being told `0`. D50's rule, asserted where the compiler
//!     is no longer standing in front of it.
//! 18. **What a failure the platform worded says** — `docs/trace.md` §11.1
//!     promises that no resolved `${ENV}` value appears in a trace, and §11.2
//!     names the three failures where one would have: a command the OS refused
//!     to spawn, an `http:` `url` that is not a URL once its references resolve,
//!     and a provider whose resolved `base_url:` `fetch` cannot parse. Each of
//!     those messages is the *engine's*, and each embeds the string it could not
//!     use, so each is caught and restated. A gate rather than a source check —
//!     `tests/trace_format_inventory.rs` holds that half — because the wording
//!     differs between the two supported runtimes: Bun quotes what `new URL`
//!     could not parse and Node says only `Invalid URL`, while Node quotes what
//!     `fetch` could not parse and Bun does not. Gate 13 asks Node the same
//!     question for exactly that reason.
//! 19. **The `human` wait board** — the pauses a run is holding, driven directly.
//!     Four of grammar 8.7's guarantees are invisible from a served app: that a
//!     pause belongs to the node whose instance path is a prefix of its own —
//!     which is what holds an enclosing budget still (Decision D102) — that a
//!     pause the run abandoned leaves the board rather than being published as a
//!     question and answered into nothing, that a released wait's expiry timer
//!     is cleared, and that an expiry and an interrupt are answered ahead of an
//!     *explicit* `on_error:` on the same node. The first three are invisible
//!     because the case that breaks them is a task nobody is awaiting; the last
//!     because every composition that can reach an expiry resolves
//!     `on_error: fail`, where routing the `on_timeout:` fallback and absorbing
//!     the expiry look alike. The timer claim has no observable consequence at
//!     all: an `unref`ed timer keeps nothing alive, so the runner counts the
//!     global `setTimeout`/`clearTimeout` calls instead.
//! 20. **The terminal a pause is answered at** — `src/cli.ts`'s prompt loop,
//!     driven over a stream this suite feeds. The acceptance suite drives the
//!     real `agent-compose run`, which is where "the command behaves" is
//!     decided; this is here because the loop is a **reader over
//!     `process.stdin`** and this project supports two engines. A `data` event's
//!     chunking, a stream's `end`, and what `setEncoding` does to a chunk are
//!     Bun's and Node's separately, so a line reader that answered them
//!     differently would leave one of the two supported readers with a `run`
//!     that hangs on a question it had printed. Six claims: what a prompt shows,
//!     that a malformed line and a refused answer both re-prompt without
//!     consuming the wait, that two pauses are asked one at a time in wait-id
//!     order — and that a pause opening *under* a question on the screen is
//!     asked after it, which is the boundary of that order — that an expiry
//!     withdraws the question and the loop moves on, that standard input ending
//!     turns every pause into the interrupt a run with no surface raises,
//!     whether it ends under a question or between two, and that a last line
//!     with no newline on it is still an answer. Gate 13 asks the same of Node.
//! 21. **The standard input that terminal really is** — the same loop over the
//!     process's own `process.stdin` rather than over a stream this suite feeds.
//!     Gate 20's stream is what lets it schedule a pause to the instant; what it
//!     cannot ask is what an operating-system pipe wired into the runtime's
//!     event loop does. Three claims, each the engine's: that a `data` listener
//!     attached at the **first question** — never before, so a run with no
//!     `human` node never drains a stream it was not given — still sees what
//!     arrived ahead of it, that `end` fires on a pipe whose EOF is older than
//!     that listener, and that letting go of the stream lets the **process
//!     exit**. The last is why the gate holds the pipe open from its own side
//!     and the runner arms a timer of its own: a run that printed its answer and
//!     then sat there is the one failure a passing test cannot tell from a slow
//!     one. Gate 13 asks both cases of Node.
//! 22. **What a built-in call stays inside** — `builtin.bash` and
//!     `builtin.files`, driven out of a golden's own runtime. The acceptance
//!     suite drives both through a model loop, which is where the wire, the
//!     trace records and Decision D119's bounces are settled; what a loop
//!     cannot show is the inside of a call, and that is where every bound PRD
//!     resolved q54 puts on these two lives. Eight of them: that a path
//!     climbing out of the workspace — `..`, an absolute path, or a symlink
//!     whose own parents do not exist yet — leaves what is outside
//!     **untouched** rather than merely drawing a complaint, that a file larger
//!     than the runtime reads is viewed from the front and edited not at all,
//!     that one shell is held across the calls of a node activity and across no
//!     more than that, that a `restart` ends that session without swallowing
//!     the command sent beside it, that a command printing more than a tool
//!     result can hold still comes back with its status, that a scrubbed child
//!     sees the declared variables and no others, that a command's deadline
//!     answers the *model* rather than failing the node, and that a defaulted
//!     workspace is one directory per execution that goes when the execution
//!     settles. Break any one of them and the call still answers something that
//!     reads correctly from outside, which is why they are driven from within
//!     the runtime rather than read off a transcript.
//!
//! # The toolchain fixture
//!
//! `tests/toolchain/` holds a `package.json` pinning exactly what the emitter
//! pins, and a lockfile per supported runtime: `bun.lock`, which
//! `bun install --frozen-lockfile` installs once per run for every gate but one,
//! and `package-lock.json`, which gate 13 installs with `npm ci`. One install
//! serves every golden because the emitted dependency set is a compiler constant
//! rather than a per-project one — two projects built by one compiler release
//! declare identical versions, so installing once and type-checking each against
//! that install is the same check at a third of the network cost.
//! `the_toolchain_fixture_pins_what_the_emitter_pins` is what keeps the fixture,
//! both lockfiles, and the emitter from disagreeing.
//!
//! Each golden is **copied** into `tests/toolchain/projects/<name>/` rather than
//! checked in place, so `node_modules/` resolution finds the shared install by
//! walking up, and the committed goldens stay exactly the bytes the emitter
//! wrote. The copies gates 13 and 15 run under Node go under
//! `tests/toolchain/node-fallback/projects/` instead, beside an npm-installed
//! `node_modules/` of their own: resolution takes the nearest one walking up, so
//! the Node gates reach what npm installed and never what Bun did. The
//! **runners** those two gates spawn are staged into `node-fallback/` for the
//! same reason — a runner's own bare imports resolve from the directory it is
//! spawned out of, which for a runner left in `tests/toolchain/` is the tree
//! `bun install` writes. [`every_runner_node_spawns_resolves_the_npm_install`] is
//! what holds that.
//!
//! # When a runtime is missing
//!
//! **In CI these gates fail; on a developer machine they skip** — for Bun and for
//! Node alike. The rule, the search order that finds `bun`, and why both halves
//! are required in CI are in `support/toolchain.rs`.

#[path = "support/goldens.rs"]
mod goldens;
#[path = "support/toolchain.rs"]
mod toolchain;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use goldens::{GOLDENS, Golden, artifact, emitted, files_under, golden, goldens_root, repository};
use serde_json::{Value, json};
use toolchain::{bun, installed, required, runner, runs};

/// The golden gate 13 runs under Node.
///
/// The richest emitted surface in the corpus: a fan-out, a subflow, stores, an
/// `http:` binding and an `exec:` one, so a Node incompatibility in any of the
/// runtime's wrappers has a module here that reaches it. Its `${ENV}` references
/// also make it the golden whose `node src/index.ts` is a real presence check
/// rather than a no-op load.
const NODE_FALLBACK_GOLDEN: &str = "triage-fanout";

/// The npm-installed toolchain gates 13 and 15 use, or `None` when Node is
/// absent and this is not CI.
///
/// A directory of its own beneath the fixture, holding a copy of the committed
/// manifest and `package-lock.json` and an npm-installed `node_modules/`. Two
/// reasons it is not the fixture directory itself: `bun install` and `npm ci`
/// would take turns rewriting one `node_modules/`, and the point of the gate is
/// that the code Node runs was resolved by **npm** from the lockfile a reader
/// without Bun would use. Module resolution takes the nearest `node_modules/`
/// walking up, so a project staged inside this directory reaches this install and
/// never the Bun one above it.
///
/// The fixture's **runners** are staged here too, and for the same rule read one
/// level out: Node resolves a module's bare specifiers from that module's own
/// directory, not from the directory of whatever it went on to import. A runner
/// spawned out of `tests/toolchain/` would therefore take its own
/// `@langchain/langgraph` from the tree `bun install` writes *there* — so the
/// gate whose subject is the npm install would be reducing a state model with a
/// `StateGraph` that came from Bun's, and on a machine with npm and no Bun it
/// would not resolve at all. Copying the runners in puts their imports on the
/// same walk-up as the staged project's, which is what makes "never what Bun
/// did" true of the whole process rather than of the project alone.
fn node_fallback() -> Option<&'static Path> {
    static FALLBACK: OnceLock<Option<PathBuf>> = OnceLock::new();
    FALLBACK
        .get_or_init(|| {
            if let Some(blocker) = node_fallback_blocker() {
                assert!(
                    !required(),
                    "the Node fallback cannot be checked here: {blocker}. PRD §9.18 keeps \
                     Node {engine} a supported fallback for every generated project, and gates \
                     13 and 15 are the only things that check it. CI installs it; see \
                     .github/workflows/ci.yml.",
                    engine = compose_core::codegen::project::NODE_ENGINE,
                );
                eprintln!(
                    "warning: skipping the Node-fallback gates — {blocker}. A Node satisfying \
                     `{engine}` is required in CI (`CI` is set there) and this run is not CI.",
                    engine = compose_core::codegen::project::NODE_ENGINE,
                );
                return None;
            }
            let source = toolchain::root();
            let root = source.join("node-fallback");
            fs::create_dir_all(&root).expect("the scratch area is writable");
            for name in ["package.json", "package-lock.json"]
                .into_iter()
                .map(str::to_string)
                .chain(runners(&source))
            {
                fs::copy(source.join(&name), root.join(&name))
                    .expect("the committed fixture is readable");
            }
            let install = Command::new("npm")
                .args(["ci", "--no-audit", "--no-fund"])
                .current_dir(&root)
                .output()
                .expect("npm runs");
            assert!(
                install.status.success(),
                "the pinned toolchain did not install under npm:\n{}\n{}",
                String::from_utf8_lossy(&install.stdout),
                String::from_utf8_lossy(&install.stderr),
            );
            Some(root)
        })
        .as_deref()
}

/// The runner scripts a directory holds, by file name, sorted.
///
/// Read from the directory rather than listed here, so a runner added to the
/// fixture is staged into the npm install without anyone having remembered to
/// come back — which is the omission
/// [`every_runner_node_spawns_resolves_the_npm_install`] would otherwise be
/// catching after the fact.
fn runners(directory: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(directory).expect("the directory is readable") {
        let name = entry.expect("the directory is readable").file_name();
        let name = name.to_str().expect("the fixture's file names are UTF-8");
        if name.ends_with(".mjs") {
            names.push(name.to_string());
        }
    }
    names.sort();
    names
}

/// Every runner this file names is a runner the fixture still holds.
///
/// The module header above is written as this suite's own index — a numbered
/// entry per gate, naming the subject it drives and the runner it drives it
/// out of — and prose is the one part of a test file the compiler never reads.
/// A gate that changed subject took its runner's name with it once already: the
/// entry for gate 22 went on describing a `builtin.list` and a listing runner
/// that the grammar and the fixture had both stopped holding, so a reader
/// navigating by the index was sent to a test that does not exist, by the name
/// of a file nothing on disk answers to. Nothing here can check that an entry
/// still *describes* its gate — but a deleted runner is a **name**, and a name
/// is checkable: every `*.mjs` this file mentions, in a `runner(…)` call or in a
/// doc comment or in a prose aside, has to be a file [`runners`] finds in
/// `tests/toolchain/`. Which makes the deletion of a runner the moment the prose
/// naming it fails, rather than the moment someone reads it.
///
/// One direction only: the fixture also holds runners other suites spawn, and
/// this file has no business naming those.
#[test]
fn this_suite_names_only_runners_the_fixture_holds() {
    const SOURCE: &str = include_str!("generated_code_gates.rs");

    let held: BTreeSet<String> = runners(&toolchain::root()).into_iter().collect();
    assert!(
        held.contains("builtin-tools.mjs"),
        "the fixture holds none of the runners this file drives, so the scan below asserts \
         nothing — the fixture moved: {held:?}"
    );

    let bytes = SOURCE.as_bytes();
    let mut named: BTreeSet<&str> = BTreeSet::new();
    for (end, _) in SOURCE.match_indices(".mjs") {
        let start = bytes[..end]
            .iter()
            .rposition(
                |byte| !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_'),
            )
            .map_or(0, |before| before + 1);
        // `".mjs"` itself, as the extension [`runners`] filters on: an extension
        // is not a runner, and no file is named for one.
        if start == end {
            continue;
        }
        named.insert(&SOURCE[start..end + ".mjs".len()]);
    }
    assert!(
        named.len() > 10,
        "the scan found {} runner names in a file that spawns a dozen — it stopped reading \
         the shape a runner is named in: {named:?}",
        named.len()
    );

    let missing: Vec<&str> = named
        .iter()
        .copied()
        .filter(|name| !held.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "this file names {missing:?}, which `tests/toolchain/` does not hold — a runner was \
         renamed or deleted and the prose that names it was left standing"
    );
}

/// Why gate 13 cannot run here, or `None` when it can.
///
/// Presence is not the question — the **floor** is. `engines.node` in every
/// emitted manifest declares `compose_core::codegen::project::NODE_ENGINE`, and
/// that number is load-bearing: below 22.18 Node does not strip types, so
/// `node src/index.ts` fails on the first `.ts` it is handed. A gate that took
/// any `node` on `PATH` would turn a developer machine whose default Node is
/// older into a red suite reporting `ERR_UNKNOWN_FILE_EXTENSION` — a message
/// about a file extension, for a machine that simply is not the one the fallback
/// is promised to. So the version is read and compared, and the answer is the
/// same rule the rest of the toolchain follows: in CI a failure, locally a skip,
/// naming what was found beside what is required.
fn node_fallback_blocker() -> Option<String> {
    let floor = version(
        compose_core::codegen::project::NODE_ENGINE
            .strip_prefix(">=")
            .expect("`NODE_ENGINE` is a `>=` floor"),
    )
    .expect("`NODE_ENGINE` names a version");

    let reported = Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    let Some(reported) = reported else {
        return Some("`node` does not answer `--version` on PATH".to_string());
    };
    let Some(found) = version(&reported) else {
        return Some(format!(
            "`node --version` said `{reported}`, which is not a version"
        ));
    };
    if found < floor {
        return Some(format!(
            "the `node` on PATH is `{reported}`, below the `{}` floor every emitted \
             `package.json` declares — it does not strip types, so `node src/index.ts` cannot run \
             at all",
            compose_core::codegen::project::NODE_ENGINE,
        ));
    }
    (!runs("npm")).then(|| "`npm` does not answer `--version` on PATH".to_string())
}

/// `major.minor.patch` from a version string, ordered as a tuple.
///
/// It reads two dialects: `v22.18.0` from `node --version` and `22.18.0` out of
/// the `>=` floor. A component that is absent is zero — `>=23` is 23.0.0 — and a
/// prerelease (`v25.0.0-nightly…`) is taken at its release number, because a
/// nightly of a major above the floor is above the floor. A component that is
/// present and is not a number is refused rather than guessed at.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    fn number(part: &str) -> Option<u64> {
        part.split(|character: char| !character.is_ascii_digit())
            .next()
            .filter(|digits| !digits.is_empty())?
            .parse()
            .ok()
    }
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = number(parts.next()?)?;
    let minor = parts.next().map_or(Some(0), number)?;
    let patch = parts.next().map_or(Some(0), number)?;
    Some((major, minor, patch))
}

/// The floor gate 13 holds `node` to is a number, and it is the emitter's own.
///
/// `NODE_ENGINE` is what every generated `package.json` declares and what the
/// emitted README tells a reader without Bun to install; the comparison that
/// decides whether this machine can check that promise has to be over the same
/// number, read as a number. A string compare would put `v9` above `v22.18.0`
/// and a presence check — which is what this gate did — would put *every* Node
/// above it, including the ones that cannot run a `.ts` file at all.
#[test]
fn the_node_fallback_floor_is_the_one_the_emitted_manifest_declares() {
    let engine = compose_core::codegen::project::NODE_ENGINE;
    let floor =
        version(engine.strip_prefix(">=").expect("a `>=` floor")).expect("the floor is a version");
    assert_eq!(
        floor,
        (22, 18, 0),
        "`{engine}` is not the floor gate 13 reads"
    );

    for above in ["v22.18.0", "v22.22.2", "v24.0.1", "v25.0.0-nightly20260101"] {
        assert!(
            version(above).expect("a version") >= floor,
            "`{above}` strips types and would be refused"
        );
    }
    for below in ["v22.17.1", "v20.20.2", "v9.11.2"] {
        assert!(
            version(below).expect("a version") < floor,
            "`{below}` does not strip types, so `node src/index.ts` cannot run there"
        );
    }
    assert_eq!(version("not-a-version"), None);
}

/// Copy one golden into the toolchain's scratch area and answer where it landed.
///
/// `purpose` names the gate the copy is for, because cargo runs the tests in one
/// binary on parallel threads and two of them re-staging one directory would
/// delete files out from under each other.
fn staged(golden: &Golden, root: &Path, purpose: &str) -> PathBuf {
    let destination = root.join("projects").join(purpose).join(golden.directory);
    let _ = fs::remove_dir_all(&destination);
    let source = goldens_root().join(golden.directory);
    for relative in files_under(&source) {
        let target = destination.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(target.parent().expect("a staged path has a parent"))
            .expect("the scratch area is writable");
        fs::copy(source.join(&relative), &target).expect("a golden file is copyable");
    }
    destination
}

/// Gate 1: every golden project type-checks against the installed pins.
///
/// `bun run typecheck` rather than a path to `tsc`, because that is the command
/// the emitted `README.md` gives a reader: it goes through the project's own
/// `scripts.typecheck`, resolves `tsc` from the shared install by walking up, and
/// runs it under Bun — so a manifest that stopped declaring the script, and a
/// pinned TypeScript that stopped running under the default runtime, both fail
/// here rather than in a reader's terminal.
#[test]
fn every_generated_project_type_checks_under_the_pinned_toolchain() {
    let Some(root) = installed() else {
        return;
    };
    let tsc = root.join("node_modules/.bin/tsc");
    assert!(
        tsc.is_file(),
        "the pinned TypeScript did not install: {}",
        tsc.display()
    );

    for golden in GOLDENS {
        let project = staged(golden, root, "typecheck");
        let output = bun()
            .args(["run", "typecheck"])
            .current_dir(&project)
            .output()
            .expect("bun runs");
        assert!(
            output.status.success(),
            "`{}` does not type-check:\n{}\n{}",
            golden.directory,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// Gate 1b: the contract a `module:` binding is held to **bites**.
///
/// Every other type gate here is positive — the committed golden compiles — and
/// a contract that stopped constraining would pass all of them. That is the one
/// direction worth testing on purpose, because the whole of PRD resolved q48's
/// answer to marker comments is "`tsc` is the merge tool": a `z.infer` that
/// widened to `any`, an emitted signature that took `input: unknown`, or a
/// schema form that lowered to a Zod object accepting anything would all leave
/// `src/tools/stamp.ts` compiling against a schema it no longer matches, and the
/// promise in four documents and a PRD entry would be untrue with nothing
/// failing.
///
/// So: a staged golden whose **emitted schema** is moved under an authored file
/// that was not, and the type gate is required to *fail*, naming the field and
/// the file. The mutation is the smallest one a real schema change makes — a
/// renamed field — and it is made in the emitted tree rather than in the
/// composition, because what is under test is the contract's grip on the
/// authored half rather than the emitter's own output.
#[test]
fn a_schema_the_authored_module_no_longer_matches_fails_the_type_gate() {
    let Some(root) = installed() else {
        return;
    };
    let golden = GOLDENS
        .iter()
        .find(|golden| golden.directory == "placed-nodes")
        .expect("the mesh golden is the one with a `module:` binding");
    let project = staged(golden, root, "typecheck-stale");

    let schemas = project.join("src/schemas.ts");
    let before = fs::read_to_string(&schemas).expect("the emitted schemas are readable");
    let declared = "export const toolStampInput = z.object({\n  path: z.string(),\n}).strict();";
    assert!(
        before.contains(declared),
        "the fixture's premise moved; `toolStampInput` is not what this gate mutates:\n{before}"
    );
    let after = before.replace(
        declared,
        "export const toolStampInput = z.object({\n  target: z.string(),\n}).strict();",
    );
    fs::write(&schemas, &after).expect("the staged copy is writable");

    let output = bun()
        .args(["run", "typecheck"])
        .current_dir(&project)
        .output()
        .expect("bun runs");
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !output.status.success(),
        "a renamed schema field left the authored module compiling, so the contract does not \
         hold it to anything:\n{report}"
    );
    assert!(
        report.contains("src/tools/stamp.ts"),
        "the failure is in the file that has to change, and says so:\n{report}"
    );
    assert!(
        report.contains("'path'"),
        "…naming the field that moved:\n{report}"
    );
}

/// The composition whose whole authored half is scaffolded, and the modules it
/// binds.
///
/// Committed with **no** `src/` beside it, which is the fixture's premise: the
/// only implementations that exist are the ones `build` writes.
const SCAFFOLDED: &str = "crates/compose-core/tests/projects/scaffolded-modules";
const SCAFFOLDED_MODULES: &[&str] = &[
    "src/tools/sealed.ts",
    "src/tools/shape.ts",
    "src/tools/sign.ts",
];

/// Gate 1c: the stub `build` writes for an absent `module:` binding compiles.
///
/// Gate 1 type-checks five golden projects and gate 1b proves the contract
/// constrains them — and every authored file in both is one a **person** wrote.
/// The other half of PRD resolved q48 is the half nobody types: `build`
/// scaffolds an absent implementation once — the contract import, the doc
/// comment, the throwing body — and then never writes that file again, so a stub
/// that stopped compiling is a file the author already has and the compiler
/// declines to replace. It would pass every gate here and fail in a terminal.
///
/// So this one runs the command's own order over a composition with **no**
/// authored half at all: scaffold what is missing into the project, read it
/// back, emit, and hand the result to the same `bun run typecheck` gate 1 uses.
/// Three tools, because a stub's shape varies with its binding — an `env:` that
/// is declared and one that is not, a description carrying a `*/`, and schemas
/// reaching past the scalars — and the emitted contract has to name a type for
/// each of them.
#[test]
fn a_scaffolded_module_implementation_type_checks_under_the_pinned_toolchain() {
    let Some(root) = installed() else {
        return;
    };
    let source = repository().join(SCAFFOLDED);
    assert_eq!(
        files_under(&source),
        ["main.yml"],
        "the fixture's premise is that nothing authored is committed beside it"
    );

    // The project the author edits, and the output directory beside it — the two
    // trees `agent-compose build` writes into, in the same relation.
    let project = root.join("projects/typecheck-scaffold/scaffolded-modules");
    let _ = fs::remove_dir_all(&project);
    fs::create_dir_all(&project).expect("the scratch area is writable");
    fs::copy(source.join("main.yml"), project.join("main.yml")).expect("the entrypoint copies");

    let resolution = compose_core::resolve_with_target(project.join("main.yml"), "local");
    assert!(
        resolution.diagnostics.is_empty(),
        "the fixture does not resolve: {:#?}",
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    assert!(
        compose_core::check(&ir).is_empty(),
        "the fixture does not validate: {:#?}",
        compose_core::check(&ir)
    );
    // The premise, asserted rather than assumed: `validate` refuses this project
    // right now, once per binding, and names `build` as the repair.
    let missing = compose_core::check_modules(&ir, &project);
    assert_eq!(
        missing.len(),
        SCAFFOLDED_MODULES.len(),
        "every binding's file is absent before the scaffold: {missing:#?}"
    );

    let scaffolds = compose_core::codegen::authored::scaffolds(&ir);
    assert_eq!(
        scaffolds
            .iter()
            .map(|scaffold| scaffold.path.as_str())
            .collect::<Vec<_>>(),
        SCAFFOLDED_MODULES,
        "the fixture binds exactly the modules this gate is written for"
    );
    for scaffold in &scaffolds {
        write_into(&project, &scaffold.path, &scaffold.contents);
    }

    let authored = compose_core::Authored::read(&ir, &project)
        .expect("the scaffolds are there to be read back");
    let built = compose_core::emit(&ir, &authored);
    let out = project.join("build/local");
    for file in built.artifact() {
        write_into(&out, &file.path, &file.contents);
    }

    let output = bun()
        .args(["run", "typecheck"])
        .current_dir(&out)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "a scaffolded implementation does not type-check, so `build` writes a file its author \
         cannot compile and will never rewrite:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// Write one `/`-separated relative path under `root`, making its directories.
fn write_into(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
    fs::create_dir_all(path.parent().expect("a relative path has a parent"))
        .expect("the scratch area is writable");
    fs::write(&path, contents).expect("the scratch area is writable");
}

/// Gate 2: every golden project constructs its graph, and its state model holds
/// exactly the channels the composition declares plus the implicit history one.
#[test]
fn every_generated_project_constructs_its_state_model() {
    let Some(root) = installed() else {
        return;
    };

    for golden in GOLDENS {
        let project = staged(golden, root, "construct");
        let output = runner("state-channels.mjs")
            .arg(&project)
            .output()
            .expect("bun runs");
        assert!(
            output.status.success(),
            "`{}` does not construct its graph:\n{}",
            golden.directory,
            String::from_utf8_lossy(&output.stderr),
        );
        let channels: Vec<String> = serde_json::from_slice(&output.stdout)
            .expect("the runner prints the channel names as JSON");

        let ir = artifact(golden);
        let mut declared: Vec<String> = ir
            .state
            .as_ref()
            .map(|state| state.entries.keys().cloned().collect())
            .unwrap_or_default();
        // Two channels a composition never declares and every graph has:
        // grammar 10.4's implicit conversation history, and the compiler's own
        // `$run` — the flow input, the execution identity, the per-bounded-edge
        // counters grammar 7.4 puts in graph state, and the routing trace
        // (see `codegen::state`). Both are named here rather than filtered out,
        // so a third one appearing is this test's failure rather than nobody's.
        declared.push("messages".to_string());
        declared.push("$run".to_string());
        assert_eq!(
            channels, declared,
            "`{}`'s state model is not the composition's channel set",
            golden.directory
        );
    }
}

/// What one golden's state model must *do*: two rounds of writes, and the state
/// they leave behind.
///
/// Gate 2 compares channel names, which a wrong reducer and a missing initial
/// value both survive. This is the table that does not: every entry is a value
/// the composition's own `reduce:` policy and `default:` decide (grammar 7.6.4,
/// 10.1, 10.2), written out so that changing the emitted reducer changes a
/// verdict here rather than a golden nobody re-derives.
///
/// Every golden has a row. A composition with nothing reduced still pins its
/// defaults, and pins that an unreduced channel with no `default:` starts
/// **unset** rather than `null` (Decision D78) — which is what makes a later
/// read of it fail naming the channel.
struct Reduction {
    /// The golden this describes, by directory.
    golden: &'static str,
    /// One object per node, in the order the nodes run.
    writes: &'static str,
    /// The whole state at quiescence, `messages` included.
    expected: &'static str,
}

const REDUCTIONS: &[Reduction] = &[
    Reduction {
        golden: "every-schema-form",
        // `seen` is deliberately never written: its `default: { count: 0 }` is
        // the only thing that can produce it. Neither is `draft`, which has no
        // default and must therefore be absent rather than null.
        writes: r#"[
            { "notes": "a", "totals": { "fixed": 1 }, "latest": "one" },
            { "notes": "b", "totals": { "skipped": 2 }, "latest": "two" }
        ]"#,
        expected: r#"{
            "notes": ["a", "b"],
            "totals": { "fixed": 1, "skipped": 2 },
            "latest": "two",
            "seen": { "count": 0 },
            "round": 1,
            "mood": "calm",
            "urgent": false,
            "messages": []
        }"#,
    },
    Reduction {
        golden: "review-loop",
        writes: r#"[
            { "draft": "first" },
            { "draft": "second", "feedback": "tighten it" }
        ]"#,
        expected: r#"{
            "draft": "second",
            "feedback": "tighten it",
            "messages": []
        }"#,
    },
    Reduction {
        golden: "triage-fanout",
        writes: r#"[
            { "patches": "one", "matches": ["a", "b"], "report_normalized": "r" },
            { "patches": "two" }
        ]"#,
        expected: r#"{
            "patches": ["one", "two"],
            "matches": ["a", "b"],
            "report_normalized": "r",
            "human_decision": "approve",
            "summary": "",
            "messages": []
        }"#,
    },
    Reduction {
        // The deploy layer forks per target and the state model does not, so the
        // same values are the answer under `staging` — which is the claim, not a
        // duplicate.
        golden: "triage-fanout-staging",
        writes: r#"[
            { "patches": "one", "matches": ["a", "b"], "report_normalized": "r" },
            { "patches": "two" }
        ]"#,
        expected: r#"{
            "patches": ["one", "two"],
            "matches": ["a", "b"],
            "report_normalized": "r",
            "human_decision": "approve",
            "summary": "",
            "messages": []
        }"#,
    },
    Reduction {
        // The mesh golden's state model, which is the graph's and nothing to do
        // with where its nodes run: `signature` and `ticket` are plain
        // `last_wins` defaults written by two nodes in two *processes*, and
        // `signatures` is the `append` channel a fan-out onto a placement fills.
        // What this pins is that a placed node's answer reaches a channel the
        // same way a local one's does — the seam is the activity, and grammar
        // 10.3's write map never learns about it.
        golden: "placed-nodes",
        writes: r#"[
            { "signature": "signed", "signatures": "one" },
            { "ticket": "notarized", "signatures": "two" }
        ]"#,
        expected: r#"{
            "approval": "",
            "countersignature": "",
            "signature": "signed",
            "ticket": "notarized",
            "signatures": ["one", "two"],
            "messages": []
        }"#,
    },
];

/// One case of the schema-lowering corpus.
#[derive(serde::Deserialize)]
struct Case {
    golden: String,
    surface: String,
    #[expect(dead_code, reason = "documentation for a reader of the corpus")]
    about: String,
    documents: Vec<Document>,
}

/// One document, and the verdict each lowering must reach on it.
///
/// `valid` is the verdict, and it is both columns' unless `zod` overrides it.
/// That shape is the point: agreement is the default a case does not have to say
/// anything about, and a difference is a **declaration** — it needs the other
/// verdict written down and a `divergence` naming one of [`DIVERGENCES`], so no
/// difference between grammar 3.8's two columns can exist without a reader
/// having agreed to it.
#[derive(serde::Deserialize)]
struct Document {
    #[expect(dead_code, reason = "documentation for a reader of the corpus")]
    why: String,
    valid: bool,
    #[serde(default)]
    zod: Option<bool>,
    #[serde(default)]
    divergence: Option<String>,
    document: Value,
    #[serde(default)]
    as_written: Option<String>,
}

impl Document {
    /// The verdict the emitted Zod must reach.
    fn zod_verdict(&self) -> bool {
        self.zod.unwrap_or(self.valid)
    }

    /// The exact JSON text the Zod column is handed.
    ///
    /// `serde_json` reads an object into a `BTreeMap`, so a document written in
    /// the corpus arrives here with its keys **sorted** and the order they were
    /// written in is gone. Object key order is not supposed to matter — JSON
    /// Schema compares instances — but "not supposed to" is the thing a corpus
    /// exists to check, and it cannot check it through a representation that
    /// has already normalized it. So a case that is about key order writes the
    /// document out a second time as `as_written`, which is passed through
    /// verbatim for `JSON.parse` to read in that order.
    ///
    /// # Panics
    ///
    /// Panics if `as_written` is not JSON, or is JSON denoting a different
    /// value than `document` — the two are one document and a case where they
    /// disagree would test something nobody wrote down.
    fn text(&self) -> String {
        let Some(written) = &self.as_written else {
            return serde_json::to_string(&self.document).expect("a document serializes");
        };
        let parsed: Value = serde_json::from_str(written)
            .unwrap_or_else(|error| panic!("`as_written` is not JSON: {written} ({error})"));
        assert_eq!(
            parsed, self.document,
            "`as_written` and `document` are the same document written twice"
        );
        written.clone()
    }
}

/// Every difference between the two columns that this compiler has decided to
/// keep, by the id a corpus document cites.
///
/// The list is `codegen::schema`'s own divergence table, transcribed: the module
/// argues each one and this decides whether the argument is still true. A
/// divergence with no document exercising it is as much a failure as a document
/// citing a divergence that is not here — the first is a claim nothing checks,
/// the second is a difference nobody agreed to.
const DIVERGENCES: &[&str] = &[
    "integer-beyond-the-safe-range",
    "multiple-of-under-a-scaled-tolerance",
    "display-name-is-not-an-addr-spec",
    "leap-second-away-from-midnight",
    "duration-is-iso-8601-not-rfc-3339",
    "punycode-payload-undecoded",
    "omitted-default-is-the-same-item",
    "dot-matches-a-code-unit",
    "dot-excludes-a-line-terminator",
    "word-boundary-is-unicode-aware",
];

fn corpus() -> Vec<Case> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema-lowering/cases.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not a corpus: {error}", path.display()))
}

/// Gate 2b: the emitted reducers and initial values do what the policies say.
///
/// The graph is real, the reducers are the emitted ones, and the comparison is
/// over the whole state object — so a channel that reduced correctly and one that
/// quietly acquired a value are both visible. `messages` is in the expectation
/// for the same reason: the implicit channel of grammar 10.4 starts empty, and
/// nothing here writes to it.
#[test]
fn the_emitted_state_model_reduces_the_way_its_policies_say() {
    let named: BTreeSet<&str> = REDUCTIONS.iter().map(|entry| entry.golden).collect();
    let goldens: BTreeSet<&str> = GOLDENS.iter().map(|golden| golden.directory).collect();
    assert_eq!(
        named, goldens,
        "every golden states what its state model does with two rounds of writes"
    );

    let Some(root) = installed() else {
        return;
    };

    for entry in REDUCTIONS {
        let golden = goldens::golden(entry.golden);
        let project = staged(golden, root, "reduce");
        let writes = project.join("state-writes.json");
        fs::write(&writes, entry.writes).expect("the scratch area is writable");

        let output = runner("state-reduction.mjs")
            .arg(&project)
            .arg(&writes)
            .output()
            .expect("bun runs");
        assert!(
            output.status.success(),
            "`{}` did not run its state model:\n{}",
            entry.golden,
            String::from_utf8_lossy(&output.stderr),
        );
        let mut answer: Value =
            serde_json::from_slice(&output.stdout).expect("the runner prints the state as JSON");
        // The compiler's own channel is not what this gate is about: it holds no
        // reduce policy an author wrote, and its own reducer is decided by
        // `the_run_channel_folds_a_steps_contributions_in_canonical_order`
        // below. What is asserted here is that it is *there* and starts empty,
        // which is the one thing a wrong `default:` would break.
        let run = answer
            .as_object_mut()
            .expect("the state is an object")
            .remove("$run")
            .expect("every state model carries the compiler's own channel");
        assert_eq!(
            run["step"], 0,
            "nothing here runs a node, so no step is taken"
        );
        assert_eq!(run["trace"], serde_json::json!([]));
        assert_eq!(run["iterations"], serde_json::json!({}));
        let expected: Value = serde_json::from_str(entry.expected).expect("the row is JSON");
        assert_eq!(
            answer, expected,
            "`{}`'s state model does not reduce the way its `reduce:` policies and `default:`s \
             say",
            entry.golden
        );
    }
}

/// Gate 2d: the compiler's own channel folds a step's contributions in the
/// canonical order, whatever order they arrive in (grammar 7.6.4).
///
/// Concurrent nodes of one step each write a piece of `$run`, and a reducer sees
/// them one at a time in whatever order the scheduler finished them. Grammar
/// 7.6.4 says completion order is never what decides the result, so the two
/// orders below have to fold to the same channel — and the trace has to come out
/// ordered by `(step, node)` rather than by arrival, or a replay would produce a
/// plausible alternative to the live run's record instead of a reproduction of
/// it.
#[test]
fn the_run_channel_folds_a_steps_contributions_in_canonical_order() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "run-channel");

    // Two nodes completing in one step, plus a later step: the shape a fork
    // produces. `review` finished first and `draft` second, which is the order
    // the canonical one is *not*.
    let arrived = r#"[
        { "step": 1, "traversals": { "review": 1 },
          "trace": [{ "step": 1, "node": "review", "outcome": "completed" }] },
        { "step": 1, "traversals": { "draft": 1 }, "iterations": { "flow.f#2": 1 },
          "trace": [{ "step": 1, "node": "draft", "outcome": "completed" }] },
        { "step": 2, "traversals": { "merge": 1 },
          "trace": [{ "step": 2, "node": "merge", "outcome": "completed" }] }
    ]"#;
    let reversed = r#"[
        { "step": 1, "traversals": { "draft": 1 }, "iterations": { "flow.f#2": 1 },
          "trace": [{ "step": 1, "node": "draft", "outcome": "completed" }] },
        { "step": 1, "traversals": { "review": 1 },
          "trace": [{ "step": 1, "node": "review", "outcome": "completed" }] },
        { "step": 2, "traversals": { "merge": 1 },
          "trace": [{ "step": 2, "node": "merge", "outcome": "completed" }] }
    ]"#;

    let fold = |contributions: &str, purpose: &str| -> Value {
        let path = project.join(format!("run-{purpose}.json"));
        fs::write(&path, contributions).expect("the scratch area is writable");
        let output = runner("run-channel.mjs")
            .arg(&project)
            .arg(&path)
            .output()
            .expect("bun runs");
        assert!(
            output.status.success(),
            "the run channel did not fold:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        serde_json::from_slice(&output.stdout).expect("the runner prints the channel as JSON")
    };

    let folded = fold(arrived, "arrived");
    assert_eq!(
        folded,
        fold(reversed, "reversed"),
        "completion order decided the channel's value"
    );

    // The step is the larger of the two, the counters merged key-wise, and the
    // trace is in `(step, node)` order rather than arrival order.
    assert_eq!(folded["step"], 2);
    assert_eq!(folded["iterations"]["flow.f#2"], 1);
    assert_eq!(
        folded["traversals"],
        serde_json::json!({ "draft": 1, "merge": 1, "review": 1 })
    );
    let nodes: Vec<&str> = folded["trace"]
        .as_array()
        .expect("a trace")
        .iter()
        .map(|entry| entry["node"].as_str().expect("a node id"))
        .collect();
    assert_eq!(nodes, ["draft", "review", "merge"]);
}

/// Gate 2i: what a fan-out does, decided from inside the runtime that does it
/// (grammar 8.6, 7.6.4, 9.4).
///
/// The acceptance suite runs both fan-out forms through a compiled graph against
/// scripted answers, which is where "the composition behaves" is settled. Five
/// of grammar 8.6's guarantees are not observable from there, and each is a rule
/// a wrong implementation would still pass a happy-path run with:
///
///   * **how many instances were in flight at once.** `max_concurrency` is a
///     normative bound (Decision D28) and an unenforced one produces the same
///     outputs, only faster and against a provider's rate limit;
///   * **which item a `fail` names.** Two items failing at different times must
///     report the **lowest-indexed** one, or two runs over one array fail about
///     different items;
///   * **whether the join returned before a detached delivery did.** "Resolved
///     at dispatch" (Decision D94) is a statement about *when*, and an outputs
///     assertion cannot see when;
///   * **that an exhausted `on_item_error: { retry: … }` resolves as `fail`
///     does** (rule 10), which needs an item that never answers;
///   * **that the ordering survives a completion order that is the reverse of
///     the source's**, at every one of the three reduce policies.
///
/// Five more are only reachable once the map node's **own policy** is in play,
/// which is every compiled map: grammar 9.3 level 3 puts a `timeout:` and a
/// `retry:` on every node a `defaults:` block covers, and the emitted
/// `examples/triage-fanout` carries both on its `dispatch` node. The runner
/// drives those through `runtime.runNode`, which is the seam a real map node
/// runs through:
///
///   * **a detached delivery still queued for a permit when the budget runs
///     out** is delivered anyway (rule 7, PRD 5.6) — its signal is its own, and
///     one that started against the node's already-aborted signal would be
///     recorded as `detached` and never sent;
///   * **a node that returned no answer still says what it dispatched.** A
///     deadline is raced, so the map's promise — and the `ItemFailure` its
///     records would have ridden out on — is abandoned;
///   * **the bound spans two executions of one node.** A detached delivery
///     outlives its call, so a bound counted per call lets a cycle's second
///     traversal, or the node's own `retry:`, reach twice the declared number;
///   * **a dispatched `flow.*` that failed keeps its own trace** (grammar 8.5),
///     which under `on_item_error: skip` is the only account of it there will
///     ever be, because the run then succeeds;
///   * **a subgraph is the one activity the budget can stop** (grammar 9.2). An
///     instance is a run of its own, so the node's signal reaches its Pregel
///     loop and it stops advancing — where an instance nothing aborted would run
///     every node it had left *after* the node that started it had failed. The
///     detached counterpart is asserted beside it, because Decision D94 puts
///     that delivery off the node's clock and the same signal must not cross
///     there.
///
/// `src/runtime.ts` is a compiler constant, byte-identical in every project this
/// release builds, so driving it directly is driving what every project runs.
#[test]
fn the_fan_out_runtime_bounds_orders_and_resolves_every_dispatch() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("triage-fanout"), root, "map-dispatch");
    let output = runner("map-dispatch.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the fan-out runtime did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let observed: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints its observations as JSON");

    // Grammar 7.6.4 clause 2: one write per channel, carrying every instance's
    // contribution in ascending source-item index — over a run whose completion
    // order was exactly the reverse.
    assert_eq!(
        observed["orderedByIndex"],
        serde_json::json!([
            { "channel": "results", "reduce": "append", "values": ["done-0", "done-1", "done-2"] }
        ])
    );
    assert_eq!(observed["orderedDispatches"], serde_json::json!([0, 1, 2]));

    // Grammar 8.6 rule 1 and Decision D28: the node's bound, and a route that
    // only ever tightens it.
    assert_eq!(observed["nodeBound"], 2, "six items ran two at a time");
    assert_eq!(observed["routedNodeBound"], 4);
    assert_eq!(
        observed["tightenedRouteBound"], 1,
        "the route's own bound held while the map's left room"
    );

    // Rule 10: `skip` drops the item and the fan-out carries on, and the writes
    // that landed are still in source-item order with the gaps closed up.
    assert_eq!(
        observed["skipped"],
        serde_json::json!([
            [0, "completed"],
            [1, "skipped"],
            [2, "completed"],
            [3, "skipped"]
        ])
    );
    assert_eq!(
        observed["skippedChannels"],
        serde_json::json!([
            { "channel": "results", "reduce": "append", "values": ["done-0", "done-2"] }
        ])
    );
    // …and under `fail`, the item reported is the lowest-indexed failure rather
    // than the first one in time: item 3 failed immediately and item 1 waited.
    // The failure carries the whole fan-out's account with it, because this is
    // the path where the map node is about to be absorbed by its own `on_error:`
    // and the items that ran already had their effects (PRD 5.3, 5.6). The two
    // that failed say so: under `fail` nothing skipped them.
    assert_eq!(
        observed["failed"],
        serde_json::json!({
            "name": "ItemFailure",
            "index": 1,
            "attempts": 1,
            "dispatches": [
                [0, "completed"],
                [1, "failed"],
                [2, "completed"],
                [3, "failed"]
            ],
        })
    );
    // A parameterized retry re-executes the whole instance…
    assert_eq!(
        observed["retried"],
        serde_json::json!({ "attempts": 3, "tries": 3 })
    );
    // …and when it runs out, the item fails, resolving as `fail` does (rule 10).
    assert_eq!(
        observed["exhausted"],
        serde_json::json!({ "name": "ItemFailure", "attempts": 2 })
    );
    // …but a deadline that ends the loop mid-backoff reports what the item
    // *did*, not what its policy allowed: `max: 5` and one attempt made, because
    // the node's signal aborted the first backoff (grammar 9.2). Both numbers a
    // reader sees are that one — the failure's, and the dispatch record's.
    assert_eq!(
        observed["abortedMidBackoff"],
        serde_json::json!({ "attempts": 1, "recorded": 1 })
    );

    // Decision D94: the join counted the detached dispatch the moment it was
    // issued. The delivery finished *after* the map node had already returned,
    // which is what "resolved at dispatch" means and what no output can show.
    assert_eq!(
        observed["detachOrder"],
        serde_json::json!(["joined-0", "joined", "detached"])
    );
    assert_eq!(
        observed["detachChannels"],
        serde_json::json!([{ "channel": "results", "reduce": "append", "values": ["0"] }]),
        "a detached dispatch writes no reduced state (grammar 8.6 rule 7)"
    );
    // Grammar 9.4: the execution id, then the frames of every node crossed from
    // the root instance down — here an outer `flow:` node, then this map's own
    // second traversal, then the item index.
    assert_eq!(
        observed["detachRecords"],
        serde_json::json!([
            { "index": 0, "outcome": "completed", "attempts": 1, "key": "exec_gate/outer/0/fan/1/0" },
            { "index": 1, "outcome": "detached", "attempts": 0, "key": "exec_gate/outer/0/fan/1/1" }
        ])
    );

    // Rule 6: a dispatch of zero instances writes nothing and completes — from
    // an empty array, and from a producer that was skipped (rule 11).
    assert_eq!(
        observed["empty"],
        serde_json::json!({ "channels": [], "dispatches": [] })
    );
    assert_eq!(
        observed["skippedProducer"],
        serde_json::json!({ "dispatches": 0 })
    );

    // Grammar 10.2, over both kinds of update: a plain write, and a map's batch
    // replayed into the same reducer in index order.
    assert_eq!(
        observed["reducers"],
        serde_json::json!({
            "appendOne": ["a", "b"],
            "appendBatch": ["a", "b", "c"],
            "mergeOne": { "a": 1, "b": 2 },
            "mergeBatch": { "a": 1, "b": 3, "c": 4 },
            "setOne": "b",
            "setBatch": "c",
        })
    );
    // …and the same batches through the **channels** those reducers belong to,
    // which is a different question: LangGraph keeps the first update to an
    // empty channel verbatim instead of calling the reducer with it, so a
    // channel with no initial value never gets the chance to unpack one. The
    // `last_wins` row without a `default:` is the one that shape reaches
    // (grammar 10.1, Decision D78), and `winner` holding the batch object
    // rather than a string is what this row is here to refuse.
    assert_eq!(
        observed["batched"],
        serde_json::json!({
            "notes": ["n-A", "n-B", "n-C"],
            "totals": { "who": "C", "note": "b" },
            "latest": "l-C",
            "winner": "w-C",
        })
    );

    // PRD 5.6's replay interaction, on the wire: what a **detached** delivery
    // hands its sink, on each of the three surfaces grammar 9.4 fixes — the
    // `Idempotency-Key` header, the `IDEMPOTENCY_KEY` variable, the
    // `idempotency_key` field of a host function's invocation context. The
    // joined dispatch beside each carries nothing, because a key on a call whose
    // outcome *is* observed would dedupe an effect meant to repeat; and a
    // binding that declares the name itself wins, as it does for `content-type`.
    assert_eq!(
        observed["deliveredHeaders"],
        serde_json::json!(["exec_gate/fan/0/1", null, "mine"])
    );
    assert_eq!(
        observed["deliveredEnv"],
        serde_json::json!({
            "detached": "exec_gate/fan/0/1",
            "joined": "",
            "declared": "mine",
            // An input field spelling the same variable is the one thing
            // grammar 9.4 says the key is never part of — so the *validator*
            // refuses that composition (`check::maps`, Decision D66, pinned by
            // `invalid-check/detached-dispatch-collides-with-its-sinks-delivery-slot`),
            // and a direct call that reaches this function anyway resolves it
            // the same way: the delivery wins.
            "collided": "exec_gate/fan/0/1",
            // …and the variable this process was started with is not a key at
            // all: the name grammar 9.4 fixes is a plain one, and a sink that
            // deduped on an operator's unrelated variable would drop repeat
            // calls a composition meant to repeat.
            "ambient": "",
        })
    );
    assert_eq!(
        observed["deliveredContext"],
        serde_json::json!(["exec_gate/fan/0/1", null])
    );
    // …and the fourth binding kind, on the same field: a `module:` sink reads
    // the key off the context it is called with, exactly as a `function:` one
    // does, so `callModule` forwarding the caller's context is what makes
    // at-least-once delivery to authored code a promise rather than a hope.
    //
    // The third argument rides along, because one call delivers both. Its
    // second name is `__proto__` — a name grammar 4.3's environment-variable
    // form accepts and `Object.prototype` answers to — and the value the
    // implementation reads is the declared string rather than a prototype,
    // which is the difference between an environment built by assignment and
    // one built out of own properties. `src/modules.ts` types that name
    // `string`; this is what makes the type true.
    assert_eq!(
        observed["deliveredModule"],
        serde_json::json!([
            {
                "key": "exec_gate/fan/0/1",
                "declared": "a literal",
                "proto": "a value, not the prototype",
                "text": "a1",
            },
            {
                "key": null,
                "declared": "a literal",
                "proto": "a value, not the prototype",
                "text": "a1",
            },
        ])
    );

    // Grammar 8.6's key table: `max_concurrency` is a node-wide **admission**
    // bound over every in-flight dispatch, detached included. Six items, half of
    // them down a detached route bounded at 2 of its own — and still never more
    // than the map's own 2 in flight, which is the whole promise a composition
    // makes to a rate-limited provider.
    // …and the hazard that comes with it: at a bound of 1 the delivery cannot be
    // admitted until the joined instance ahead of it finishes, which is after
    // the map node has returned. It is still delivered — a message a bound
    // merely delayed past the join would otherwise be a message lost (D94).
    //
    // `aheadOfJoined` is that queue read from the other end, which is the end
    // where the bound and grammar 8.6 rule 7 can contradict each other: rule 7
    // says nothing a detached dispatch does can **delay** the enclosing flow
    // instance, so the detached item is at index 0 and the joined item behind it.
    // A delivery that took the only permit at index 0 and held it until it
    // settled would make the join wait out the sink, and the answer here would
    // read `["delivered", "joined", "returned"]`. `joinedBehindAHangingSink` is
    // the same shape with a sink that never answers at all — the case where
    // holding the permit does not delay the join but ends it, since a map node
    // carrying no `timeout:` (grammar 9.3 level 4) has nothing to cut the wait
    // short and the flow instance blocks for ever on a fire-and-forget delivery.
    assert_eq!(
        observed["detachedBound"],
        serde_json::json!({
            "declared": 2,
            "peak": 2,
            "queuedThenDelivered": ["joined", "returned", "delivered"],
            "aheadOfJoined": ["joined", "returned", "delivered"],
            "joinedBehindAHangingSink": "joined-returned",
        })
    );

    // A map node under its own `timeout:`, driven through `runNode`. The
    // delivery was still **queued** for a permit when the budget ran out — the
    // joined instance ahead of it held the map's only one — and it was still
    // delivered: it runs under a signal of its own, so it does not start against
    // the node's already-aborted one, throw before anything reaches the wire,
    // and disappear into the catch that keeps a detached dispatch from failing
    // the flow. A record saying `detached` for a message nobody sent is the lost
    // message grammar 8.6 rule 7 and PRD 5.6 trade dedupe-on-a-key to avoid.
    assert_eq!(
        observed["deadlineWhileQueued"],
        serde_json::json!({
            "outcome": "skipped",
            "timedOut": true,
            "attempted": ["exec_probe/queued/0/1"],
            "delivered": ["exec_probe/queued/0/1"],
        })
    );
    // …and the account the node still owes. A deadline is *raced* (grammar 9.2),
    // so the map's promise is abandoned where it stands and neither its answer
    // nor an `ItemFailure` ever arrives — yet item 0 completed and item 1 was
    // delivered to a sink before the budget expired, and the record is the only
    // place either is visible. Item 2 was still in flight, so it has no outcome
    // to report and the entry's own error accounts for it.
    assert_eq!(
        observed["deadlineKeepsTheRecord"],
        serde_json::json!({
            "outcome": "skipped",
            "dispatches": [[0, "completed"], [1, "detached"]],
            "keys": ["exec_probe/partial/0/0", "exec_probe/partial/0/1"],
        })
    );
    // …and the bound on that recovery, which is what keeps it honest. Reading
    // records off a node's **input** is the one path that does not start from
    // something the node produced, and `runNode` takes it for every node that
    // failed — so a plan has to be recognised by something a composition cannot
    // write. `instances` and `records` are field names grammar 2.1 allows, and a
    // node declaring an `input:` with both arrays fails here holding the
    // composition's own data; `docs/trace.md` §3 says only a `map` node's entry
    // carries `dispatches` and §5 says its elements are `DispatchRecord`s.
    assert_eq!(
        observed["aPlanShapedInputIsNotAPlan"],
        serde_json::json!({ "outcome": "skipped", "dispatches": null })
    );

    // Grammar 8.6's key table again, over the span the bound has to cover: a
    // detached delivery outlives the call that issued it (D94), so permits
    // counted per *call* are not a bound on the node at all. Both ways a node
    // runs twice are driven — a second traversal of a bounded cycle, and the
    // node's own `retry:` re-executing the whole fan-out — and each would reach
    // 2 in flight against a declared 1 if the gates were rebuilt per call.
    assert_eq!(
        observed["admissionAcrossTraversals"],
        serde_json::json!({ "declared": 1, "peak": 1 })
    );
    assert_eq!(
        observed["admissionAcrossRetries"],
        serde_json::json!({
            "declared": 1,
            "peak": 1,
            // The node really did run its fan-out twice, and its own `on_error:`
            // absorbed the second failure — otherwise "the bound held" would be
            // a claim about one execution.
            "attempts": 2,
            "outcome": "skipped",
        })
    );

    // The other half of a bound that spans executions. Permits outliving a call
    // is what makes the bound real, and it is also the one way a detached
    // delivery can still be in front of a joined instance: a later execution of
    // the node knows nothing of the deliveries an earlier one left queued, and
    // grammar 8.6 rule 7 says nothing a detached dispatch does can delay the
    // enclosing flow instance. Two deliveries are issued at a bound of 1, so the
    // second is queued on the *node* gate; the next execution's joined instance
    // is then served ahead of it, waiting out only the delivery already running.
    // Under a first-come queue this reads `delivered-1` before `joined`.
    assert_eq!(
        observed["joinAheadOfAQueuedDelivery"],
        serde_json::json!(["delivered-0", "joined", "returned", "delivered-1"])
    );

    // Grammar 8.5: a dispatched `flow.*` that failed keeps the trace of its own
    // instance. `on_item_error: skip` drops the item and the run **succeeds**,
    // so every guard, budget and attempt inside that boundary is either on this
    // record or nowhere at all (PRD 5.3).
    assert_eq!(
        observed["failedSubflowRecord"],
        serde_json::json!({
            "outcome": "skipped",
            "inner": [{
                "step": 1,
                "flow": "flow.worker",
                "node": "work",
                "traversal": 0,
                "outcome": "failed",
                "attempts": 2,
                "error": "Error: boom",
            }],
            "error": "SubflowFailure: the instance of `flow.worker` did not run to quiescence: Error: boom",
        })
    );

    // Grammar 9.2 over a subgraph, which is the one activity a deadline can
    // really stop. `runActivity` races every other kind and leaves it running,
    // because a host function cannot be unscheduled — an instance can: it is a
    // run of its own, and the signal reaches its Pregel loop. So the node's
    // budget ends the *instance*, not only the node's wait for it: `two` never
    // ran, and `one` — already in flight, and deaf to any signal on purpose —
    // finished into a value nobody read, exactly as an abandoned host function
    // does. Without the signal crossing, `effects` reads `["one", "two"]` and
    // the instance would have gone on issuing effects, and holding the map
    // node's admission permit, for as long as it had nodes left.
    assert_eq!(
        observed["subflowOnTheNodesClock"],
        serde_json::json!({
            "outcome": "skipped",
            "timedOut": true,
            "atReturn": [],
            "effects": ["one"],
            // …and the boundary the same signal must not cross: a **detached**
            // dispatch is off the node's clock (D94), so its instance runs to
            // quiescence after the map node has already given up on the joined
            // item beside it.
            "detached": { "outcome": "skipped", "effects": ["one", "two"] },
        })
    );

    // Rules 2 and 4: a union may declare a variant tagged `default` and a
    // `default:` catch-all beside it — routes are keyed by variant tag and
    // `default:` is a map-block key, so the two never collide in the source.
    // They must not collide in the record either: `selectRoute` searches the
    // named routes first and gets it right, and two routes may share a target,
    // so the tag is the only thing that could tell a reader which ran.
    assert_eq!(
        observed["defaultTagCollision"],
        serde_json::json!([
            [0, "default", "agent.named"],
            [1, "$default", "agent.catchall"]
        ])
    );

    // Grammar 9.3 level 1: the outermost instantiation site wins a field
    // (Decision D79), an absent one is filled in from the inner site, and a
    // `human` node takes neither `timeout` nor `retry` from it (Decision D102).
    assert_eq!(
        observed["policy"],
        serde_json::json!({
            "outermost": { "timeoutMs": 30_000 },
            "filledIn": { "timeoutMs": 30_000, "onError": "skip" },
            "none": null,
            "exempt": { "onError": "skip" },
            "plain": { "timeoutMs": 10_000, "onError": "fail" },
        })
    );
}

/// Gate 19: the `human` wait board — addressing, abandonment, the timer a
/// released wait leaves behind, and the budget a wait does not spend
/// (grammar 8.7, 9.2, PRD 5.11).
///
/// The acceptance suite answers, expires and addresses pauses through a served
/// app, which is where the composition's behaviour is decided. Twelve claims are
/// not decidable there — eight because the case that breaks them is a task
/// **nobody is awaiting**, one because the case that breaks it is a bug in the
/// runtime rather than anything a composition can ask for, two because every
/// composition that can reach one declares an `on_error:` the orderings agree
/// on, and one because it needs the journal's write to fail on command:
///
///   * **a pause is addressed by, and belongs to, an instance path.** A node
///     holds the pauses its own path is a prefix of, which is the whole of what
///     links a wait to the node that dispatched the instance it happened in —
///     and is what [`runActivity`] reads to hold its deadline still (D102);
///   * **an abandoned pause leaves the board.** A wait whose node stopped
///     waiting would otherwise be published by the status route as a question a
///     person can still answer and taken by the resume route as an answer that
///     goes nowhere. Reaching it from a composition needs a dispatch abandoned
///     at a moment a test cannot schedule;
///   * **a retry ladder abandons what its failed attempt left parked**, before
///     the next attempt re-executes the instance at the same site — in both
///     ladders, a node's `retry:` and an item's `on_item_error:`. The shape
///     needs one branch of an instance to fail while a sibling is parked, which
///     is a scheduling a composition cannot ask for;
///   * **a settlement reaches its own pause and no other.** A wait id is an
///     instance path and a path is re-run, so the board can hold a successor
///     under an id a stale expiry timer still remembers. What breaks it is a
///     timer firing after its entry was settled and replaced — an interleaving
///     of one abandonment, one re-park and one callback that no served run can
///     be asked for;
///   * **the board refuses to displace a wait that is still waiting.** The
///     abandon-first ordering the two ladders keep is the board's own rule
///     rather than a convention held at their call sites: a pause that silently
///     displaced an unsettled one would leave a task parked on a promise no
///     resume, abandonment or release could reach — a run that hangs, which is
///     the one failure indistinguishable from a slow machine. Unreachable from
///     any composition by construction, which is exactly why it is asserted
///     here;
///   * **an abandonment is not an activity outcome.** `on_error:` governs what
///     the model, the process or the request did; a pause nobody is waiting for
///     any more is the run's own unwinding, and a `skip` that absorbed one would
///     route a graph past a `human` node whose answer the composition declared
///     it needed;
///   * **an expiry and an interrupt are not activity outcomes either**, and the
///     node that decides it is one declaring `on_timeout:` beside an *explicit*
///     `on_error:` — the pair grammar 8.7 permits, since it is `timeout:` and
///     `retry:` a `human` node refuses (D102). The expiry routes to
///     `on_timeout:`'s target **instead of** the node's own edges (grammar 9.2)
///     and the interrupt leaves the node, both over the top of a `skip` that
///     absorbs an ordinary delivery failure at that same node. A served app
///     cannot tell the orderings apart: every fixture that reaches an expiry
///     resolves `on_error: fail`, where they agree;
///   * **a settlement the journal cannot record fails the node.** The wait is
///     marked settled before its record is written, so a write that threw out
///     of the settlement would leave a wait nothing may settle again holding a
///     promise nothing ever settles — a run that hangs rather than one that
///     fails, on both settlements that are journaled. Deciding it needs the
///     write to refuse on command, which is a stub recorder rather than a
///     composition;
///   * **a released wait's expiry timer is cleared.** An `unref`ed timer keeps
///     nothing alive, so it is invisible to `process.getActiveResourcesInfo()`
///     and to the process exiting: the runner counts the global
///     `setTimeout`/`clearTimeout` calls made with the pause's own budget, which
///     is the only place that question has an answer at all;
///   * **the budget an activity divides is held still with the timer.**
///     `context.deadline` is what a model route subtracts the clock from
///     ([`requestBudget`]), so a reading that stayed put while the node's own
///     timer was held would hand the first call after the wait a budget the wait
///     had spent. What a served app can show is the node not failing; what the
///     reading *said* is only visible from inside the activity.
///
/// `src/runtime.ts` is a compiler constant, byte-identical in every project this
/// release builds, so driving it directly is driving what every project runs.
#[test]
fn the_human_wait_board_addresses_abandons_and_releases_every_pause() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "human-waits");
    let output = runner("human-waits.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the wait board did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let observed: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints its observations as JSON");
    the_wait_board_behaved(&observed);
}

/// Everything gate 19 asks of the wait board, as a function of the runner's
/// answer.
///
/// Factored out for the reason gates 9 to 12 are: gate 13 re-runs this runner
/// under the Node fallback, and two columns that asserted separately could come
/// to different verdicts by drifting apart rather than by the runtimes
/// disagreeing.
fn the_wait_board_behaved(observed: &Value) {
    // Grammar 9.4, flattened: two instances of one `map` hold two pauses, told
    // apart by the path that addresses them and published in id order rather
    // than in the order the scheduler happened to park them.
    assert_eq!(
        observed["addressing"],
        json!({
            "ids": ["fan/0/0/sign/0", "fan/0/1/sign/0"],
            "under_the_map": 2,
            "under_one_instance": 1,
            "under_the_wait_itself": 1,
            "under_another_node": 0,
            "both_pending": ["pending", "pending"],
        })
    );

    // …and abandoning the node that dispatched them settles both: nothing is
    // published, nothing is open, and a resume is refused as the abandonment it
    // is rather than taken with a `202` that discards the answer.
    assert_eq!(
        observed["abandoning"]["both_settled"],
        json!(["HumanAbandoned", "HumanAbandoned"])
    );
    assert_eq!(observed["abandoning"]["still_published"], json!([]));
    assert_eq!(observed["abandoning"]["still_open"], json!(0));
    assert_eq!(observed["abandoning"]["refusal"]["ok"], json!(false));
    assert_eq!(
        observed["abandoning"]["refusal"]["reason"],
        json!("settled")
    );
    assert!(
        observed["abandoning"]["refusal"]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("no longer held"),
        "the refusal says what became of the wait: {observed}"
    );

    // …and it is `runActivity` that does it, on every way one node execution can
    // end. The reachable shape is a node's own **deadline** racing an instance
    // parked below it — the node is over, so the pause under it is one nothing
    // will read the answer of — and the runner drives the same `finally` with a
    // throwing activity, because what is under test is the wiring rather than
    // which error reached it.
    assert_eq!(
        observed["orphans"],
        json!({
            "before": 1,
            "after": 0,
            "published": [],
            "settled": "HumanAbandoned",
            "node_failed": "NodeFailure",
            "refusal": "settled",
        })
    );

    // …and a **retry** is the other way one task stops waiting for a pause: the
    // next attempt re-executes the instance at the same site, so an attempt that
    // ended with a pause still parked would hand its successor a board already
    // holding a wait under the id that successor is about to use. Every attempt
    // begins with nothing held under its site, both pauses end abandoned, and
    // the node fails the way a node whose every attempt failed does. Without the
    // abandon between attempts the second entry reads `1`.
    assert_eq!(
        observed["retrying"],
        json!({
            "open_at_each_attempt": [0, 0],
            "settled": ["HumanAbandoned", "HumanAbandoned"],
            "published": [],
            "node_failed": "NodeFailure",
        }),
        "a retry ladder left the pauses of a failed attempt on the board: {observed}"
    );

    // The same seam for `on_item_error: { retry: … }`, whose attempts all
    // re-execute one dispatched instance at one site.
    assert_eq!(
        observed["item_retrying"],
        json!({
            "open_at_each_attempt": [0, 0],
            "settled": ["HumanAbandoned", "HumanAbandoned"],
            "published": [],
            "node_failed": "NodeFailure",
        }),
        "an item's retry ladder left the pause of a failed attempt on the board: {observed}"
    );

    // And the half of that which is not an ordering: a settlement reaches the
    // pause it belongs to and no other. A wait id is an instance path and a path
    // is re-run, so a stale closure — an expiry timer above all — can outlive the
    // entry it settles and find a *successor* answering to its id. Driven by
    // abandoning a pause, opening its successor at the same site, and only then
    // firing the budget the abandoned one had armed: what it must settle is
    // itself, which is nothing, leaving the live question published, answerable,
    // and answered. A settlement that went by id instead reports the live pause
    // `expired`, publishes nothing, and refuses the honest answer.
    assert_eq!(
        observed["successor"],
        json!({
            "stale": "HumanAbandoned",
            "open_after_the_stale_budget_ran_out": 1,
            "published": ["wrap/0/sign/0"],
            "taken": { "ok": true, "wait": "wrap/0/sign/0" },
            "live": "resolved",
        }),
        "a stale pause's expiry settled the wait that succeeded it: {observed}"
    );

    // …and the ordering that section relies on is the board's rule rather than a
    // convention: a pause opened under an id an **unsettled** wait still answers
    // to is refused outright. Silently displacing it would take that wait off the
    // board with nothing able to reach it — no resume, no abandonment, no release
    // — leaving its task parked on a promise nothing can settle, which is a run
    // that hangs rather than a run that fails. The standing pause is untouched by
    // the refusal: published, open, and what the answer reaches.
    assert_eq!(
        observed["displacing"]["refused"],
        json!("WaitBoardInvariant"),
        "the board displaced a pause that was still waiting: {observed}"
    );
    assert!(
        observed["displacing"]["said"]
            .as_str()
            .unwrap_or_default()
            .contains("`wrap/0/sign/0` is still waiting"),
        "the refusal names the id it broke at: {observed}"
    );
    assert_eq!(
        observed["displacing"]["published"],
        json!(["wrap/0/sign/0"]),
        "{observed}"
    );
    assert_eq!(observed["displacing"]["open"], json!(1), "{observed}");
    assert_eq!(
        observed["displacing"]["taken"],
        json!({ "ok": true, "wait": "wrap/0/sign/0" }),
        "{observed}"
    );
    assert_eq!(
        observed["displacing"]["standing"],
        json!("resolved"),
        "the pause that was already there is the one the answer reached: {observed}"
    );
    assert_eq!(
        observed["displacing"]["output"],
        json!({ "decision": "approve" }),
        "{observed}"
    );

    // An abandonment is an unwinding rather than an outcome, so it passes the
    // node's `on_error:` untouched. The control above it is what makes this an
    // assertion about the guard: the same node under the same `skip` really does
    // absorb an ordinary delivery failure, entry, edges and all.
    assert_eq!(
        observed["absorbing"]["delivery_failure"],
        json!({ "outcome": "skipped", "goto": ["__end__"] })
    );
    assert_eq!(observed["absorbing"]["pending"], json!(["sign/0"]));
    assert_eq!(
        observed["absorbing"]["abandoned"],
        json!("HumanAbandoned"),
        "`skip` absorbing this would route the graph past the human: {observed}"
    );

    // …and an **expiry** is not one either, which is the precedence a node
    // declaring both keys turns on: `on_timeout:` transfers control to its route
    // *instead of* the node's own edges (grammar 8.7, 9.2), over the top of an
    // explicit `on_error: skip` that would otherwise mark the node skipped and
    // send it down them. The node's edge goes to `__end__` and the route does
    // not, so `goto` is the whole assertion: a `runNode` that consulted
    // `policy.onError` first reports `["__end__"]` and `"skipped"` here. No
    // served composition can decide it — every fixture that reaches an expiry
    // resolves `on_error: fail`, where the two orderings agree.
    assert_eq!(
        observed["expiry_over_skip"],
        json!({
            "settled": "resolved",
            "goto": ["note"],
            "outcome": "failed",
            "fallback": "note",
            "pause_settled": "expired",
            "published": [],
        }),
        "`on_error: skip` absorbed the expiry instead of `on_timeout:` routing it: {observed}"
    );

    // …and neither is an **interrupt**: a run with no way to answer stops at the
    // pause rather than being skipped past it, under the same explicit `skip`.
    // `runActivity` wraps it in a `NodeFailure` — the interrupt is on the cause
    // chain, which is what `runNode` answers ahead of the policy and what
    // `cli.ts` reads for its own exit path.
    assert_eq!(
        observed["interrupt_over_skip"],
        json!({
            "threw": true,
            "name": "NodeFailure",
            "interrupt": "HumanInterrupt",
            "node": "sign",
        }),
        "`on_error: skip` absorbed the interrupt instead of the run stopping: {observed}"
    );

    // The budget reading moves with the hold: fixed while the timer is armed —
    // it is the instant that timer will fire — and sliding with the clock while
    // a pause below the node is open, which is the whole of "the budget does not
    // run while a human is thinking" said to the activity that divides it.
    assert_eq!(
        observed["budget"]["armed_moved_by"],
        json!(0),
        "an armed budget expires at one instant: {observed}"
    );
    assert_eq!(
        observed["budget"]["rearmed_moved_by"],
        json!(0),
        "…and so does the same budget re-armed after the answer: {observed}"
    );
    let held = observed["budget"]["held_moved_by"]
        .as_i64()
        .unwrap_or_else(|| panic!("the runner reports how far the held budget moved: {observed}"));
    assert!(
        held >= 90,
        "a budget held for 100ms of pause moved {held}ms, so the wait was spending it: {observed}"
    );
    assert_eq!(observed["budget"]["settled"], json!("resolved"));

    // A 24-hour budget arms one timer, and releasing the run clears it — rather
    // than leaving it, and the closure it holds, alive for the day.
    assert_eq!(
        observed["timer"],
        json!({
            "armed": 1,
            "cleared_while_pending": 0,
            "cleared_after_release": 1,
            "settled": "HumanAbandoned",
            "after_release": "not-waiting",
        })
    );

    // The ordinary path, unchanged by any of it: the pause is published with
    // what the human is shown and the schema their answer is held to, the answer
    // is parsed against that schema, and the wait settles exactly once.
    assert_eq!(
        observed["answering"]["published"]["id"],
        json!("review/0/sign/0")
    );
    assert_eq!(
        observed["answering"]["published"]["shown"],
        json!({ "question": "ship it?" })
    );
    assert_eq!(
        observed["answering"]["published"]["schema"]["properties"]["decision"]["enum"],
        json!(["approve", "reject"])
    );
    assert_eq!(
        observed["answering"]["taken"],
        json!({ "ok": true, "wait": "review/0/sign/0" })
    );
    assert_eq!(observed["answering"]["settled"], json!("resolved"));
    assert_eq!(
        observed["answering"]["output"],
        json!({ "decision": "reject" })
    );
    assert_eq!(
        observed["answering"]["pause"],
        json!({
            "settled": "resumed",
            "paused_at_is_an_instant": true,
            "settled_at_is_an_instant": true,
            "expires_at": null,
        })
    );
    assert_eq!(
        observed["answering"]["twice"],
        json!({ "ok": false, "reason": "settled" })
    );
    assert_eq!(
        observed["answering"]["mismatched"],
        json!({ "ok": false, "reason": "settled" })
    );
    assert_eq!(observed["answering"]["still_published"], json!([]));

    // …and a journal that refuses the settled wait's record fails the **node**
    // rather than leaving the pause parked for ever. The wait is marked settled
    // before the record is written, so a write that threw out of the settlement
    // would leave a wait nothing may settle again holding a promise nothing ever
    // settles — the `human` node's `await` never returns, and the run neither
    // fails nor parks nor ends. Without the guard `settled` reads `"pending"`
    // and the delivery reports the write's error as its own.
    assert_eq!(
        observed["unwritable_answer"],
        json!({
            "delivery": { "ok": true, "threw": null },
            "settled": "Error",
            "reported": "the journal refused this record",
            "still_published": [],
            "again": "settled",
        }),
        "a pause whose record could not be written left its node parked: {observed}"
    );

    // The same on the arm a `setTimeout` fires, where a throw is an uncaught
    // exception rather than something a caller could report — and where the
    // expiry must not route either, since `on_timeout:` taken past a wait whose
    // expiry the journal does not hold is a budget the resume spends again.
    assert_eq!(
        observed["unwritable_expiry"],
        json!({
            "settled": "Error",
            "reported": "the journal refused this record",
            "still_published": [],
        }),
        "an expiry whose record could not be written did not fail its node: {observed}"
    );

    // A run with no answer surface never registers a pause: an `agent-compose
    // run` whose standard input is not a terminal has no way to answer one, so
    // the node raises instead of parking, and there is nothing for a status
    // route to publish (grammar 8.7).
    assert_eq!(
        observed["unanswerable"],
        json!({ "settled": "HumanInterrupt", "published": [] })
    );

    a_pause_a_worker_opened_is_a_pause(observed);
}

/// A pause a **worker** opened is this board's own entry, and behaves as a local
/// one does (`docs/distributed.md` §3.4, PRD resolved q46).
///
/// The parity bar the resolution sets is "a placed `human:` node must mean what
/// the same node unplaced means", and the sections above are the local half of
/// exactly these readings — so a divergence shows as two assertions in one file
/// disagreeing rather than as a served hub behaving oddly.
///
/// Split out for [`the_wait_board_behaved`]'s own reason: gate 13 re-runs this
/// runner under the Node fallback, and one function is what keeps the two
/// columns from drifting into different verdicts.
fn a_pause_a_worker_opened_is_a_pause(observed: &Value) {
    // Published as a pause is published, under the identity the worker derived
    // — and with the **contract this hub holds**, read out of the descriptor
    // registry rather than off anything the wire carried (§4.3).
    assert_eq!(
        observed["remote_answered"]["published"],
        json!([{
            "id": "escalate/0/sign/0",
            "flow": "flow.sign_off",
            "node": "sign",
            "shown": { "question": "ship it?" },
            "schema": {
                "type": "object",
                "properties": { "decision": { "enum": ["approve", "reject"] } },
                "required": ["decision"],
                "additionalProperties": false,
            },
        }]),
        "a worker's pause is not published the way a local one is: {observed}"
    );
    // …and dated where it was planted rather than where it was asked. Both
    // instants of a published wait are the holding generation's, which is what
    // makes the pair an interval on either side of the wire (§3.4, PRD resolved
    // q46's parity bar); the wire's instant is on the settled dispatch row,
    // which is the record of what that machine's clock said.
    assert_eq!(
        observed["remote_answered"]["published_paused_at_is_an_instant"],
        json!(true),
        "a worker's pause is published with no date on it: {observed}"
    );
    assert_eq!(
        observed["remote_answered"]["published_paused_at_is_the_wires"],
        json!(false),
        "the wait publishes the instant the worker's clock stamped rather than the one this hub \
         planted it at, so a reader is shown a pair read off two machines: {observed}"
    );
    // Counted by `pausesUnder`, which is the reading `runActivity` holds a
    // dispatching node's deadline still by (D102): the time a person spends
    // thinking is not time the placed node's `timeout:` counts, exactly as it is
    // not time an unplaced one's counts.
    assert_eq!(observed["remote_answered"]["held_under"], json!(1));
    assert_eq!(observed["remote_answered"]["held_elsewhere"], json!(0));
    // A payload the node's `output:` refuses is a `mismatch` and consumes
    // nothing, which is the resume-payload validation §3.4 promises is the
    // single-process one.
    assert_eq!(observed["remote_answered"]["refused"], json!("mismatch"));
    assert_eq!(
        observed["remote_answered"]["waiting_after_a_mismatch"],
        json!(1)
    );
    // …and the answer settles it into exactly the record `runHuman` writes for a
    // pause the hub held itself, which is what the redispatch replays.
    assert_eq!(observed["remote_answered"]["settled"], json!("resolved"));
    assert_eq!(
        observed["remote_answered"]["record"]["settled"],
        json!("resumed")
    );
    assert_eq!(
        observed["remote_answered"]["record"]["output"],
        json!({ "decision": "approve" })
    );
    assert_eq!(
        observed["remote_answered"]["record_carries_the_published_pause"],
        json!(true),
        "the record holds an instant the board never published, so the answered pause's trace \
         entry reports a wait the execution was never under (docs/trace.md §3.4): {observed}"
    );
    assert!(
        observed["remote_answered"]["record"]["settledAt"].is_string(),
        "the record does not say when the wait stopped waiting: {observed}"
    );
    // **The hub's writer is handed the record inside the settlement**, before the
    // promise the resume route answers `202` off resolves — which is where
    // `runHuman` calls `slot.keep` and for its reason: a record written in a
    // later turn of the loop is one a process killed in between never wrote, and
    // the next start re-derives the pause off the settled dispatch row and asks
    // the person a second time (`docs/durability.md` §3.4).
    assert_eq!(
        observed["remote_answered"]["wrote"],
        json!(["kept", "resolved"]),
        "the answer to a pause a worker opened is journaled after the promise it is acknowledged \
         off resolves: {observed}"
    );
    assert_eq!(
        observed["remote_answered"]["written_record"], observed["remote_answered"]["record"],
        "the record handed to the hub's writer is not the one the node goes on with: {observed}"
    );
    assert_eq!(
        observed["remote_answered"]["waiting_after_the_answer"],
        json!(0)
    );
    // …and a second answer is refused with the sentence a local pause's second
    // answer is refused with. A delivery that re-settled it would journal a
    // second `human` record over an answer somebody already gave — the record
    // below says the first one stands.
    assert_eq!(
        observed["remote_answered"]["twice"],
        json!("settled"),
        "a wait a worker opened took a second answer: {observed}"
    );
    assert_eq!(
        observed["remote_answered"]["record_after_the_second_answer"],
        observed["remote_answered"]["record"],
        "the second answer rewrote the record the first one settled: {observed}"
    );

    // An expiry settles into the record whose replay raises the node's own
    // `on_timeout:` — the composition's route, decided by the same line of the
    // same function an unplaced pause's expiry is decided by.
    assert_eq!(observed["remote_expired"]["settled"], json!("resolved"));
    assert_eq!(
        observed["remote_expired"]["record"]["settled"],
        json!("expired")
    );
    assert!(
        observed["remote_expired"]["record"].get("output").is_none(),
        "an expired wait recorded an answer nobody gave: {observed}"
    );
    assert_eq!(
        observed["remote_expired"]["written_record"], observed["remote_expired"]["record"],
        "an expiry is not journaled through the writer an answer is, so a run could take \
         `on_timeout:` past a wait whose expiry the journal does not hold: {observed}"
    );

    // **The budget is the composition's, and the clock is this hub's.** The
    // pause driven here carries an `expires_at` an hour in this process's past —
    // what a worker an hour behind stamps on a question it just asked — and the
    // node's own `timeout:` is a minute. Armed off the wire the wait would have
    // expired on the next tick and no person could ever have answered it; armed
    // off the descriptor it is still open, and takes the answer.
    assert_eq!(
        observed["remote_skewed"]["settled_while_the_budget_runs"],
        json!("pending"),
        "a worker's clock decided when this hub's wait expired, so a `timeout:` the composition \
         declares means something different on every machine: {observed}"
    );
    assert_eq!(observed["remote_skewed"]["settled"], json!("resolved"));
    assert_eq!(
        observed["remote_skewed"]["record"]["settled"],
        json!("resumed")
    );
    // …and the deadline a reader is shown is the deadline that fires. The wire's
    // instant is an hour in this process's past, so a board that republished it
    // would show the question as expired for the whole minute the resume surface
    // still takes its answer — a status route contradicting the resume route.
    assert_eq!(
        observed["remote_skewed"]["published_expires_at_is_the_wires"],
        json!(false),
        "the wait publishes the instant the worker's clock stamped rather than the one this hub \
         armed, so a surface shows a deadline that is not the one that will fire: {observed}"
    );
    assert_eq!(
        observed["remote_skewed"]["published_expires_at_is_ahead"],
        json!(true),
        "the wait publishes a deadline this process is already past while its own timer runs on: \
         {observed}"
    );
    assert_eq!(
        observed["remote_skewed"]["record_dates_the_deadline_it_published"],
        json!(true),
        "the record holds a deadline other than the one the board published, so a replayed wait \
         reports a budget the execution was never under: {observed}"
    );
    // **And the pair those rules leave is one clock's**, which is PRD resolved
    // q46's parity bar for status visibility: the same node unplaced dates both
    // members off one reading, so `expires_at − paused_at` is the node's
    // `timeout:` — and a placed pause has to publish the same. The worker driven
    // here runs an hour ahead, which is what makes the reading decidable rather
    // than a rounding: a board that had kept the wire's `pausedAt` beside its own
    // deadline would publish a question dated an hour from now expiring a minute
    // from now, an *inverted* pair. The assertions below pin the three halves of
    // it — neither published member came off the wire, the dating is this
    // planting's, and the gap is the whole minute the composition declares.
    assert_eq!(
        observed["remote_planting"]["settled_while_the_budget_runs"],
        json!("pending"),
        "a wait a worker dated in this hub's future was settled before anyone could answer it: \
         {observed}"
    );
    assert_eq!(
        observed["remote_planting"]["published_paused_at_is_the_wires"],
        json!(false),
        "the wait publishes the instant the worker's clock stamped rather than the one this hub \
         planted it at, so a reader is shown a pair read off two machines (docs/distributed.md \
         §3.4): {observed}"
    );
    assert_eq!(
        observed["remote_planting"]["published_expires_at_is_the_wires"],
        json!(false),
        "the wait publishes the worker's deadline rather than the one this hub armed: {observed}"
    );
    let budget = observed["remote_planting"]["budget_from_the_planting_ms"]
        .as_i64()
        .unwrap_or(-1);
    assert!(
        (60_000..=65_000).contains(&budget),
        "the deadline published is {budget}ms after the planting, where the node declares a \
         minute: a worker's clock moved the budget: {observed}"
    );
    let dated = observed["remote_planting"]["dated_from_the_planting_ms"]
        .as_i64()
        .unwrap_or(i64::MAX);
    assert!(
        (0..=5_000).contains(&dated),
        "the wait is dated {dated}ms from the planting, where a wait a process opens is dated the \
         instant it opens it: a worker's clock decided when this hub's question was asked: \
         {observed}"
    );
    let gap = observed["remote_planting"]["published_gap_ms"]
        .as_i64()
        .unwrap_or(0);
    assert_eq!(
        gap, 60_000,
        "`expires_at − paused_at` came back {gap}ms, where the node declares a minute: the two \
         published members are no longer one clock's reading and its budget, so a reader \
         computing what is left of a question measures the offset between two machines instead \
         (docs/distributed.md §3.4, PRD resolved q46): {observed}"
    );
    assert_eq!(
        observed["remote_planting"]["settled"],
        json!("resolved"),
        "a wait planted from a wildly skewed pause could not be answered: {observed}"
    );
    assert_eq!(
        observed["remote_planting"]["record_carries_the_published_pair"],
        json!(true),
        "the journaled record holds a pair the board never published, so the answered pause's \
         trace entry reports instants the execution was never under (docs/trace.md §3.4): \
         {observed}"
    );
    // …and a node with no `timeout:` publishes no deadline, whatever the wire
    // dated the pause: grammar 8.7 makes that wait unbounded, and an instant
    // nothing will fire is not one a surface may show.
    assert_eq!(
        observed["remote_unbounded"],
        json!({ "settled": "pending", "published_expires_at": null }),
        "a pause under a node that declares no `timeout:` published an expiry nothing will ever \
         fire: {observed}"
    );
    // A pause naming a `human:` node this build no longer declares is resolved
    // q29's disagreement — a journal that does not describe this run — rather
    // than a bare failure a node's `on_error:` could absorb. Only a restart on a
    // rebuilt artifact reaches it: the result route refuses such a pause before
    // the dispatch is settled.
    assert_eq!(
        observed["remote_unregistered"],
        json!({
            "settled": "ReplayDivergence",
            "names_the_record": true,
            "published": 0,
        }),
        "a pause naming a node this build does not declare did not fail as a divergence naming \
         the record its answer would have been written under: {observed}"
    );
    // **The budget is armed whole every time the wait is planted**, which is the
    // half a hub restart decides: a process that re-derives an unanswered pause
    // plants it again, exactly as a resumed generation re-parks a local wait
    // nobody answered (`docs/durability.md` §5) — and PRD resolved q46 does not
    // let a placement decide "how long do I have". The second planting of one
    // identity is what stands in for the restart here, and it lasts the node's
    // own thirty milliseconds rather than the nothing its predecessor left.
    assert_eq!(observed["remote_replanted"]["first"], json!("resolved"));
    assert_eq!(observed["remote_replanted"]["replanted"], json!("resolved"));
    let lasted = observed["remote_replanted"]["lasted"]
        .as_i64()
        .unwrap_or(-1);
    assert!(
        lasted >= 25,
        "a re-planted wait lasted {lasted}ms of the thirty its node declares, so a hub restart \
         spends a person's budget on the downtime it was not open for: {observed}"
    );

    // The two settlements that are the run's own shape rather than the
    // composition's reach a remote pause exactly as they reach a local one.
    assert_eq!(
        observed["remote_unsettled"],
        json!({
            "abandoned": "HumanAbandoned",
            "withdrawn": "HumanInterrupt",
            "unanswerable": "HumanInterrupt",
        }),
        "a worker's pause survives a run that stopped waiting for it: {observed}"
    );

    // …and the other end of the wire: the identity a worker sends home is the
    // one the **same** `runHuman` opens locally at the same view, which is the
    // whole of what "the same wait identity derivation" means.
    assert_eq!(
        observed["travelling"]["opened"],
        json!(["escalate/0/sign/0"])
    );
    assert_eq!(
        observed["travelling"]["carried"],
        json!({
            "name": "RemoteHumanPause",
            "wait": "escalate/0/sign/0",
            "flow": "flow.sign_off",
            "node": "sign",
            "shown": { "question": "ship it?" },
            "effect": {
                "key": "escalate/0/sign/0#human/0",
                "site": "escalate/0/sign/0",
                "ordinal": 0,
                "request": "{\"node\":\"sign\"}",
            },
            "travels_as_an_interrupt": true,
        }),
        "a pause reached where pauses settle home did not carry what §3.4 puts on the wire: \
         {observed}"
    );
    // Nothing was parked where the pause travelled: a worker holds no board, and
    // a wait left on one there is a question no surface could ever reach.
    assert_eq!(observed["travelling"]["published"], json!([]));
}

/// Gate 20: the terminal a `run` answers a pause at, driven directly.
///
/// The acceptance suite answers pauses through the real `agent-compose run`,
/// which is where "the command behaves" is decided. This gate exists for the
/// half that is **the engine's**: the prompt loop is a reader over
/// `process.stdin` — a `data` event, an `end` event, an encoding — and this
/// project supports two runtimes (PRD §9.18). A line reader that chunked
/// differently, or that never saw the `end` of a stream, would leave one of the
/// two supported readers with a `run` that hangs on a question it printed.
///
/// So the same runner answers under Bun here and under Node in gate 13, with the
/// same assertions ([`the_terminal_asked_and_took_every_answer`]), over
/// `src/cli.ts` and `src/runtime.ts` — compiler constants, byte-identical in
/// every project this release builds.
#[test]
fn the_terminal_prompt_loop_asks_reads_and_delivers_every_answer() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "interactive-pause");
    let output = runner("interactive-pause.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the terminal answer surface did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let observed: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints its observations as JSON");
    the_terminal_asked_and_took_every_answer(&observed);
}

/// Everything gate 20 asks of the terminal answer surface, as a function of the
/// runner's answer.
///
/// Factored out for the reason [`the_wait_board_behaved`] is: gate 13 re-runs
/// this runner under the Node fallback, and two columns that asserted separately
/// could come to different verdicts by drifting apart rather than by the
/// runtimes disagreeing.
fn the_terminal_asked_and_took_every_answer(observed: &Value) {
    // The prompt is what a person is given, so all four parts of it are pinned:
    // which pause this is, where it is, what they are shown, and what their
    // answer has to fit. The last is a *sketch* of the node's `output:` rather
    // than its JSON Schema — the schema is what the status route hands a program
    // — and `note?` is the half of it that says which properties the schema does
    // not require.
    let asked = observed["prompted"]["said"]
        .as_str()
        .expect("the prompt is text");
    for part in [
        "pause `review/0/sign/0` — flow.sign_off node `sign`",
        "\"question\": \"ship it?\"",
        "\"draft\": \"a draft\"",
        "answer: { decision: \"approve\" | \"reject\", note?: string }",
        "answer `review/0/sign/0` with one line of JSON: ",
    ] {
        assert!(
            asked.contains(part),
            "the prompt is missing `{part}`:\n{asked}"
        );
    }
    assert!(
        !asked.contains("expires:"),
        "a node declaring no `timeout:` has no deadline to show (grammar 8.7):\n{asked}"
    );

    // …and one line of JSON answers it, exactly as a resume does: the value
    // reaches the node's result and the pause records `"resumed"`.
    assert_eq!(observed["prompted"]["settled"], json!("resolved"));
    assert_eq!(
        observed["prompted"]["output"],
        json!({ "decision": "approve", "note": "looks right" })
    );
    assert_eq!(observed["prompted"]["pause"], json!("resumed"));
    assert_eq!(observed["prompted"]["published"], json!([]));

    // A line that is not JSON, an answer the node's `output:` refuses, and a
    // blank line are three ways of not answering, and none of them consumes the
    // wait: the fourth line does, which it could not if any of the three had.
    let refused = observed["refused"]["said"]
        .as_str()
        .expect("the refusals are text");
    assert!(
        refused.contains("an answer is one line of JSON, and this line is not one: "),
        "a line that does not parse is refused as one:\n{refused}"
    );
    assert!(
        refused.contains(
            "that answer does not fit the `human` node's `output:`, so `review/0/sign/0` is still \
             waiting for one that does: "
        ),
        "…and one the schema refuses is the resume route's `400` at this surface:\n{refused}"
    );
    assert_eq!(
        observed["refused"]["prompts"],
        json!(4),
        "the question, then one re-prompt after each of the three lines that did not answer \
         it:\n{refused}"
    );
    assert_eq!(observed["refused"]["settled"], json!("resolved"));
    assert_eq!(
        observed["refused"]["output"],
        json!({ "decision": "reject" })
    );

    // Two pauses **waiting together** are asked one at a time and in wait-id
    // order — the order the status route publishes them in, which is the
    // composition's rather than the scheduler's. They were opened in the other
    // order.
    assert_eq!(
        observed["two"]["asked"],
        json!(["fan/0/0/sign/0", "fan/0/1/sign/0"])
    );
    assert_eq!(
        observed["two"]["after_the_first"],
        json!({
            "first": "resolved",
            "second": "pending",
            "published": ["fan/0/1/sign/0"],
        }),
        "answering one pause leaves the other waiting, and it is the one still published"
    );
    assert_eq!(
        observed["two"]["outputs"],
        json!([{ "decision": "approve" }, { "decision": "reject" }]),
        "each answer reached the pause it was typed for"
    );

    // …and the boundary of that guarantee, which the section above cannot see:
    // a pause that opens **while a question is on the screen** is asked after
    // it, even though its id sorts first. The order is over the pauses open when
    // a question is asked, not over every pause the run makes — the alternative
    // is withdrawing a question somebody may already be answering, and grammar
    // 8.7 and PRD §9.21 both say so in exactly these terms.
    assert_eq!(
        observed["later"]["asked"],
        json!(["fan/0/1/sign/0", "fan/0/0/sign/0"]),
        "a pause that opened under a question on the screen is asked after it"
    );
    assert_eq!(
        observed["later"]["outputs"],
        json!([{ "decision": "approve" }, { "decision": "reject" }]),
        "…and the first line answered the question that was on the screen: a loop that \
         re-ordered on the latecomer would have given `approve` to `fan/0/0/sign/0`"
    );

    // A budget that ran out while the question was on the screen withdraws it —
    // with the sentence a late answer would have been refused with — and the
    // loop asks the next pause rather than reading a line into a wait nothing is
    // holding.
    assert_eq!(observed["expired"]["settled"], json!("HumanExpiry"));
    assert_eq!(
        observed["expired"]["expires_at_was_shown"],
        json!(true),
        "a node declaring a `timeout:` shows the deadline it published"
    );
    let withdrawn = observed["expired"]["said"]
        .as_str()
        .expect("the withdrawal is text");
    assert!(
        withdrawn.contains(
            "this question is withdrawn: the wait at `review/0/sign/0` expired, and `on_timeout` \
             has already routed the execution on (grammar 8.7)"
        ),
        "the prompt says what happened to it:\n{withdrawn}"
    );
    assert_eq!(
        observed["expired"]["next"],
        json!("resolved"),
        "…and the loop moved on: the pause after it was asked and answered on the same stream"
    );
    assert_eq!(
        observed["expired"]["next_output"],
        json!({ "decision": "approve" })
    );

    // Standard input ending is the answer surface going away, which is the same
    // shape as a run that never had one (grammar 8.7): the pause it was showing
    // becomes the interrupt, and so does the next pause the run opens.
    assert!(
        observed["input_ended"]["said"]
            .as_str()
            .unwrap_or_default()
            .contains("standard input ended, so nothing can answer this run's pauses any more."),
        "{}",
        observed["input_ended"]
    );
    assert_eq!(observed["input_ended"]["settled"], json!("HumanInterrupt"));
    assert!(
        observed["input_ended"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("is waiting for a human and this run has no way to answer"),
        "{}",
        observed["input_ended"]
    );
    assert_eq!(
        observed["input_ended"]["later"],
        json!("HumanInterrupt"),
        "the board is unresumable from then on, so a later pause raises where it opens \
         rather than parking on a question nothing can answer"
    );
    assert_eq!(observed["input_ended"]["published"], json!([]));

    // An end that arrived **between** questions is noticed before the next one
    // is rendered. The loop hears about it on the stream's own event, and parked
    // with no pause open it has no read outstanding for that event to answer —
    // so the check is the one thing standing between an operator and a whole
    // prompt block printed under a surface that was already gone.
    let between = observed["ended_between"]["said"]
        .as_str()
        .expect("the surface's last words are text");
    assert_eq!(
        observed["ended_between"]["answered"],
        json!("resolved"),
        "the pause before the end was answered normally: {between}"
    );
    assert!(
        between.contains("pause `review/0/sign/0`"),
        "…so its question was asked:\n{between}"
    );
    assert!(
        !between.contains("pause `review/1/sign/0`"),
        "…and the pause that opened after standard input ended is never rendered: a question \
         printed and withdrawn on the line under it is one that was never askable:\n{between}"
    );
    assert!(
        between.contains("standard input ended, so nothing can answer this run's pauses any more."),
        "{between}"
    );
    assert_eq!(
        observed["ended_between"]["later"],
        json!("HumanInterrupt"),
        "…and it raises where it opened, exactly as a pause a run with no surface reaches"
    );

    // A stream whose last line carries no newline is still an answer: `printf
    // '{"decision":"approve"}'` is a script that answered, and a reader that only
    // took lines up to a newline would sit on it until the stream closed.
    assert_eq!(
        observed["unterminated"],
        json!({ "settled": "resolved", "output": { "decision": "approve" } })
    );
}

/// Gate 21: the prompt loop over a **real** `process.stdin`.
///
/// Gate 20 drives the same loop over a `node:stream` `PassThrough`, which is
/// what lets it schedule a pause to the instant. What it cannot ask is whether
/// the loop works on the stream `src/cli.ts` actually hands it: a process's
/// standard input is a pipe the operating system owns and the runtime wires
/// into its event loop, and three of its properties are the engine's rather
/// than the reader's — that a `data` listener attached at the *first question*
/// still sees bytes that arrived before it, that `end` fires on a pipe whose
/// EOF preceded that listener, and that `Lines.stop()` lets go of the handle so
/// the process **exits**.
///
/// The last is why this gate spawns rather than calling `output()`: the pipe is
/// held open from this side for the whole of the child's life, so nothing but
/// the loop's own `pause()` can release it. A run that printed its answer and
/// then sat on a stream it had finished with is the failure shape a test cannot
/// tell from a slow one, and it is the one an operator meets as "it never came
/// back" — so the runner arms an unref'd timer of its own and exits `9` on a
/// loop that stayed alive, which arrives here as a code and a sentence.
///
/// Both cases run under Bun here and under Node in gate 13, with the same
/// assertions ([`the_real_standard_input_was_read_and_released`]).
#[test]
fn the_prompt_loop_reads_and_releases_the_processs_own_standard_input() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "interactive-stdin");
    the_real_standard_input_was_read_and_released(&|which, typed| {
        over_a_real_pipe(runner("interactive-stdin.mjs"), &project, which, typed)
    });
}

/// Everything gate 21 asks of the loop over a process's own standard input, as
/// a function of a way to run the runner.
///
/// Factored out for the reason [`the_wait_board_behaved`] is: gate 13 re-runs
/// both cases under the Node fallback, and two columns that asserted separately
/// could come to different verdicts by drifting apart rather than by the
/// runtimes disagreeing.
fn the_real_standard_input_was_read_and_released(run: &dyn Fn(&str, Option<&str>) -> Value) {
    // A line written into the child's pipe answers the pause, and the process
    // ends — with this side still holding the write end open, so what let go of
    // standard input was the loop.
    let answered = run("answers", Some("{\"decision\":\"approve\"}\n"));
    assert_eq!(
        answered["settled"],
        json!("resolved"),
        "a line on the real pipe answers the pause: {answered}"
    );
    assert_eq!(answered["output"], json!({ "decision": "approve" }));
    assert_eq!(
        answered["pause"],
        json!("resumed"),
        "…and it records exactly as a resume does (docs/trace.md §3)"
    );
    assert!(
        answered["said"]
            .as_str()
            .unwrap_or_default()
            .contains("answer `review/0/sign/0` with one line of JSON: "),
        "the question really was asked: {answered}"
    );

    // A pipe closed before the loop ever attached to it: the EOF is older than
    // the `data` listener, which is the one ordering a lazily-attached reader
    // can miss. Missing it is a run that hangs on a question it printed, so it
    // is asserted as the outcome a run with no surface has.
    let ended = run("eof", None);
    assert_eq!(
        ended["settled"],
        json!("HumanInterrupt"),
        "an EOF that preceded the reader still ends the surface: {ended}"
    );
    // …and the whole of what it said is that sentence. The surface was gone
    // before the run started, so a question was never askable, and a block
    // printed in full — the wait id, the `shown:` payload, the schema, the
    // deadline — with the withdrawal on the line under it is the shape
    // `Lines.spent()` exists to prevent. Asserted as an equality rather than as
    // an absence of "pause `": the `end` event cannot fire before the listener
    // that hears it is attached, so *anything* written before this sentence
    // means the loop asked its first question a turn too early.
    assert_eq!(
        ended["said"],
        json!("\nstandard input ended, so nothing can answer this run's pauses any more.\n"),
        "an EOF that preceded the reader withdraws the surface without rendering a prompt: {ended}"
    );
}

/// Run `interactive-stdin.mjs` with a real pipe on the child's standard input.
///
/// `typed` is what is written into it: `Some` writes the line and **keeps the
/// write end open** past the child's exit, so the only thing that can release
/// the child's standard input is the child; `None` closes it immediately, which
/// is the EOF-before-the-reader case.
///
/// `Child::wait` closes a child's standard input before waiting to avoid a
/// deadlock, so the handle is taken out of the child first — this function's
/// whole subject is what happens when nobody else closes it.
fn over_a_real_pipe(
    mut command: Command,
    project: &Path,
    which: &str,
    typed: Option<&str>,
) -> Value {
    use std::io::{Read, Write};

    let mut child = command
        .arg(project)
        .arg(which)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the runtime runs");
    let mut writing = child.stdin.take().expect("the child's input is a pipe");
    match typed {
        Some(line) => {
            writing
                .write_all(line.as_bytes())
                .expect("the pipe takes it");
            writing.flush().expect("the pipe takes it");
        }
        None => drop(writing),
    }
    let status = child.wait().expect("the child ends");
    let mut said = String::new();
    child
        .stdout
        .take()
        .expect("the child's output is a pipe")
        .read_to_string(&mut said)
        .expect("the report is text");
    let mut complained = String::new();
    child
        .stderr
        .take()
        .expect("the child's errors are a pipe")
        .read_to_string(&mut complained)
        .expect("the complaint is text");
    assert!(
        status.success(),
        "the `{which}` case of the real-standard-input runner failed ({status}):\n{complained}"
    );
    serde_json::from_str(&said).expect("the runner prints its observations as JSON")
}

/// Gate 2e: what the single string-typed property of a `tool.*` binds, on each
/// of the two surfaces that can implement one (grammar 6.1).
///
/// The grammar states the exception once per binding and the two sentences
/// differ by a word: `exec:` binds "trimmed raw stdout", `http:` binds "the raw
/// response text". A shared decoder makes it easy for one reading to be applied
/// to both by accident and for nobody to notice — trailing whitespace is a shell
/// artefact on stdout, where a command that ends its output with a newline has
/// said nothing by it, and payload in a response body, where every byte is what
/// the server chose to send. So both are run, over one payload with whitespace
/// at either end, and the difference is asserted rather than assumed.
#[test]
fn a_raw_binding_trims_stdout_and_takes_a_response_body_verbatim() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "raw-decoding");

    let output = runner("raw-decoding.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the raw-binding runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");
    a_raw_binding_bound(&answer);
}

/// The verdict `raw-decoding.mjs` has to come back with, whichever runtime ran
/// it.
///
/// A function rather than a body, because gate 13 runs the same runner under
/// Node: what a binding binds is the *runtime's* answer — `child_process`,
/// `fetch` and a `TextDecoder` all belong to the engine — so a verdict that held
/// under one and not the other is a composition that reads differently for half
/// its readers. Sharing the assertions is what keeps the two runs one claim
/// instead of two that can drift.
fn a_raw_binding_bound(answer: &Value) {
    let sent = answer["sent"].as_str().expect("the payload it sent");
    assert_ne!(
        sent.trim(),
        sent,
        "the payload has to carry whitespace for either reading to be visible"
    );
    assert_eq!(
        answer["exec"]["text"].as_str(),
        Some(sent.trim()),
        "an `exec` implementation binds trimmed raw stdout (grammar 6.1)"
    );
    assert_eq!(
        answer["http"]["text"].as_str(),
        Some(sent),
        "an `http` implementation binds the raw response text — every byte of \
         it, which is what its own sentence says"
    );
}

/// The artifact's content hash means the same thing in both languages
/// (`docs/distributed.md` §3.5, §4).
///
/// The rule has two implementations by construction and neither is optional:
/// `compose_core::codegen::artifact::hash` writes the constant when the compiler
/// emits, and `contentHash` in the emitted `src/mesh.ts` is what a worker
/// re-derives from the entries it unpacked "before unpacking anything" (§4
/// step 2). A worker whose answer differed by a byte would refuse every artifact
/// a hub serves it — or, the other way round, materialise one it had not really
/// verified — so this is the CEL corpus's discipline applied to the second pair
/// of implementations this project has.
///
/// Asked of the **mesh** golden, because that is the project whose answer a
/// worker will act on; the rule itself is composition-independent, which the
/// unit tests beside `artifact::hash` cover.
#[test]
fn the_artifact_hash_is_the_same_in_both_languages() {
    let Some(root) = installed() else {
        return;
    };
    let golden = goldens::golden("placed-nodes");
    let project = staged(golden, root, "artifact-hash");

    let output = runner("artifact-content-hash.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the artifact-hash runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");

    let emitted = goldens::emitted(golden);
    // The **whole tree**, generated and carried alike: PRD resolved q49 widened
    // the artifact to "what `build` wrote plus the authored files the spec
    // references", and this golden binds one. A hash taken over half of it would
    // agree with nothing — not with the constant the emitter wrote, and not with
    // what a worker computes from the entries it unpacked.
    let tree: Vec<compose_core::GeneratedFile> = emitted.artifact().cloned().collect();
    assert!(
        !emitted.carried().is_empty(),
        "`{}` carries no authored file, so this says nothing about the widened list",
        golden.directory
    );
    let declared = answer["declared"].as_str().expect("the declared hash");
    assert_eq!(
        declared,
        compose_core::codegen::artifact::hash(&tree),
        "the constant the emitter wrote is not the hash it computes"
    );
    assert_eq!(
        answer["computed"].as_str(),
        Some(declared),
        "the emitted project hashes its own tree to something other than the hash it declares: \
         a worker verifying what it unpacked would refuse the artifact this hub serves"
    );
    let mut served: Vec<&str> = answer["files"]
        .as_array()
        .expect("the file list")
        .iter()
        .map(|path| path.as_str().expect("a path"))
        .collect();
    served.sort_unstable();
    assert_eq!(
        served,
        emitted.paths().collect::<Vec<_>>(),
        "`ARTIFACT_FILES` is not the set this build emitted"
    );
}

/// The dispatch board's park order, its two settle verbs, and the payload it
/// carries (`docs/distributed.md` §3.2, §3.4, §6.2).
///
/// Three properties a served hub cannot show, driven against `src/journal.ts`
/// directly — a compiler constant, byte-identical in every project, so this is
/// what every project runs:
///
///   * **park order is insertion order under a tie.** §6.2's "dispatch resumes
///     in park order" is what a joining worker's scan follows and what §6.4's
///     undispatched-is-a-pause row leans on for fairness. `parked_at` has
///     millisecond resolution and a fan-out parks every instance from one
///     synchronous burst, so they share it; the tiebreak that decides them has
///     to be the order they went on the board. Twelve waits, because the
///     tiebreak this replaced was a **string** compare over `<instance
///     path>/<ordinal>` — `sign/10` sorts before `sign/2` — so a corpus of four
///     would agree with either rule, and no fixture a served hub runs fans out
///     wide enough to tell them apart.
///
///   * **`settleDispatch` answers whether *this* call settled it.** Its contract
///     says so and the difference is invisible over the wire, because
///     `/workers/result` reads the row's status before it calls: a re-posted
///     result is `204` either way. A future caller that trusted the answer to
///     tell a first settle from a re-post would take a second result's outcome
///     as newly journaled — and the row must keep the outcome it has, which is
///     asserted beside it.
///
///   * **`releaseDispatch` is a claim undone, and nothing else.** §7 makes
///     dispatch at-least-once — "a hub that cannot tell whether a dispatch
///     arrived re-issues it" — and the poll's own guard re-issues by putting the
///     row back where a worker hung up before the answer was written. Three
///     things have to hold of that and none is reachable over HTTP: the row
///     returns to **its own place** in the park order rather than to the end of
///     the queue, only the session holding it may hand it back, and a row that
///     has moved on since — settled by a result, superseded by a deadline — is
///     left exactly as it is.
///
///   * **§3.2's four OPTIONAL payload fields are on the row**, and a row parked
///     without them carries none. That is what lets the poll answer omit the
///     keys rather than send `null`, and it is why a hub restarted mid-dispatch
///     hands over what its predecessor would (§8 rule 3: no dispatch state
///     anywhere but the journal).
#[test]
fn the_dispatch_board_resumes_in_park_order_and_settles_once() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("placed-nodes"), root, "dispatch-board");

    let output = runner("dispatch-board.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the dispatch board did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let observed: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");

    let order = &observed["parkOrder"];
    assert_eq!(
        order["order"], order["wanted"],
        "the board does not resume in park order: a one-worker pool would run a `map`'s items \
         0, 1, 10, 11, …, 2, 3, and the item that has waited longest — whose `timeout:` has been \
         running longest — is not the one taken next (docs/distributed.md §6.2)"
    );
    assert_eq!(
        order["ofExecution"], order["wanted"],
        "`dispatchesOf` promises park order too, and a status report reads it"
    );
    let insertion: Vec<i64> = order["insertion"]
        .as_array()
        .expect("the runner reports the insertion order it read back")
        .iter()
        .map(|held| {
            held.as_i64().unwrap_or_else(|| {
                panic!("a row read out of the journal carries no `order`: {order:#}")
            })
        })
        .collect();
    assert!(
        insertion.windows(2).all(|pair| pair[0] < pair[1]),
        "the tiebreak the board sorts by is not carried on the rows it answers with, so a \
         synchronous reader of them — the status report — has to invent one of its own and can \
         publish an order the queue does not drain in (docs/distributed.md §6.2): {order:#}"
    );

    let release = &observed["release"];
    assert_eq!(release["claimed"], json!("dispatched"), "{release:#}");
    assert_eq!(release["claimedBy"], json!("wrk_one"), "{release:#}");
    assert_eq!(
        release["releasedByAnother"],
        json!(false),
        "a session that is not holding the row handed it back, so any worker could take work off \
         another's session (docs/distributed.md §7): {release:#}"
    );
    assert_eq!(release["released"], json!(true), "{release:#}");
    assert_eq!(release["status"], json!("parked"), "{release:#}");
    assert_eq!(
        release["session"],
        json!("absent"),
        "a row put back on the board still names the session that could not receive it: {release:#}"
    );
    assert_eq!(release["dispatchedAt"], json!("absent"), "{release:#}");
    assert_eq!(
        release["orderAfterRelease"], release["wantedAfterRelease"],
        "a released row did not go back to its own place in the park order: the item that has \
         waited longest is no longer the one taken next, and its `timeout:` has been running the \
         whole time (docs/distributed.md §6.2, §6.5): {release:#}"
    );
    assert_eq!(
        release["releasedSettled"],
        json!(false),
        "a row a worker's result already settled was put back on the board: {release:#}"
    );
    assert_eq!(release["settledStatus"], json!("settled"), "{release:#}");
    assert_eq!(
        release["releasedSuperseded"],
        json!(false),
        "a row the hub superseded was put back on the board, so work the execution has gone past \
         would be dispatched again (docs/distributed.md §6.3): {release:#}"
    );
    assert_eq!(
        release["supersededStatusAfter"],
        json!("superseded"),
        "{release:#}"
    );

    let settlement = &observed["settlement"];
    assert_eq!(
        settlement["first"],
        json!(true),
        "the call that settled the dispatch did not say so"
    );
    assert_eq!(
        settlement["again"],
        json!(false),
        "a row an earlier result already settled answers `true`, so a caller cannot tell a first \
         settle from a re-post (docs/distributed.md §3.4)"
    );
    assert_eq!(
        settlement["outcome"]["value"]["signature"],
        json!("s"),
        "the second settle overwrote the outcome the first one journaled"
    );
    assert_eq!(
        settlement["afterSupersede"],
        json!(false),
        "a superseded row answers `true`, which is the `409` case reading as the `204` one"
    );
    assert_eq!(settlement["supersededStatus"], json!("superseded"));
    assert_eq!(
        settlement["unknown"],
        json!(false),
        "a `dispatch_id` this journal never held answers `true`"
    );

    let payload = &observed["payload"];
    assert_eq!(payload["itemIndex"], json!(3));
    assert_eq!(
        payload["history"],
        json!([{ "role": "assistant", "text": "a release of release.dmg" }]),
        "the conversation a placed `agent:` node is dispatched with did not survive the journal"
    );
    assert_eq!(payload["policy"], json!({ "timeoutMs": 30_000 }));
    for field in ["itemIndex", "history", "policy"] {
        assert_eq!(
            payload["absent"][field],
            json!("absent"),
            "a row parked without §3.2's `{field}` reads back carrying one, so the poll answer \
             would send a value where the document omits a key"
        );
    }
}

/// Gate 2f: a command that never reads its input still completes.
///
/// Grammar 8.2 sends a scalar `input:` to the child's standard input, and no
/// command is obliged to drain it. When one does not, the pipe closes under a
/// write still in flight and Node reports EPIPE as an `error` **event on the
/// stream** — outside the promise `runExec` settles, so grammar 9's policies
/// cannot see it: unhandled, it aborts the process past `retry`, `timeout`,
/// `skip` and `fallback` alike, past the `catch` that writes the run's trace
/// (PRD 5.3), and under `serve` it would end every concurrent execution.
///
/// That sentence is about **Node**, and this gate runs under Bun, which is why
/// the assertions live in [`the_unread_input_was_survived`] and gate 13 hands
/// them Node's own answer to the same runner. How a stream delivers a write that
/// failed is the engine's to decide, so a gate about it that asked only one
/// engine would be evidence for whichever half of the claim it happened to run.
///
/// A gate rather than a unit test because the failure is a property of the
/// **process**: nothing about the returned value is wrong, the returned value
/// never arrives. And a payload larger than a pipe buffer rather than a
/// convenient one, because a small write lands in the kernel's buffer and
/// succeeds whether or not anybody reads it — which is exactly how this shipped.
#[test]
fn a_command_that_never_reads_its_input_still_completes() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "unread-stdin");

    let output = runner("unread-stdin.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "writing to a command that does not read its input killed the process:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");
    the_unread_input_was_survived(&answer);
}

/// The verdict `unread-stdin.mjs` has to come back with, whichever runtime ran
/// it — shared with gate 13 for the reason [`a_raw_binding_bound`] gives, and
/// most sharply here: which of a rejected promise and an `error` event an EPIPE
/// arrives as is a property of the engine's own stream implementation, so this is
/// the gate whose subject is least the compiler's and most the runtime's.
fn the_unread_input_was_survived(answer: &Value) {
    assert!(
        answer["sent"].as_u64().is_some_and(|bytes| bytes > 65_536),
        "the payload has to exceed a pipe buffer for the write to fail at all, \
         found {} bytes",
        answer["sent"]
    );
    assert_eq!(
        answer["result"]["exit_code"], 0,
        "the child's own exit code is the node's outcome: declining the input is \
         not a failure (grammar 8.2)"
    );
    assert_eq!(
        answer["result"]["stdout"], "done",
        "and its output is what the node decodes"
    );
}

/// Gate 2j: a failure the **platform** worded carries no resolved `${ENV}`
/// value.
///
/// `docs/trace.md` §11.1 promises that no resolved environment value appears in
/// a trace, and §11.2 names the three failures where one would have: a command
/// the OS refused to spawn, an `http:` `url` that is not a URL once its
/// references resolve, and a provider whose resolved `base_url:` `fetch` cannot
/// parse. Each embeds the resolved string in the message the *engine* composes,
/// so each is caught and restated.
///
/// A fourth message rides along for the opposite reason: an `${ENV}` reference
/// that is **not set** is worded by this runtime, precisely, and a restatement
/// standing in front of that one would replace a diagnosis with a
/// misattribution. See [`the_unset_reference_was_named_rather_than_restated`].
///
/// A gate rather than a source-level check — that half is
/// `tests/trace_format_inventory.rs`'s
/// `a_failure_the_platform_worded_is_restated_rather_than_quoted` — because the
/// wording is the engine's and the two supported engines word them differently:
/// Bun quotes the resolved string a `new URL` could not parse and Node says only
/// `Invalid URL`, while Node quotes the endpoint `fetch` could not parse and Bun
/// says only `fetch() URL is invalid`. A promise about a public surface that
/// held on one runtime and not the other would be no promise at all, so gate 13
/// asks the same question of Node.
#[test]
fn no_failure_the_platform_worded_carries_a_resolved_env_value() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "resolved-env");

    let output = runner("resolved-env-in-failures.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the probe did not reach all three failures under Bun:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    the_resolved_values_stayed_out_of_the_messages(
        &serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object"),
    );
}

/// The verdict `resolved-env-in-failures.mjs` has to come back with, whichever
/// runtime ran it.
///
/// Two claims per message, and the second is what keeps the first from being
/// satisfied by saying nothing: the resolved value is **absent**, and the
/// reference — or, for the provider, its typed address — is **present**. A
/// runtime that answered `an activity failed` would pass a check for the secret
/// alone and leave a reader with nothing to fix (PRD G3).
fn the_resolved_values_stayed_out_of_the_messages(answer: &Value) {
    let secret = answer["secret"]
        .as_str()
        .unwrap_or_else(|| panic!("the runner names the value it planted: {answer}"));
    for (surface, names) in [
        ("exec", "${OPS_BIN}"),
        ("http", "${SIGNED_ENDPOINT}"),
        ("provider", "provider.acme"),
    ] {
        let message = answer[surface]
            .as_str()
            .unwrap_or_else(|| panic!("the runner reports the `{surface}` failure: {answer}"));
        assert!(
            !message.contains(secret),
            "the `{surface}` failure carries the resolved `${{ENV}}` value, which \
             `docs/trace.md` §11.1 says no field of a trace does: {message}"
        );
        assert!(
            message.contains(names),
            "…and the `{surface}` failure has to name `{names}`, or a reader is told \
             something failed and nothing about what to fix: {message}"
        );
    }
    the_unset_reference_was_named_rather_than_restated(answer);
}

/// The other fault at two of the same sites: a reference that is not set at all.
///
/// A restatement stands between the platform's message and `TraceEntry.error`,
/// and a restatement that also stands in front of *this* message replaces a
/// precise diagnosis with a false one. `runHttp` interpolating inside its `try`
/// would report an unset `${ENV}` as "is not a URL once its `${ENV}` references
/// are resolved" — when nothing resolved — and would word it identically to a
/// reference that *is* set to a non-URL, so a reader of the trace could no
/// longer tell the two apart. `runExec` interpolating its `cwd:` inside the
/// `new Promise` executor would report an unset working directory as
/// "`<command>` could not be run", naming the command for a fault that is not
/// the command's.
///
/// Both halves are asserted for the reason the loop above asserts two: the
/// variable's name has to be **there**, and the restatement has to be **gone**.
/// A check for the name alone would pass on a message that carried both.
fn the_unset_reference_was_named_rather_than_restated(answer: &Value) {
    for (surface, names, site, restatement) in [
        (
            "unsetUrl",
            "`UNSET_ENDPOINT` is not set",
            "`tool.probe` `http.url`",
            "is not a URL once",
        ),
        (
            "unsetCwd",
            "`UNSET_ROOT` is not set",
            "`tool.probe` `exec.cwd`",
            "could not be run",
        ),
    ] {
        let message = answer[surface]
            .as_str()
            .unwrap_or_else(|| panic!("the runner reports the `{surface}` failure: {answer}"));
        assert!(
            message.contains(names) && message.contains(site),
            "an unset reference is diagnosed by naming it and the surface that \
             referenced it — `{names} (referenced by {site})` — which is the whole of \
             what a reader has to fix (PRD G3): {message}"
        );
        assert!(
            !message.contains(restatement),
            "…and `{surface}` restates it as `…{restatement}…`, which is a different \
             fault from the one that happened, and the same words a reference that \
             *is* set produces: {message}"
        );
    }
}

/// Gate 2g: a declared `Content-Type` replaces the runtime's rather than joining
/// it.
///
/// Header names are case-insensitive (grammar 6.1) and `fetch` is not: it builds
/// its `Headers` by appending each key of the object it is handed, so
/// `Content-Type` declared beside the `content-type` `runHttp` sends with a JSON
/// body reaches the server as one field carrying **both** media types. An API
/// that dispatches on it answers 415 to a composition that reads correctly.
///
/// Both directions are asserted, because a runtime that dropped the header
/// handling altogether would pass the first half: the declared media type is
/// what arrives when there is one, and `application/json` is what arrives when
/// there is not.
#[test]
fn a_declared_content_type_replaces_the_one_the_runtime_would_have_sent() {
    let Some(answer) = http_request_gate("declared-headers") else {
        return;
    };
    the_declared_media_type_arrived_alone(&answer);
}

/// The media-type half of `http-request.mjs`'s answer, whichever runtime ran it.
///
/// Shared with gate 13 for the reason [`a_raw_binding_bound`] gives: `fetch` and
/// its `Headers` are the runtime's, and the appending behaviour this is about is
/// a property of that implementation rather than of the emitted code.
fn the_declared_media_type_arrived_alone(answer: &Value) {
    assert_eq!(
        answer["declared"]["header"].as_str(),
        Some("application/vnd.acme+json"),
        "a declared media type is the whole of the header the server sees"
    );
    assert_eq!(
        answer["default"]["header"].as_str(),
        Some("application/json"),
        "and a binding that declares none still says what its body is"
    );
    for which in ["declared", "default"] {
        assert_eq!(
            answer[which]["body"].as_str(),
            Some(r#"{"goal":"g"}"#),
            "the body is the bound object either way"
        );
    }
}

/// Gate 2h: the object a `GET` binding sends becomes the URL's parameters.
///
/// The other half of grammar 6.1's convention for a request the block did not
/// write out: the bound input object is the JSON body on a body-bearing method
/// and the **query string** on `GET`/`HEAD`. Which of the two slots codegen
/// fills is committed in the goldens; this is what the runtime does with the
/// object it was handed — that it reaches the server as parameters at all, and
/// that a value which is not a string is spelled rather than dropped.
#[test]
fn a_bound_input_object_reaches_a_get_as_its_query_string() {
    let Some(answer) = http_request_gate("query-string") else {
        return;
    };
    the_bound_object_arrived_as_parameters(&answer);
}

/// The query-string half of `http-request.mjs`'s answer, whichever runtime ran
/// it — shared with gate 13, because how a `URL`'s `searchParams` spell a
/// non-string and encode a space is the runtime's answer too.
fn the_bound_object_arrived_as_parameters(answer: &Value) {
    let url = answer["query"]["url"]
        .as_str()
        .expect("the URL it received");
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    assert_eq!(path, "/tickets", "the binding's own path is untouched");
    let mut parameters: Vec<&str> = query.split('&').collect();
    parameters.sort_unstable();
    assert_eq!(
        parameters,
        ["limit=3", "term=a+widget"],
        "every property of the bound object is a parameter, percent-encoded, \
         and a number is spelled rather than dropped"
    );
    assert_eq!(
        answer["query"]["body"].as_str(),
        Some(""),
        "and a `GET` sends no body (grammar 6.1)"
    );
}

/// Gate 16: the local store backends do what PRD 5.8 says a store does.
///
/// `src/stores.ts` is where the zero-infra guarantee actually lives, and almost
/// none of it is visible from a golden diff: which partition a `scope:`
/// addresses, whether an idempotency key is honoured, what a `list` answers a
/// prefix with, and whether a key that would climb out of a directory becomes a
/// file name that cannot. A composition that used a store would produce the
/// same emitted bytes with every one of those wrong.
///
/// So the module is driven directly, with bindings the runner writes rather than
/// ones `src/graph.ts` emits: what `graph.ts` emits is already committed and
/// already exercised end to end by the acceptance suite, and what is in question
/// here is the backend underneath it. Gate 13 runs the same runner under Node,
/// because a WebAssembly SQLite over `node:fs` is exactly the kind of dependency
/// that could behave differently on the fallback runtime.
///
/// One claim here is durability's rather than the catalogue's: **a lock a killed
/// writer left behind does not seal the store**. The driver's virtual file
/// system takes SQLite's lock as a directory beside the file, and a `run` killed
/// inside a write never removes it — so the store a resume has to read past its
/// frontier (`docs/durability.md` §5) is one a crash could otherwise render
/// permanently unopenable. The lock is planted rather than raced for, because
/// the window a real crash lands in is one statement wide.
#[test]
fn the_local_store_backends_partition_dedupe_and_encode_what_they_are_given() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("review-loop"), root, "stores");
    let data = root.join("projects").join("stores").join("data");
    let _ = fs::remove_dir_all(&data);

    let output = runner("store-backends.mjs")
        .arg(&project)
        .arg(&data)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the store-backend runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");
    the_local_backends_behaved(&answer);
}

/// What the store-backend runner answered, whichever runtime ran it — shared
/// with gate 13 for the reason [`a_raw_binding_bound`] is: the SQLite here is a
/// WebAssembly module over `node:fs`, and a difference between the two engines
/// would be a difference in what every store in every emitted project does.
fn the_local_backends_behaved(answer: &Value) {
    // The `kv` catalogue of grammar 11.4, row by row.
    assert_eq!(
        answer["kvHit"],
        json!({ "value": { "theme": "dark" }, "found": true })
    );
    assert_eq!(
        answer["kvMiss"],
        json!({ "found": false }),
        "a miss answers `found: false` with **no** `value` at all (Decision D110)"
    );
    assert_eq!(answer["kvList"], json!({ "keys": ["a", "b"] }));
    assert_eq!(
        answer["kvPrefixed"],
        json!({ "keys": ["b"] }),
        "`prefix:` filters, and the answer is in key order"
    );
    assert_eq!(answer["kvDeleted"], json!({ "deleted": true }));
    assert_eq!(
        answer["kvDeletedAgain"],
        json!({ "deleted": false }),
        "a delete of nothing deleted nothing"
    );

    // At-least-once, deduped on grammar 9.4's key: the repeated write answers
    // what the first attempt answered and does **not** happen twice.
    assert_eq!(answer["dedupedWrite"], json!({ "key": "a" }));
    assert_eq!(
        answer["afterDedupedWrite"]["value"]["theme"], "dark",
        "a write whose key the backend had already applied is not applied again"
    );
    assert_eq!(
        answer["afterSecondWrite"]["value"]["theme"], "changed",
        "…and a write under a different key is a different effect"
    );

    // A `prefix:` outside the BMP. SQLite's `substr` counts characters and a
    // JavaScript `.length` counts UTF-16 code units, so a filter that mixed the
    // two answers this legal read with the wrong keys — and grammar 11.4 puts no
    // character restriction on the parameter.
    assert_eq!(
        answer["astralAll"],
        json!({ "keys": ["zzz", "\u{1F600}alpha", "\u{1F600}beta"] }),
        "the keys are there to be filtered in the first place"
    );
    assert_eq!(
        answer["astralPrefix"],
        json!({ "keys": ["\u{1F600}alpha", "\u{1F600}beta"] }),
        "an astral prefix matches the keys that start with it"
    );
    assert_eq!(
        answer["astralDeeper"],
        json!({ "keys": ["\u{1F600}alpha"] }),
        "…and one more character narrows it, rather than sliding off the key"
    );
    assert_eq!(
        answer["astralMiss"],
        json!({ "keys": [] }),
        "a prefix nothing carries still matches nothing"
    );
    assert_eq!(
        answer["astralBlobPrefix"], answer["astralPrefix"],
        "the `blob` backend's `list` answers the same prefix the same way: one op \
         of grammar 11.4's catalogue, two backends"
    );

    // …and in the same **order**, which is the half of that claim a prefix over
    // one astral character does not decide. SQLite compares UTF-8 bytes and a
    // JavaScript sort compares UTF-16 code units, and above the BMP the two
    // disagree: bytes put U+FF00 (`EF BC 80`) before U+1F600 (`F0 9F 98 80`),
    // while the surrogate `D83D` puts U+1F600 first. Grammar 11.4 fixes no
    // order, so neither is wrong on its own — but `list` is one op of one
    // catalogue, and with the `limit:` the row requires the two would answer
    // one store's worth of content with different keys.
    assert_eq!(
        answer["kvOrder"],
        json!(["zz", "\u{FF00}", "\u{1F600}"]),
        "SQLite's `ORDER BY key` is UTF-8 byte order"
    );
    assert_eq!(
        answer["blobOrder"], answer["kvOrder"],
        "and the `blob` backend sorts its file names the same way"
    );
    assert_eq!(
        answer["kvOrderLimited"],
        json!(["zz", "\u{FF00}"]),
        "which is what `limit:` truncates"
    );
    assert_eq!(
        answer["blobOrderLimited"], answer["kvOrderLimited"],
        "so a `limit:` answers the same keys from either backend"
    );

    // A keyed `blob` write whose grammar 9.4 key is longer than a file name.
    // The key is composed from the execution and one frame per enclosing `map`,
    // so its length is the graph's; a ledger that made it a file name directly
    // would fail here *after* applying the effect, leaving the retry below to
    // apply it a second time.
    assert!(
        answer["deepKeyLength"]
            .as_u64()
            .is_some_and(|length| length > 255),
        "the probe's key has to be longer than a file name for this to be the \
         case it is about: {}",
        answer["deepKeyLength"]
    );
    assert_eq!(answer["deepWrite"], json!({ "key": "deep.txt" }));
    assert_eq!(
        answer["deepValue"]["value"], "first",
        "the retry under the same key answered what the first attempt answered \
         and did not overwrite it"
    );
    assert_eq!(
        answer["deepDeduped"],
        json!([false, true]),
        "…which is the ledger doing its job, not the write failing"
    );

    // The records a trace entry carries (PRD 5.8): a read keeps its answer, a
    // write keeps its key and whether the backend had seen it.
    let records = answer["records"]
        .as_array()
        .expect("every op records what it did");
    let write = records
        .iter()
        .find(|record| record["idempotencyKey"] == "k/1")
        .expect("the first write is recorded");
    assert_eq!(write["effect"], "write");
    assert_eq!(write["deduped"], json!(false));
    assert!(
        records
            .iter()
            .any(|record| record["idempotencyKey"] == "k/1" && record["deduped"] == json!(true)),
        "the repeated write is recorded as the duplicate it was: {records:#?}"
    );
    let read = records
        .iter()
        .find(|record| record["effect"] == "read" && record["op"] == "get")
        .expect("a read is recorded");
    assert_eq!(read["answer"]["found"], json!(true));
    assert!(read["idempotencyKey"].is_null(), "a read carries no key");

    // `scope: session` — one store, one partition per session key.
    assert_eq!(
        answer["sessionSame"],
        json!({ "value": { "text": "mine" }, "found": true }),
        "a later execution under the same session key reads what the first wrote"
    );
    assert_eq!(
        answer["sessionOther"],
        json!({ "found": false }),
        "…and another session key is another partition"
    );
    let unkeyed = answer["sessionUnkeyed"]
        .as_str()
        .expect("a session-scoped store with no session identity is refused");
    assert!(
        unkeyed.contains("store.memory") && unkeyed.contains("scope: session"),
        "the refusal names the store (grammar 11.3): {unkeyed}"
    );

    // `scope: execution` — dies with the run, and never crosses to another.
    assert_eq!(answer["executionHit"]["found"], json!(true));
    assert_eq!(
        answer["executionAfterRelease"],
        json!({ "found": false }),
        "what an execution-scoped store held is gone when the run ends"
    );
    assert_eq!(answer["executionOther"], json!({ "found": false }));

    // `blob` — files on disk, keyed reversibly.
    assert_eq!(
        answer["blobHit"],
        json!({ "value": "first", "found": true })
    );
    assert_eq!(answer["blobMiss"], json!({ "found": false }));
    assert_eq!(
        answer["blobList"],
        json!({ "keys": ["notes/one.txt", "notes/two.txt", "other"] }),
        "a `list` answers with the keys that were written, not with what the \
         filesystem made of them"
    );
    assert_eq!(
        answer["blobPrefixed"],
        json!({ "keys": ["notes/one.txt", "notes/two.txt"] })
    );
    assert_eq!(answer["blobDeleted"], json!({ "deleted": true }));
    assert_eq!(
        answer["blobLimited"],
        json!({ "keys": ["notes/one.txt"] }),
        "`limit:` bounds what a `list` answers"
    );
    assert!(
        answer["blobEmptyKey"]
            .as_str()
            .is_some_and(|message| message.contains("empty key")),
        "{}",
        answer["blobEmptyKey"]
    );
    assert!(
        answer["blobLongKey"]
            .as_str()
            .is_some_and(|message| message.contains("one key per file")),
        "a key no filesystem can hold is refused by name: {}",
        answer["blobLongKey"]
    );
    assert_eq!(
        answer["blobEncoded"], "%2E%2E%2Fescape%20me",
        "`.` and `/` are both encoded, so no key becomes a path that climbs out"
    );

    // The lock a killed writer never gave back does not take the store with it.
    // This driver takes SQLite's lock by creating `<file>.lock` as a directory
    // and gives it back by removing it, so a `run` killed inside a write leaves
    // one nothing else will ever remove — and left standing it refuses every
    // later open of that store, the resume of the very execution the crash
    // interrupted included (`docs/durability.md` §2, §5). Without the deadline
    // and the break beneath it the write below fails `SQLITE_BUSY` at once and
    // every later run of the project fails with it.
    assert_eq!(
        answer["sealedWrite"],
        Value::Null,
        "a store a crashed writer left locked refused the write that came next: {}",
        answer["sealedWrite"]
    );
    assert_eq!(
        answer["sealedRead"],
        json!({ "value": { "theme": "dark" }, "found": true }),
        "…and the write it took is the one a later read answers with"
    );
    assert_eq!(
        answer["sealedLockGone"],
        json!(true),
        "…and the corpse is gone rather than waited out once per op"
    );

    // A backend grammar 14.3 names and this release does not implement.
    let production = answer["productionBackend"]
        .as_str()
        .expect("a `redis` backend is refused rather than answered from the wrong store");
    assert!(
        production.contains("redis") && production.contains("M3"),
        "the refusal names the backend and the milestone that lands it: {production}"
    );
}

/// Gate 22: what the two built-in tools do inside one call — `src/runtime.ts`,
/// driven directly.
///
/// The acceptance suite drives these through a model loop, which is where the
/// wire, the trace records and Decision D119's bounces are decided. What a loop
/// cannot show is the inside of a call, and each of these is a claim PRD
/// resolved q54 makes there: that an escaping path leaves everything outside the
/// workspace **untouched**, that a file larger than the runtime reads is viewed
/// from the front and edited not at all, that one shell really is held across
/// the calls of a node activity and not across two, that a `restart` the model
/// asked for ends that session and does not swallow the command sent with it,
/// that a scrubbed child sees the declared variables and no others, that a
/// command's deadline answers the model rather than failing the node, and that a
/// defaulted workspace is one directory per execution that goes when the
/// execution settles.
#[test]
fn the_built_in_tools_are_bounded_by_their_workspace_session_and_deadline() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(goldens::golden("triage-fanout"), root, "builtins");
    let scratch = root.join("projects").join("builtins").join("scratch");
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("the scratch area is writable");

    let output = runner("builtin-tools.mjs")
        .arg(&project)
        .arg(&scratch)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the built-in runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");
    the_built_ins_stayed_inside_their_bounds(&answer);
}

/// What `builtin-tools.mjs` has to come back with.
fn the_built_ins_stayed_inside_their_bounds(answer: &Value) {
    // --- Containment (PRD resolved q54, Decision D119) --------------------
    let containment = &answer["containment"];
    assert_eq!(
        containment["refused"],
        json!({
            "climb": true,
            "absolute": true,
            "symlink": true,
            "dangling": true,
            "read": true,
            "linkedDirectory": true,
            "underLinkedDirectory": true,
            "farUnderLinkedDirectory": true,
            "prefixSibling": true
        }),
        "every way out of the workspace is refused **as a refusal**, which is what goes back to \
         the model rather than failing the node: {containment}"
    );
    assert_eq!(
        (
            &containment["outsideUnchanged"],
            &containment["outsideNotCreated"]
        ),
        (&json!(true), &json!(true)),
        "the file outside the workspace was written through anyway, so the refusals above are a \
         message rather than a bound: {containment}"
    );
    // The two crossings a *parent-only* resolution lets through leave nothing on
    // disk to find by name, because they name directories that did not exist
    // before the call: what says they were refused is that the directory outside
    // the workspace still holds exactly what it held.
    assert_eq!(
        containment["outsideEntries"],
        json!(["secret.txt"]),
        "a `create` through a symlinked directory, at a path whose own parents do not exist yet, \
         made them outside the workspace: `create` makes its parents, so the containment check \
         has to resolve the deepest ancestor that exists rather than stop at the parent \
         (grammar 6.1, PRD resolved q54): {containment}"
    );
    // The crossing every case above is blind to: a *sibling* whose name extends
    // the workspace's, which a resolution catches and a prefix comparison that
    // does not count the separator does not. Everything else here resolves under
    // a different directory outright, so this is the only case that can tell the
    // two comparisons apart — and the write it would have let through lands in a
    // directory the composition never offered (`/srv/work-backup` beside
    // `/srv/work`).
    assert_eq!(
        (
            &containment["siblingUnchanged"],
            &containment["siblingEntries"]
        ),
        (&json!(true), &json!(["secret.txt"])),
        "a `create` at `../<workspace>-evil/secret.txt` reached the sibling directory: a real \
         path that merely *starts* with the workspace's is not inside it, so the containment \
         check has to compare whole path segments (grammar 6.1, PRD resolved q54): {containment}"
    );
    let through = containment["throughLinkMessage"]
        .as_str()
        .unwrap_or_default();
    assert!(
        through.contains("resolves outside this tool's workspace")
            && through.contains("out/deep/nested.txt"),
        "…and the refusal quotes the path the model chose rather than the link it went through \
         (PRD G3): {through}"
    );
    assert_eq!(
        containment["program"],
        json!({
            "tool": "files",
            "operation": "create",
            "path": "../outside/secret.txt"
        }),
        "a refused call still records what the model asked for (`docs/trace.md` §7.4): \
         {containment}"
    );
    let refusal = containment["message"].as_str().unwrap_or_default();
    assert!(
        refusal.contains("resolves outside this tool's workspace") && refusal.contains(".."),
        "the refusal says what was wrong and quotes the path the model chose (PRD G3): {refusal}"
    );

    // --- The file tool, end to end ----------------------------------------
    let editing = &answer["editingTool"];
    assert_eq!(
        editing["created"],
        json!({ "path": "notes/todo.md", "bytes_written": 14, "change": "wrote 14 bytes" }),
        "`create` wrote the whole file, making the directory it named: {editing}"
    );
    assert_eq!(
        editing["viewed"],
        json!({ "path": "notes/todo.md", "content": "     1\tone\n     2\ttwo\n     3\tthree" }),
        "`view` answers with numbered lines, which is what the provider-defined tool answers \
         and what `insert_line` counts: {editing}"
    );
    assert_eq!(
        editing["replaced"],
        json!({
            "path": "notes/todo.md",
            "replaced_at_line": 2,
            "snippet": "     1\tone\n     2\tTWO\n     3\tthree",
            "change": "replaced one occurrence at line 2"
        }),
        "{editing}"
    );
    assert_eq!(
        editing["inserted"],
        json!({
            "path": "notes/todo.md",
            "inserted_after_line": 1,
            "lines_inserted": 1,
            "snippet": "     1\tone\n     2\tone and a half\n     3\tTWO\n     4\tthree",
            "change": "inserted 1 line(s) after line 1"
        }),
        "{editing}"
    );
    assert_eq!(
        editing["onDisk"], "one\none and a half\nTWO\nthree\n",
        "the four operations left the file the edits describe, trailing newline included: \
         {editing}"
    );
    assert_eq!(
        editing["listed"],
        json!({ "path": "notes", "entries": ["todo.md"], "truncated": false }),
        "a `view` of a directory lists it one level deep: {editing}"
    );
    assert_eq!(
        (
            &editing["ambiguousRefused"],
            &editing["missingRefused"],
            &editing["absentRefused"]
        ),
        (&json!(true), &json!(true), &json!(true)),
        "a `str_replace` that matched twice, one that matched nothing, and a `view` of a path \
         that is not there are all the model's to correct: {editing}"
    );
    let ambiguous = editing["ambiguousMessage"].as_str().unwrap_or_default();
    assert!(
        ambiguous.contains("2 times") && ambiguous.contains("surrounding text"),
        "the ambiguous edit's refusal says how many it found and what to do about it: {ambiguous}"
    );
    assert_eq!(
        editing["createdProgram"],
        json!({ "tool": "files", "operation": "create", "path": "notes/todo.md", "change": "wrote 14 bytes" }),
        "the trace's record of an edit is the operation, the path and a **sentence** — never the \
         bytes written (`docs/trace.md` §7.4, §11): {editing}"
    );

    // --- The bound on what one `files` call reads --------------------------
    //
    // The path is the model's, so the size of the read is the model's too unless
    // the runtime bounds it — the same reason the shell's buffer is bounded as
    // its bytes arrive, reached from the other end. `view` takes the front and
    // says so; an edit is refused, because it writes back what it read.
    let heavy = &answer["readBound"];
    assert!(
        heavy["bytesOnDisk"].as_u64().unwrap_or_default() > 4_000_000,
        "the case needs a file past the runtime's read bound to be about anything: {heavy}"
    );
    assert!(
        heavy["viewedLength"].as_u64().unwrap_or_default() < 40_000
            && heavy["saidItStopped"] == json!(true),
        "a `view` of a file larger than the runtime reads answers with a bounded head and says \
         where it stopped, rather than pulling the file into this process: {heavy}"
    );
    assert_eq!(
        (&heavy["editRefused"], &heavy["stillWholeOnDisk"]),
        (&json!(true), &json!(true)),
        "an edit of a file past the read bound is refused **and** leaves the file whole: an edit \
         rewrites what it read, so a truncated read there would truncate the file rather than \
         the answer: {heavy}"
    );
    let heavy_message = heavy["editMessage"].as_str().unwrap_or_default();
    assert!(
        heavy_message.contains("an edit rewrites the whole file") && heavy_message.contains("bash"),
        "…and the refusal says why and names the tool with no such bound (PRD G3, D119): \
         {heavy_message}"
    );

    // --- The shell session (one per node activity) -------------------------
    let session = &answer["session"];
    assert_eq!(
        session["stayedInInner"],
        json!(true),
        "a `cd` in one call is still in effect in the next: one shell per node activity is what \
         makes `cd build && cmake ..` then `make` a thing a model can write: {session}"
    );
    assert_eq!(
        session["remembered"], "[42]",
        "…and so is a variable it set: {session}"
    );
    let fresh = session["freshActivity"].as_str().unwrap_or_default();
    assert!(
        fresh.ends_with("[]") && !fresh.contains("/inner"),
        "a second activity starts in the workspace with none of the first's state — the \
         ordinal-reset rule read for a shell: {session}"
    );
    assert_eq!(
        session["failed"],
        json!({ "stdout": "out\n", "stderr": "err\n", "exit_code": 3 }),
        "a nonzero exit is an **answer**: both streams and the status come back to the model, \
         and the node did not fail: {session}"
    );
    assert_eq!(
        session["failedProgram"],
        json!({
            "tool": "bash",
            "command": "echo out; echo err >&2; (exit 3)",
            "exitCode": 3
        }),
        "…and the trace records the command and the status, which is the q54 ruling c carve-out: \
         {session}"
    );
    let quit = session["quitNotice"].as_str().unwrap_or_default();
    assert!(
        quit.contains("the shell exited") && quit.contains("fresh shell"),
        "a model that ended its own shell is told so, and told what the next call will find, \
         rather than left to wonder where its state went: {session}"
    );

    // --- The restart the model asks for (PRD resolved q54) -----------------
    let restart = &answer["restart"];
    assert_eq!(
        restart["alone"],
        json!({
            "stdout": "",
            "stderr": "",
            "notice": "the shell session was restarted: its working directory is the workspace \
                       again, and no shell state carried over"
        }),
        "a restart on its own answers with what it did and **no** `exit_code`: no command ran, \
         so there is no status to report: {restart}"
    );
    assert_eq!(
        restart["aloneProgram"],
        json!({ "tool": "bash", "command": "restart" }),
        "…and the trace's record of it says the same, which is `docs/trace.md` §7.4's presence \
         rule for `exitCode` — absent where no command completed: {restart}"
    );
    let after = restart["after"].as_str().unwrap_or_default();
    assert!(
        after.ends_with("[unset]") && !after.contains("/inner"),
        "…and the session really ended: the next call is in the workspace with none of the \
         state the restart threw away: {restart}"
    );
    assert_eq!(
        restart["commandRan"],
        json!(true),
        "a `command` sent **with** a restart runs — a runtime that dropped it would answer the \
         model `exit_code: 0` for a write that never happened: {restart}"
    );
    let where_it_ran = restart["whereItRan"].as_str().unwrap_or_default();
    assert!(
        where_it_ran.ends_with("[unset]") && !where_it_ran.contains("/inner"),
        "…in the **fresh** session, which is what the restart beside it asked for: {restart}"
    );
    assert_eq!(
        restart["withCommand"]["exit_code"],
        json!(5),
        "…and it comes back with its own status rather than a manufactured one: {restart}"
    );
    let restarted_notice = restart["withCommand"]["notice"]
        .as_str()
        .unwrap_or_default();
    assert!(
        restarted_notice.contains("the model asked for a restart")
            && restarted_notice.contains("fresh shell"),
        "…and the model is told which session it ran in: {restart}"
    );
    assert_eq!(
        restart["withCommandProgram"]["command"],
        json!("printf \"ran\\n\" > made-by-restart.txt; pwd; echo \"[${kept-unset}]\"; (exit 5)"),
        "…and the trace records the command that ran, with its status, rather than the word \
         `restart`: {restart}"
    );
    assert_eq!(
        restart["withCommandProgram"]["exitCode"],
        json!(5),
        "…with the status it really exited: {restart}"
    );

    // --- A command that prints more than the runtime holds -----------------
    let bounded = &answer["bounded"];
    assert_eq!(
        bounded["exitCode"],
        json!(0),
        "a command that printed megabytes still settled with its status: {bounded}"
    );
    let length = bounded["length"]
        .as_u64()
        .expect("the runner measures what came back");
    assert!(
        length < 200_000,
        "what a call answers with is bounded, and this one came back with {length}          characters: a tool result is text a model reads (PRD resolved q54)"
    );
    assert_eq!(
        (
            &bounded["keptTheHead"],
            &bounded["keptTheTail"],
            &bounded["saidWhatItDropped"]
        ),
        (&json!(true), &json!(true), &json!(true)),
        "the **middle** is what a bound drops: the head is the answer and the tail is          where a command's ending is, and the drop is said rather than silent: {bounded}"
    );
    assert_eq!(
        bounded["after"]["stdout"], "still usable\n",
        "…and the session survived it, which is what says the trim did not eat the          marker that closes a command: {bounded}"
    );

    // --- The scrubbed environment (PRD resolved q54 ruling b) --------------
    let environment = &answer["environment"];
    assert_eq!(
        environment["scrubbed"], "[unset][unset]",
        "a child of a binding that declared nothing sees nothing — not even a variable this \
         process holds: {environment}"
    );
    assert_eq!(
        environment["declared"], "[unset][a value the binding wrote]",
        "…the declared one and no more: {environment}"
    );
    assert_eq!(
        environment["inherited"], "[the value this process holds][unset]",
        "…and `inherit_env: true` is the explicit opt-in that widens it: {environment}"
    );

    // --- The command deadline ---------------------------------------------
    let timeout = &answer["timeout"];
    assert_eq!(
        timeout["result"]["timed_out"],
        json!(true),
        "a command that outran its bound comes back as a tool result: {timeout}"
    );
    assert_eq!(
        timeout["program"],
        json!({ "tool": "bash", "command": "sleep 30", "timedOut": true }),
        "…recorded with the command and no exit status, because none completed: {timeout}"
    );
    let elapsed = timeout["elapsedMs"]
        .as_u64()
        .expect("the runner times the call it made");
    assert!(
        elapsed < 10_000,
        "the 300ms bound was reached at its own deadline rather than at the `sleep`'s \
         (took {elapsed}ms): {timeout}"
    );
    let notice = timeout["result"]["notice"].as_str().unwrap_or_default();
    assert!(
        notice.contains("`300ms`") && notice.contains("fresh shell"),
        "the model is told which bound it hit and that its shell state went with it: {notice}"
    );
    assert_eq!(
        timeout["after"]["stdout"], "still here\n",
        "…and the call after it runs in a shell the runtime opened to replace the one it killed: \
         {timeout}"
    );

    // --- The default workspace --------------------------------------------
    let workspaces = &answer["workspaces"];
    assert_eq!(
        (
            &workspaces["underTheDataDirectory"],
            &workspaces["sharedByBothTools"]
        ),
        (&json!(true), &json!(true)),
        "a binding that wrote no `workspace:` works in one directory per execution, under the \
         project's data directory, shared with every other built-in that took the default: \
         {workspaces}"
    );
    assert_eq!(
        workspaces["keptWhenParked"],
        json!(true),
        "an execution whose row stays open keeps what it wrote, because the generation that \
         resumes it reads past the frontier into those files (`docs/durability.md` §5): \
         {workspaces}"
    );
    assert_eq!(
        workspaces["goneWhenSettled"],
        json!(true),
        "…and a run that ended takes its workspace with it: {workspaces}"
    );
}

/// The runner both `http:` request gates read, run once per gate so each one
/// fails on its own.
fn http_request_gate(purpose: &str) -> Option<Value> {
    let root = installed()?;
    let project = staged(goldens::golden("review-loop"), root, purpose);

    let output = runner("http-request.mjs")
        .arg(&project)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the `http:` request runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    Some(serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object"))
}

/// Gate 7: the channel names the compiler refuses are names LangGraph refuses.
///
/// `codegen::state::INHERITED_PROPERTY_NAMES` is why `agent-compose build`
/// rejects a composition declaring `state:\n  constructor: …`, which the
/// validator accepts and which produces a project that type-checks, loads, and
/// dies at `new StateGraph(State)`. Every other gate here runs over a golden;
/// this one cannot, because the emitter refuses to produce the project — so the
/// channel table is built directly and LangGraph is asked.
///
/// Two claims, both of them things that could quietly stop being true:
///
/// * the list is the object model's own — `Object.getOwnPropertyNames(
///   Object.prototype)` under the default runtime, not a transcription of it.
///   Gate 13 asks Node the same question, because the list is the JavaScript
///   engine's and this compiler's refusal has to hold for both;
/// * every name on it really breaks construction under the pinned LangGraph, and
///   two ordinary names do not. A release that fixed the lookup would fail here,
///   which is the signal to drop the refusal rather than keep it out of habit.
#[test]
fn a_channel_named_after_an_inherited_property_cannot_be_built_at_all() {
    if installed().is_none() {
        return;
    }

    let refused = compose_core::codegen::state::INHERITED_PROPERTY_NAMES;
    // Two names a check matching on shape rather than on membership would take
    // with it: one ordinary channel name, and the near-miss the fixtures use.
    let controls = ["draft", "constructors"];
    let probed: Vec<&str> = refused.iter().copied().chain(controls).collect();

    let output = runner("inherited-channel-names.mjs")
        .arg(serde_json::to_string(&probed).expect("the names serialize"))
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the inherited-name runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");

    let inherited: Vec<&str> = answer["inherited"]
        .as_array()
        .expect("`inherited` is an array")
        .iter()
        .map(|name| name.as_str().expect("a name is a string"))
        .collect();
    assert_eq!(
        inherited, refused,
        "`INHERITED_PROPERTY_NAMES` is not `Object.getOwnPropertyNames(Object.prototype)` under \
         the pinned Node; the refusal is over or under what the object model actually carries"
    );

    let rejected: Vec<&str> = answer["rejected"]
        .as_array()
        .expect("`rejected` is an array")
        .iter()
        .map(|name| name.as_str().expect("a name is a string"))
        .collect();
    assert_eq!(
        rejected, refused,
        "the names `build` refuses are not the names LangGraph refuses; the controls {controls:?} \
         must construct and every inherited name must not"
    );
}

/// Gate 2c: loading the project is what checks its environment (PRD 5.9).
///
/// `src/env.ts` says a presence check exists and the emitted `README.md` says
/// when it runs; both are claims about a call, and a module that declared
/// `readEnvironment` and never called it would satisfy `tsc`, construct its
/// graph, and leave both sentences false. So the gate is the command the README
/// names first: `bun src/index.ts`, once with the composition's variables removed
/// and once with them set. Gate 13 runs the fallback spelling of the same command
/// on one golden, because the claim is about the module rather than the runtime.
#[test]
fn the_generated_project_checks_its_environment_when_it_is_loaded() {
    let Some(root) = installed() else {
        return;
    };

    let mut checked = 0usize;
    for golden in GOLDENS {
        let project = staged(golden, root, "environment");
        let ir = artifact(golden);
        // **The hub's own list, not the composition's whole environment.**
        // `docs/distributed.md` §9.1 partitions the manifest per process, and
        // `readEnvironment()` checks the process this is: a variable only a
        // placement's worker needs is one this deployment cannot leak, so
        // demanding it here would be the false requirement §9.1 is written
        // against. For a composition with no `placements:` the two are the same
        // list, which is every golden but one.
        let partition = compose_core::codegen::env::Partition::of(&ir);
        let references = compose_core::codegen::env::References::for_process(
            &ir,
            &partition,
            &compose_core::codegen::env::Process::Hub,
        );
        let names: Vec<&str> = references.names().collect();

        let mut sealed = bun();
        sealed.arg(project.join("src/index.ts"));
        for name in &names {
            sealed.env_remove(name);
        }
        let sealed = sealed.output().expect("bun runs");

        if names.is_empty() {
            assert!(
                sealed.status.success(),
                "`{}` references no variable and still refused to load:\n{}",
                golden.directory,
                String::from_utf8_lossy(&sealed.stderr),
            );
            continue;
        }

        assert!(
            !sealed.status.success(),
            "`{}` loaded with {:?} unset; the presence check did not run",
            golden.directory,
            names,
        );
        let complaint = String::from_utf8_lossy(&sealed.stderr);
        for name in &names {
            assert!(
                complaint.contains(name),
                "`{}` did not name the missing `{name}`:\n{complaint}",
                golden.directory,
            );
            assert!(
                references
                    .sites(name)
                    .iter()
                    .all(|site| complaint.contains(site)),
                "`{}` named `{name}` without saying where it is referenced:\n{complaint}",
                golden.directory,
            );
        }

        // …and the other direction, which is the half `docs/distributed.md` §9.1
        // exists for: a variable that belongs to a **placement** and to no
        // process this hub is must not be demanded here. `KEYCHAIN_PASSWORD` on
        // a machine that has no keychain is the failure it names; the hub's
        // refusing to start over one it never reads is the same failure wearing
        // the compiler's face.
        for process in partition.processes() {
            if matches!(process, compose_core::codegen::env::Process::Hub) {
                continue;
            }
            let held =
                compose_core::codegen::env::References::for_process(&ir, &partition, process);
            for variable in held.names() {
                if names.contains(&variable) {
                    continue;
                }
                assert!(
                    !complaint.contains(variable),
                    "`{}` refused to start over `{variable}`, which only the placement `{}` runs \
                     anything that reads (docs/distributed.md §9.1):\n{complaint}",
                    golden.directory,
                    process.name(),
                );
            }
        }

        let mut supplied = bun();
        supplied.arg(project.join("src/index.ts"));
        for name in &names {
            supplied.env(name, "supplied");
        }
        let supplied = supplied.output().expect("bun runs");
        assert!(
            supplied.status.success(),
            "`{}` refused to load with every variable set:\n{}",
            golden.directory,
            String::from_utf8_lossy(&supplied.stderr),
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no golden references an environment variable, so nothing exercised the check"
    );
}

/// Gate 17: the project's own command line refuses an option its verb does not
/// take.
///
/// `agent-compose run` is shielded by its own argument parser and sends only the
/// flags it knows; the emitted command line is what PRD 5.12's eject path leaves
/// a reader with, and it is the only launch surface an ejected project has. A
/// `--name` it accepted and never read would be a caller asking for the JSON
/// record and getting human output at exit `0`, with nothing anywhere to say so
/// — which is D50's rule ("a typo in `retrry:` must be a diagnostic, not a
/// silently ignored key") broken on the one surface the compiler does not stand
/// in front of.
///
/// Both verbs, because their option lists are declared separately and a list
/// that went stale on one of them is exactly what this catches. The refusal must
/// also *list what the verb takes*: a reader who mistyped `--format` is one
/// character from the answer, and PRD G3 makes saying so part of the product.
#[test]
fn the_generated_command_line_refuses_an_option_its_verb_does_not_take() {
    let Some(root) = installed() else {
        return;
    };
    let golden = golden("review-loop");
    let project = staged(golden, root, "usage");
    let references = compose_core::codegen::env::References::of(&artifact(golden));

    let launch = |arguments: &[&str]| {
        let mut command = bun();
        command.arg(project.join("src/index.ts"));
        command.args(arguments);
        for name in references.names() {
            command.env(name, "supplied");
        }
        command.output().expect("bun runs")
    };

    for (arguments, verb, mistyped, listed) in [
        (
            ["run", "flow.review_loop", "--fromat", "json"].as_slice(),
            "run",
            "--fromat",
            ["`--input`", "`--session`", "`--format`"].as_slice(),
        ),
        (
            ["serve", "--prot", "8787"].as_slice(),
            "serve",
            "--prot",
            ["`--host`", "`--port`"].as_slice(),
        ),
        // The option table is an object, and every object carries
        // `constructor`. A membership test spelled as a lookup reads that
        // inherited function as an arity and takes the flag — the same
        // property-versus-key confusion `build` refuses a channel named
        // `constructor` over (gate 8), one layer out.
        (
            ["run", "flow.review_loop", "--constructor", "x"].as_slice(),
            "run",
            "--constructor",
            ["`--input`", "`--session`", "`--format`"].as_slice(),
        ),
    ] {
        let output = launch(arguments);
        assert_eq!(
            output.status.code(),
            Some(2),
            "`{arguments:?}` is a command that could not run, which is exit 2:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        assert!(
            output.stdout.is_empty(),
            "…and nothing ran, so stdout carries no answer: {}",
            String::from_utf8_lossy(&output.stdout),
        );
        let complaint = String::from_utf8_lossy(&output.stderr);
        assert!(
            complaint.contains(&format!("`{mistyped}` is not an option of `{verb}`")),
            "the refusal names the flag and the verb:\n{complaint}"
        );
        for option in listed {
            assert!(
                complaint.contains(option),
                "…and lists {option}, which the verb does take:\n{complaint}"
            );
        }
    }
}

/// Every declared divergence is exercised, and every cited one is declared.
///
/// This runs without a toolchain on purpose: it is about the corpus and the
/// module's own table, not about what either column answers, so a machine with
/// no Node still fails when a divergence is invented or abandoned.
#[test]
fn every_divergence_between_the_two_columns_is_declared_and_exercised() {
    let cases = corpus();
    let mut cited: BTreeSet<&str> = BTreeSet::new();
    for case in &cases {
        for document in &case.documents {
            match (document.zod, &document.divergence) {
                (None, None) => {}
                (Some(zod), Some(divergence)) => {
                    assert_ne!(
                        zod, document.valid,
                        "`{}` declares a divergence on `{}` and then gives both columns the same \
                         verdict",
                        case.surface, document.document
                    );
                    assert!(
                        DIVERGENCES.contains(&divergence.as_str()),
                        "`{}` cites `{divergence}`, which `codegen::schema`'s divergence table \
                         does not declare",
                        case.surface
                    );
                    cited.insert(divergence.as_str());
                }
                (Some(_), None) => panic!(
                    "`{}` gives the two columns different verdicts on `{}` without naming a \
                     divergence",
                    case.surface, document.document
                ),
                (None, Some(divergence)) => panic!(
                    "`{}` names the divergence `{divergence}` on `{}` without saying what the Zod \
                     column answers",
                    case.surface, document.document
                ),
            }
        }
    }

    let declared: BTreeSet<&str> = DIVERGENCES.iter().copied().collect();
    assert_eq!(
        declared.len(),
        DIVERGENCES.len(),
        "a divergence is declared twice"
    );
    assert_eq!(
        declared.difference(&cited).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "a divergence is declared and no document exercises it, so nothing says it is still true"
    );
}

/// The corpus's key-order mechanism is used, and says the same thing twice.
///
/// [`Document::text`] asserts that a document's two spellings denote one value,
/// so walking the corpus is what checks it; and a mechanism no case uses is one
/// that could stop working without a failure, so at least one document has to
/// carry `as_written`. Today that is `state.authors`' key-order pair, which is
/// the only place object key order is the subject rather than an accident.
///
/// Runs without a toolchain: it is about the corpus, not about either column.
#[test]
fn a_document_written_out_twice_says_the_same_thing_both_times() {
    let cases = corpus();
    let mut written = 0usize;
    for case in &cases {
        for document in &case.documents {
            let _ = document.text();
            if document.as_written.is_some() {
                written += 1;
            }
        }
    }
    assert!(
        written > 0,
        "no case writes a document out as text, so nothing pins a rule about key order"
    );
}

/// Gate 3: each column of grammar 3.8's table answers what the corpus says it
/// does — and where they differ, that the difference is the declared one.
#[test]
fn the_emitted_zod_agrees_with_the_json_schema_lowering() {
    let cases = corpus();
    assert!(!cases.is_empty(), "the corpus is empty");

    // The Rust column first: it needs no toolchain, so a divergence in the
    // lowering itself is reported even on a machine with no Node.
    let mut expected: Vec<Vec<bool>> = Vec::new();
    for case in &cases {
        let golden = goldens::golden(&case.golden);
        let ir = artifact(golden);
        let surfaces = compose_core::codegen::schema::surfaces(&ir);
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == case.surface)
            .unwrap_or_else(|| panic!("`{}` declares no `{}`", case.golden, case.surface));
        let schema = match &surface.body {
            compose_core::codegen::schema::Body::Fields(fields) => {
                compose_core::codegen::schema::json_field_map(fields)
            }
            compose_core::codegen::schema::Body::Type(ty) => {
                compose_core::codegen::schema::json_type_node(ty)
            }
        };
        // `format` is an annotation by default in draft 2020-12; the DSL means it
        // as a constraint (grammar 3.3 lists a closed vocabulary of them), and the
        // emitted Zod enforces it, so the JSON column is asked to as well.
        let validator = jsonschema::options()
            .should_validate_formats(true)
            .build(&schema)
            .unwrap_or_else(|error| {
                panic!(
                    "`{}` does not lower to a valid schema: {error}",
                    case.surface
                )
            });

        let mut verdicts = Vec::new();
        for document in &case.documents {
            let accepted = validator.is_valid(&document.document);
            assert_eq!(
                accepted,
                document.valid,
                "the JSON Schema lowering of `{}` {} `{}`",
                case.surface,
                if accepted { "accepts" } else { "rejects" },
                document.document
            );
            verdicts.push(accepted);
        }
        expected.push(verdicts);
    }

    let Some(root) = installed() else {
        return;
    };

    let checked = zod_column(&cases, root, "schema-lowering", runner, "Bun");
    assert_eq!(
        checked,
        expected.iter().map(Vec::len).sum::<usize>(),
        "not every document reached both columns"
    );
}

/// Answer the corpus's Zod column under one runtime, and hold every verdict to
/// what the corpus says. Answers how many documents were checked.
///
/// `spawn` is the only thing gate 3 and gate 15 differ in: [`runner`] points it
/// at Bun and [`node_command`] at the Node fallback. The staging, the grouping of
/// cases by golden, the JSON text each document is handed and the comparison are
/// all here, so the two engines cannot come to different verdicts by drifting
/// apart — the same reason gates 9 to 12 hand their assertions to both.
fn zod_column(
    cases: &[Case],
    root: &Path,
    purpose: &str,
    spawn: fn(&str) -> Command,
    runtime: &str,
) -> usize {
    // One run per golden the corpus reaches.
    let mut checked = 0usize;
    for directory in cases
        .iter()
        .map(|case| case.golden.as_str())
        .collect::<BTreeSet<_>>()
    {
        let golden = goldens::golden(directory);
        let project = staged(golden, root, purpose);
        let indices: Vec<usize> = cases
            .iter()
            .enumerate()
            .filter(|(_, case)| case.golden == directory)
            .map(|(index, _)| index)
            .collect();

        let names = compose_core::codegen::names::Names::of(&artifact(golden));
        let input: Vec<Value> = indices
            .iter()
            .map(|index| {
                let case = &cases[*index];
                serde_json::json!({
                    "export": names.value(&case.surface),
                    // JSON *text* rather than values: the runner parses each one
                    // itself, so the key order a case wrote survives to the Zod
                    // column. See `Document::text`.
                    "documents": case
                        .documents
                        .iter()
                        .map(Document::text)
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let input_path = project.join("schema-lowering-cases.json");
        fs::write(
            &input_path,
            serde_json::to_string(&input).expect("the corpus serializes"),
        )
        .expect("the scratch area is writable");

        let output = spawn("zod-conformance.mjs")
            .arg(&input_path)
            .arg(&project)
            .output()
            .expect("the runtime runs");
        assert!(
            output.status.success(),
            "the Zod corpus did not run against `{directory}` under {runtime}:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        let verdicts: Vec<Vec<bool>> = serde_json::from_slice(&output.stdout)
            .expect("the runner prints one array of verdicts per case");
        assert_eq!(verdicts.len(), indices.len());

        for (verdict, index) in verdicts.into_iter().zip(indices) {
            let case = &cases[index];
            for (accepted, document) in verdict.into_iter().zip(&case.documents) {
                assert_eq!(
                    accepted,
                    document.zod_verdict(),
                    "the emitted Zod for `{}` {} `{}` under {runtime}; the corpus says it {}{} — \
                     grammar 3.8's two columns have drifted",
                    case.surface,
                    if accepted { "accepts" } else { "rejects" },
                    document.document,
                    if document.zod_verdict() {
                        "accepts"
                    } else {
                        "rejects"
                    },
                    document.divergence.as_ref().map_or_else(
                        || ", as the JSON Schema lowering does".to_string(),
                        |divergence| format!(", diverging from the JSON column as `{divergence}`"),
                    )
                );
                checked += 1;
            }
        }
    }
    checked
}

/// One surface whose emitted Zod says more than the schema a provider would be
/// handed for it.
struct Weakening {
    /// The surface, by canonical path — all of them from `every-schema-form`,
    /// which is the golden that reaches every spelling.
    surface: &'static str,
    /// The JSON Schema keyword this compiler's own lowering writes and the
    /// conversion of the emitted Zod does not.
    keyword: &'static str,
    /// What the check is, for the failure message.
    about: &'static str,
}

/// What `withStructuredOutput` would lose, one row per spelling that loses
/// something.
///
/// Zod models a `.refine` as an opaque predicate, and a JSON Schema conversion
/// has nowhere to put it — so every check `codegen::schema` writes out rather
/// than borrowing from a constructor disappears on the way to the model. The
/// rows are the three shapes that happens in, plus one reached through a nested
/// property, because "only the top level is affected" would be a comforting and
/// wrong reading of the first three.
const WEAKENINGS: &[Weakening] = &[
    Weakening {
        surface: "state.at",
        keyword: "format",
        about: "`format: date-time`, written out as `refine(rfc3339DateTime, …)`",
    },
    Weakening {
        surface: "state.mark",
        keyword: "maxLength",
        about: "`max_length`, written out over a code-point count",
    },
    Weakening {
        surface: "state.tags",
        keyword: "uniqueItems",
        about: "`unique_items`, which Zod has no built-in for",
    },
    Weakening {
        surface: "agent.shaper.output",
        keyword: "format",
        about: "`format: email` on a nested property, two levels in",
    },
];

/// Gate 4: what PRD 5.2's delivery mechanism would hand a provider, and what it
/// drops on the way.
///
/// `withStructuredOutput` does not send Zod to anyone: `@langchain/core` converts
/// it to JSON Schema first, and that conversion keeps what Zod models as a check
/// and drops what it models as a refinement. Six of the ten formats, both length
/// bounds and `unique_items` are refinements, so the contract a model is
/// constrained by is strictly weaker than the parse its answer then faces —
/// which is the failure `codegen::schema` was written to prevent, pointed the
/// other way.
///
/// That question — a conversion of this Zod, or the lowering this compiler
/// already publishes and the corpus proves equal to the parse — is **answered**:
/// PRD 9.16 takes the lowering, and `codegen::runtime`'s agent call sends it over
/// `fetch` with no `withStructuredOutput` in the path. So this gate measures the
/// road not taken, and that is the point of keeping it: the conversion is the
/// library's own, the schemas are the committed goldens, and the day it stops
/// dropping refinements this fails — which is exactly the revisit condition 9.16
/// names.
#[test]
fn what_the_structured_output_mechanism_would_be_handed() {
    let golden = goldens::golden("every-schema-form");
    let ir = artifact(golden);
    let surfaces = compose_core::codegen::schema::surfaces(&ir);
    let names = compose_core::codegen::names::Names::of(&ir);

    // The compiler's own lowering first: it needs no toolchain, and a row whose
    // keyword this column does not even write would be checking nothing.
    for row in WEAKENINGS {
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == row.surface)
            .unwrap_or_else(|| panic!("`every-schema-form` declares no `{}`", row.surface));
        let published = match &surface.body {
            compose_core::codegen::schema::Body::Fields(fields) => {
                compose_core::codegen::schema::json_field_map(fields)
            }
            compose_core::codegen::schema::Body::Type(ty) => {
                compose_core::codegen::schema::json_type_node(ty)
            }
        };
        assert!(
            mentions(&published, row.keyword),
            "this compiler's lowering of `{}` writes no `{}`, so the row says nothing about {}",
            row.surface,
            row.keyword,
            row.about,
        );
    }

    let Some(root) = installed() else {
        return;
    };
    let project = staged(golden, root, "structured-output");
    let mut convert = runner("structured-output-schema.mjs");
    convert.arg(&project);
    for row in WEAKENINGS {
        convert.arg(names.value(row.surface));
    }
    let output = convert.output().expect("bun runs");
    assert!(
        output.status.success(),
        "the emitted schemas did not convert:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let converted: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one schema per export");

    for row in WEAKENINGS {
        let handed = &converted[names.value(row.surface)];
        assert!(
            !mentions(handed, row.keyword),
            "`{}` reaches a provider carrying `{}` after all: {} survived the conversion, so the \
             emitted Zod and the schema a model is constrained by no longer differ here. That is \
             the premise of `codegen::schema`'s *What a provider is handed* — re-read it before \
             deleting this row.",
            row.surface,
            row.keyword,
            row.about,
        );
    }
}

/// Whether a JSON Schema writes this keyword anywhere inside it.
fn mentions(schema: &Value, keyword: &str) -> bool {
    match schema {
        Value::Object(map) => {
            map.contains_key(keyword) || map.values().any(|value| mentions(value, keyword))
        }
        Value::Array(items) => items.iter().any(|item| mentions(item, keyword)),
        _ => false,
    }
}

/// The fixture, **both** its lockfiles, and the emitter pin the same versions.
///
/// Without this the gates would keep passing against whatever was installed the
/// last time someone touched the fixture, while `build` emitted a manifest for
/// something else — a green suite about the wrong toolchain. Both lockfiles are
/// read, because a pin bump that regenerated one of them and not the other would
/// leave the two supported runtimes checked against different resolutions, which
/// is the one thing having two of them must not cost.
#[test]
fn the_toolchain_fixture_pins_what_the_emitter_pins() {
    let path = toolchain::root().join("package.json");
    let text = fs::read_to_string(&path).expect("the toolchain fixture is readable");
    let manifest: Value = serde_json::from_str(&text).expect("the fixture is JSON");

    for (section, pins) in [
        ("dependencies", compose_core::codegen::project::PINS),
        ("devDependencies", compose_core::codegen::project::DEV_PINS),
    ] {
        let block = manifest[section]
            .as_object()
            .unwrap_or_else(|| panic!("the fixture declares no `{section}`"));
        assert_eq!(
            block.len(),
            pins.len(),
            "`{section}` in the fixture and in the emitter differ in size"
        );
        for (package, version) in pins {
            assert_eq!(
                block.get(*package).and_then(Value::as_str),
                Some(*version),
                "the fixture does not pin `{package}` at the version the emitter does"
            );
        }
    }

    // A lockfile is what a `--frozen-lockfile` install resolves from; one that
    // predates a pin bump would install the old version and the gates would
    // check it. `bun.lock` first, because that is the default install.
    let lock = fs::read_to_string(toolchain::root().join("bun.lock"))
        .expect("the Bun lockfile is committed");
    let declared = &lock;
    for (section, pins) in [
        ("dependencies", compose_core::codegen::project::PINS),
        ("devDependencies", compose_core::codegen::project::DEV_PINS),
    ] {
        for (package, version) in pins {
            // `bun.lock` is JSONC — trailing commas and all — so it is read as
            // text rather than parsed. The two spellings are the workspace's own
            // declaration and the resolved entry, and both have to name the pin:
            // the first is what `--frozen-lockfile` compares the manifest
            // against, and the second is what actually gets installed.
            for entry in [
                format!("\"{package}\": \"{version}\""),
                format!("\"{package}\": [\"{package}@{version}\""),
            ] {
                assert!(
                    declared.contains(&entry),
                    "the committed `bun.lock` does not carry `{entry}` for `{section}`; \
                     run `bun install` in `tests/toolchain` and commit it"
                );
            }
        }
    }

    // …and `package-lock.json`, which gate 13 installs with `npm ci`.
    let lock = fs::read_to_string(toolchain::root().join("package-lock.json"))
        .expect("the npm lockfile is committed");
    let lock: Value = serde_json::from_str(&lock).expect("the lockfile is JSON");
    let root = &lock["packages"][""];
    for (section, pins) in [
        ("dependencies", compose_core::codegen::project::PINS),
        ("devDependencies", compose_core::codegen::project::DEV_PINS),
    ] {
        for (package, version) in pins {
            assert_eq!(
                root[section][*package].as_str(),
                Some(*version),
                "the committed `package-lock.json` does not pin `{package}` at `{version}`; \
                 run `npm install --package-lock-only` in `tests/toolchain`"
            );
        }
    }
}

/// Gate 13: the Node fallback, installed with npm and run under Node.
///
/// PRD §9.18 makes Bun the default and keeps **Node >= 22.18 a supported
/// fallback**, and the emitted `README.md` prints the commands for it. Every gate
/// above runs under Bun, so without this one that whole second column would be a
/// paragraph nothing executes — and it would rot silently, because a Bun-only API
/// slipping into `codegen::runtime` breaks nothing a Bun-run suite can see.
///
/// So this is the README's fallback block, run: `npm ci` from the committed
/// `package-lock.json`, `npm run typecheck`, the graph constructed, the state
/// model **invoked** against the same expectations gate 2b holds it to, and
/// `node src/index.ts`. One golden ([`NODE_FALLBACK_GOLDEN`]) rather than the
/// corpus, because what is in question is the runtime rather than any
/// composition — gate 14 is the one that covers every emitted module, statically.
///
/// # The gates whose subject is the runtime itself
///
/// Type-checking, construction and a reduction are about the emitted code, and
/// they would answer the same under any engine that runs it. Gates 9 to 12 are
/// not like that: an EPIPE arriving as a rejected promise or as an `error` event
/// is `child_process`'s answer, a `Content-Type` composed by appending is
/// `Headers`'s, a spelled-out number in a query string is `URLSearchParams`'s,
/// and what a raw binding binds runs through all of them. Those were the gates
/// running under Node before Bun became the default, and a Bun-only suite would
/// let `node src/index.ts` break for a reader while every gate stayed green. So
/// the three runners behind them are re-run here, under Node, against the
/// assertions their own gates make — the *same functions*, so the two runs cannot
/// come to different verdicts by drifting apart.
///
/// Gate 18's runner is re-run for the sharpest form of the same reason: the
/// messages it is about are *written* by the engine, and the two engines write
/// them differently. Bun quotes the resolved string a `new URL` could not parse
/// where Node says only `Invalid URL`, and Node quotes the endpoint `fetch`
/// could not parse where Bun does not — so `docs/trace.md` §11.1's promise about
/// a public surface would be exactly half-checked without this column.
///
/// They are pointed at this gate's own staged project rather than a second copy:
/// `src/runtime.ts` is a compiler constant, byte-identical in every project this
/// release builds (see `codegen::runtime`), and `runExec`/`runHttp` are all the
/// runners import.
///
/// Gate 19's runner is re-run because a `human` pause is the runtime's most
/// timer-dependent construct and a timer is the engine's — `setTimeout`,
/// `unref`, `clearTimeout` and the arithmetic `runActivity` does around them.
/// The pause runtime has no second column anywhere else: the acceptance suite
/// serves under Bun too.
///
/// Gate 5 is deliberately **not** among them, and the reason is worth writing
/// down so the omission stays a decision. Its subject is the emitted runtime's
/// own scheduling rather than an engine API — `map-dispatch.mjs` answers
/// identically under both today — and it decides deadline questions on margins of
/// tens of milliseconds (`timeoutMs: 80` against a 120 ms activity, and more like
/// it). Running it a second time would double this suite's exposure to a loaded
/// runner's timing for a claim that is not about the runtime, which is a worse
/// trade than the gap it closes: gate 14 is what says nothing engine-specific is
/// in those paths, and this gate is what says the engine can run them.
///
/// The inherited-property probe rides along for a reason of its own: gate 8's
/// list is the JavaScript **engine's**, and `codegen::state` refuses channel names
/// from it at compile time. A list checked only against JavaScriptCore would be a
/// compiler constant that is wrong for every reader on V8, so both engines are
/// asked the same question.
#[test]
fn a_generated_project_installs_type_checks_and_runs_under_the_node_fallback() {
    let Some(root) = node_fallback() else {
        return;
    };
    let golden = goldens::golden(NODE_FALLBACK_GOLDEN);
    let project = staged(golden, root, "node-fallback");

    let tsc = root.join("node_modules/.bin/tsc");
    assert!(
        tsc.is_file(),
        "`npm ci` did not install the pinned TypeScript: {}",
        tsc.display()
    );
    let typecheck = Command::new("npm")
        .args(["run", "typecheck"])
        .current_dir(&project)
        .output()
        .expect("npm runs");
    assert!(
        typecheck.status.success(),
        "`{NODE_FALLBACK_GOLDEN}` does not type-check under the npm install:\n{}\n{}",
        String::from_utf8_lossy(&typecheck.stdout),
        String::from_utf8_lossy(&typecheck.stderr),
    );

    let construct = node_command("state-channels.mjs")
        .arg(&project)
        .output()
        .expect("node runs");
    assert!(
        construct.status.success(),
        "`{NODE_FALLBACK_GOLDEN}` does not construct its graph under Node:\n{}",
        String::from_utf8_lossy(&construct.stderr),
    );

    // The same table gate 2b holds every golden to, answered by Node instead of
    // Bun. A run rather than a load: this is where the emitted reducers, the
    // initial values and LangGraph's own scheduler all execute.
    let entry = REDUCTIONS
        .iter()
        .find(|entry| entry.golden == NODE_FALLBACK_GOLDEN)
        .expect("the fallback golden has a reduction row");
    let writes = project.join("state-writes.json");
    fs::write(&writes, entry.writes).expect("the scratch area is writable");
    let reduced = node_command("state-reduction.mjs")
        .arg(&project)
        .arg(&writes)
        .output()
        .expect("node runs");
    assert!(
        reduced.status.success(),
        "`{NODE_FALLBACK_GOLDEN}` did not run its state model under Node:\n{}",
        String::from_utf8_lossy(&reduced.stderr),
    );
    let mut answer: Value =
        serde_json::from_slice(&reduced.stdout).expect("the runner prints the state as JSON");
    answer
        .as_object_mut()
        .expect("the state is an object")
        .remove("$run")
        .expect("every state model carries the compiler's own channel");
    let expected: Value = serde_json::from_str(entry.expected).expect("the row is JSON");
    assert_eq!(
        answer, expected,
        "`{NODE_FALLBACK_GOLDEN}` reduces differently under the fallback runtime"
    );

    // `node src/index.ts`, the README's fallback launch, with the composition's
    // variables supplied.
    let references = compose_core::codegen::env::References::of(&artifact(golden));
    let names: Vec<&str> = references.names().collect();
    assert!(
        !names.is_empty(),
        "`{NODE_FALLBACK_GOLDEN}` references no variable, so loading it says nothing about \
         the presence check the README promises"
    );
    let mut launch = Command::new("node");
    launch.arg(project.join("src/index.ts"));
    for name in &names {
        launch.env(name, "supplied");
    }
    let launch = launch.output().expect("node runs");
    assert!(
        launch.status.success(),
        "`node src/index.ts` — the fallback launch the emitted README documents — failed:\n{}",
        String::from_utf8_lossy(&launch.stderr),
    );

    // Gates 9 to 12, asked of the runtime that answers them differently or not at
    // all. `http-request.mjs` covers two of them in one process — the runners are
    // split under Bun so each gate fails on its own, and there is one gate here.
    a_raw_binding_bound(&node_runner("raw-decoding.mjs", &project));
    the_unread_input_was_survived(&node_runner("unread-stdin.mjs", &project));
    let request = node_runner("http-request.mjs", &project);
    the_declared_media_type_arrived_alone(&request);
    the_bound_object_arrived_as_parameters(&request);

    // Gate 2j, and the one whose subject is most the engine's: which of the two
    // runtimes quotes a resolved string in a URL it could not parse is the
    // engine's own choice, so `docs/trace.md` §11.1's promise is only as good as
    // the runtime that held it last.
    the_resolved_values_stayed_out_of_the_messages(&node_runner(
        "resolved-env-in-failures.mjs",
        &project,
    ));

    // Gate 19, for the reason gate 5 is left out rather than against it: a
    // `human` pause is the runtime's most **timer**-dependent construct, and a
    // timer is the engine's. `runHuman` arms a `setTimeout` and probes for
    // `unref`, which the two runtimes implement separately; `runActivity` holds
    // and re-arms one, and what it re-arms it with is arithmetic over
    // `clearTimeout`'s semantics and the timer's identity. Its margins are the
    // opposite of gate 5's, too — a wait is settled by an event and the one
    // budget it measures is held across a 100 ms tick it is *not* spending — so
    // asking the fallback costs a second and no flakiness. Without this column
    // the whole of the pause runtime runs on one engine: the acceptance suite
    // serves under Bun as well.
    the_wait_board_behaved(&node_runner("human-waits.mjs", &project));

    // Gate 20, and the one whose subject is *only* the engine: the prompt loop a
    // `run` answers a pause with is a reader over `process.stdin`, and a `data`
    // event's chunking, a stream's `end` and what `setEncoding` does to a chunk
    // are Bun's and Node's separately. Without this column half the supported
    // readers would have a `run` that could hang on a question it had printed,
    // and nothing in the suite would say so.
    the_terminal_asked_and_took_every_answer(&node_runner("interactive-pause.mjs", &project));

    // Gate 21, and the reason that gate is not gate 20 with a different stream:
    // everything above drives the loop over a `PassThrough`, and the stream
    // `src/cli.ts` actually reads is the process's own. A pipe the operating
    // system owns, wired into the runtime's event loop, decides three things
    // this column cannot get from the other — that a `data` listener attached
    // at the first question still sees what arrived before it, that `end` fires
    // on a pipe whose EOF is older than that listener, and that `pause()` lets
    // go of the handle so `node src/index.ts run` **exits** rather than sitting
    // on a stream it has finished with. The Node launch above it is a load with
    // no verb and no input, so without this the fallback runtime's `run` could
    // hang on a printed question with every gate green.
    the_real_standard_input_was_read_and_released(&|which, typed| {
        over_a_real_pipe(
            node_command("interactive-stdin.mjs"),
            &project,
            which,
            typed,
        )
    });

    // Gate 16, asked of the other runtime. The local store backends are a
    // WebAssembly SQLite over `node:fs` and a directory of files, which is
    // exactly the shape of dependency that can behave differently on the
    // fallback — and every store in every emitted project runs on it.
    let data = root
        .join("projects")
        .join("node-fallback")
        .join("store-data");
    let _ = fs::remove_dir_all(&data);
    let stored = node_command("store-backends.mjs")
        .arg(&project)
        .arg(&data)
        .output()
        .expect("node runs");
    assert!(
        stored.status.success(),
        "the store-backend runner failed under Node:\n{}",
        String::from_utf8_lossy(&stored.stderr),
    );
    the_local_backends_behaved(
        &serde_json::from_slice(&stored.stdout).expect("the runner prints one JSON object"),
    );

    // Gate 8's list, asked of the other engine.
    let refused = compose_core::codegen::state::INHERITED_PROPERTY_NAMES;
    let probed: Vec<&str> = refused
        .iter()
        .copied()
        .chain(["draft", "constructors"])
        .collect();
    let probe = node_command("inherited-channel-names.mjs")
        .arg(serde_json::to_string(&probed).expect("the names serialize"))
        .output()
        .expect("node runs");
    assert!(
        probe.status.success(),
        "the inherited-name runner failed under Node:\n{}",
        String::from_utf8_lossy(&probe.stderr),
    );
    let probe: Value =
        serde_json::from_slice(&probe.stdout).expect("the runner prints one JSON object");
    assert_eq!(
        probe["inherited"],
        serde_json::to_value(refused).expect("the names serialize"),
        "`INHERITED_PROPERTY_NAMES` is not `Object.getOwnPropertyNames(Object.prototype)` under \
         Node; the compiler refuses a set of channel names the fallback runtime does not"
    );
    assert_eq!(
        probe["rejected"],
        serde_json::to_value(refused).expect("the names serialize"),
        "the names `build` refuses are not the names LangGraph refuses under Node"
    );
}

/// A `node` command that runs one of the fixture's runners, from the copy staged
/// inside the npm install.
///
/// The Node counterpart of [`runner`], and the reason it takes the same argument
/// in the same order: a gate that re-runs a corpus under the fallback engine
/// differs from the Bun one in this function and nowhere else.
///
/// The **staged** copy and not the committed one, for the reason [`node_fallback`]
/// gives: a runner carries bare imports of its own, and Node resolves those from
/// the directory the runner is in. Spawning `tests/toolchain/state-reduction.mjs`
/// would hand the Node gate a `StateGraph` out of Bun's `node_modules/` — or, on
/// a machine that has npm and no Bun, `ERR_MODULE_NOT_FOUND` for a package the
/// fixture pins and npm installed.
///
/// # Panics
///
/// Panics when the npm install is absent, so call it only after [`node_fallback`]
/// has answered `Some` — every caller already has to, since it is what stages the
/// runner this spawns.
fn node_command(script: &str) -> Command {
    let root = node_fallback().expect("the npm toolchain installed, so the runners are staged");
    let mut command = Command::new("node");
    command.arg(root.join(script));
    command
}

/// Every runner Node spawns lives inside the npm install, and its own imports
/// resolve there.
///
/// The bug this is written against is silent on the machine most likely to be
/// running it. Node resolves a module's bare specifiers from *that module's*
/// directory, so a runner spawned out of `tests/toolchain/` reads
/// `@langchain/langgraph` from the tree `bun install` writes there — and gates 13
/// and 15 stay green on a developer machine with both runtimes while the state
/// model they reduce is driven by a `StateGraph` from Bun's tree over
/// `Annotation`s from npm's, two copies of one package in one process and no
/// reading of the npm install by the runner at all. The same code on a machine
/// with npm and no Bun — precisely the reader the fallback is promised to — dies
/// with `ERR_MODULE_NOT_FOUND` for a package the fixture pins and npm installed;
/// on a cold CI checkout it is a race against whichever test calls
/// `installed()` first. None of that is visible from a gate's own assertions, so
/// the property is asserted here instead of being left to be noticed.
///
/// Three parts, because the invariant needs all three: the npm install holds a
/// byte-identical copy of every committed runner, [`node_command`] spawns *that*
/// copy, and the pinned packages those runners name really do resolve inside the
/// install — asked of `node` itself, because the resolution rule is the runtime's
/// and a suite that restated it would be checking its own restatement.
///
/// Only the **pinned** specifiers are probed. [`imports`] is deliberately a
/// text scan rather than a parser — it reads a runner's prose too, and
/// `state-reduction.mjs` explains a reducer with the words `differs from
/// "assign"` — so "everything that is not relative or `node:`" is not the set
/// that has to resolve. The set that has to is the one `package.json` pins.
#[test]
fn every_runner_node_spawns_resolves_the_npm_install() {
    let Some(root) = node_fallback() else {
        return;
    };
    let fixture = toolchain::root();
    let committed = runners(&fixture);
    assert!(!committed.is_empty(), "the fixture commits no runners");
    assert_eq!(
        runners(root),
        committed,
        "the npm install does not hold every committed runner, and one spawned from anywhere \
         else resolves its own imports in the Bun install beside it"
    );

    let mut named: BTreeSet<String> = BTreeSet::new();
    for name in &committed {
        let staged = fs::read(root.join(name)).expect("the staged runner is readable");
        assert_eq!(
            staged,
            fs::read(fixture.join(name)).expect("the committed runner is readable"),
            "`{name}` in the npm install is not the committed runner"
        );

        let spawned = node_command(name);
        let arguments: Vec<&std::ffi::OsStr> = spawned.get_args().collect();
        let expected = root.join(name);
        assert_eq!(
            arguments,
            [expected.as_os_str()],
            "`node_command` does not spawn the copy of `{name}` staged in the npm install"
        );

        let source = String::from_utf8(staged).expect("a runner is UTF-8");
        named.extend(
            imports(&source)
                .into_iter()
                .filter(|specifier| pinned(specifier)),
        );
    }
    assert!(
        named.contains("@langchain/langgraph"),
        "no runner names a pinned package, so this gate asserts nothing — the scan or the \
         fixture moved: {named:?}"
    );

    for specifier in &named {
        // `--input-type=module -e` resolves against the working directory, which
        // is where the runners now are — the same walk-up they get.
        let script = format!(
            "import {{ fileURLToPath }} from \"node:url\";\n\
             process.stdout.write(fileURLToPath(import.meta.resolve({specifier:?})));"
        );
        let resolve = Command::new("node")
            .args(["--input-type=module", "-e", script.as_str()])
            .current_dir(root)
            .output()
            .expect("node runs");
        assert!(
            resolve.status.success(),
            "`{specifier}`, which a staged runner imports, does not resolve in the npm \
             install:\n{}",
            String::from_utf8_lossy(&resolve.stderr),
        );
        let resolved = PathBuf::from(String::from_utf8_lossy(&resolve.stdout).into_owned());
        assert!(
            resolved.starts_with(root.join("node_modules")),
            "a staged runner's `{specifier}` resolves to `{}`, which is outside the npm install \
             at `{}` — under a `node_modules/` some other install wrote",
            resolved.display(),
            root.display(),
        );
    }
}

/// One of the fixture's runners, under Node, over a project staged in the npm
/// install.
///
/// Same script, same argument, same JSON object back as [`runner`] — so the
/// assertions gates 9 to 12 make can be handed either runtime's answer without
/// knowing which one produced it.
fn node_runner(script: &str, project: &Path) -> Value {
    let output = node_command(script)
        .arg(project)
        .output()
        .expect("node runs");
    assert!(
        output.status.success(),
        "`{script}` failed under the Node fallback:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object")
}

/// A module specifier no emitted file may import, or an expression no emitted
/// file may write.
struct Unportable {
    /// The text that gives it away, matched literally.
    token: &'static str,
    /// What it is, for the failure message.
    about: &'static str,
}

/// Everything a generated module is forbidden to reach for.
///
/// Every entry is Bun-only, or new enough that the Node floor
/// `compose_core::codegen::project::NODE_ENGINE` declares does not have it. The
/// `Bun.` row is matched with an identifier character after the dot on purpose:
/// a sentence in a comment that happens to end in "…under Bun." is prose, and a
/// gate that failed on it would be a gate people learn to work around.
const UNPORTABLE: &[Unportable] = &[
    Unportable {
        token: "globalThis.Bun",
        about: "the `Bun` global, which no other runtime defines",
    },
    Unportable {
        token: "process.isBun",
        about: "a Bun-detection flag, which the fallback runtime does not set",
    },
    Unportable {
        token: "import.meta.main",
        about: "`import.meta.main`, which Node did not have at 22.18",
    },
    Unportable {
        token: "\"bun:",
        about: "a `bun:` module specifier (`bun:sqlite`, `bun:ffi`, `bun:test`)",
    },
    Unportable {
        token: "'bun:",
        about: "a `bun:` module specifier (`bun:sqlite`, `bun:ffi`, `bun:test`)",
    },
];

/// Gate 14: no emitted module reaches for an API only Bun has.
///
/// Bun being the default (PRD §9.18) is a statement about how a generated project
/// is installed, launched and gated — never about what may be written into one.
/// The emitted modules stay runtime-neutral, and that is what makes
/// `node src/index.ts` a promise the README can keep.
///
/// Gate 13 runs one golden under Node, which catches a Bun-only call on a path
/// that golden takes. This is the other half, and it is the stronger one: every
/// module of every golden, read rather than run, so a Bun-only call behind a
/// `store:` branch nothing in the corpus exercises fails here too.
///
/// The import check is a **whitelist**: relative, a `node:` builtin, or a package
/// `compose_core::codegen::project::PINS` names. A blacklist would only ever
/// catch what somebody had already thought of, and the interesting failure is the
/// dependency nobody has added yet.
#[test]
fn no_emitted_module_reaches_for_an_api_the_fallback_runtime_lacks() {
    let mut modules = 0usize;
    for golden in GOLDENS {
        for file in emitted(golden).files() {
            if !file.path.starts_with("src/") || !file.path.ends_with(".ts") {
                continue;
            }
            modules += 1;
            let source = &file.contents;

            for entry in UNPORTABLE {
                assert!(
                    !source.contains(entry.token),
                    "`{}/{}` writes `{}` — {}. A generated module runs on Bun and on Node \
                     (PRD §9.18), so it may use neither runtime's private surface.",
                    golden.directory,
                    file.path,
                    entry.token,
                    entry.about,
                );
            }
            for (index, _) in source.match_indices("Bun.") {
                let next = source[index + "Bun.".len()..].chars().next();
                assert!(
                    !next.is_some_and(|character| character.is_alphabetic()
                        || character == '_'
                        || character == '$'),
                    "`{}/{}` calls the `Bun` global, which no other runtime defines. \
                     A generated module runs on Bun and on Node (PRD §9.18).",
                    golden.directory,
                    file.path,
                );
            }

            for specifier in imports(source) {
                assert!(
                    portable(&specifier),
                    "`{}/{}` imports `{specifier}`, which is neither relative, a `node:` \
                     builtin, nor one of the packages the manifest pins. A generated project \
                     installs exactly `PINS` + `DEV_PINS` under any of three installers, so an \
                     import outside that set does not resolve in a reader's directory at all.",
                    golden.directory,
                    file.path,
                );
            }
        }
    }
    assert!(modules > 0, "no emitted module was read");
}

/// Every module specifier a source file names.
///
/// All three spellings a module can reach a package by: a static `from "…"`, a
/// dynamic `import("…")`, and a bare **side-effect** `import "…";`, which names
/// no binding and so is the one form the `from` rows never see. The whitelist of
/// [`no_emitted_module_reaches_for_an_api_the_fallback_runtime_lacks`] is only
/// "over every import" if the scan is, and a side-effect import of a package the
/// manifest does not pin resolves in the toolchain's shared install — where the
/// gates run — while resolving nowhere in a reader's directory.
///
/// `import(` and `import ` cannot both match one occurrence: the character after
/// the keyword is a parenthesis in the dynamic form and a quote in the bare one.
/// Quotes are matched on both sides, so a specifier holding one is read as far as
/// its own closing quote rather than to the end of the line.
fn imports(source: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (opening, quote) in [
        ("from \"", '"'),
        ("from '", '\''),
        ("import(\"", '"'),
        ("import('", '\''),
        ("import \"", '"'),
        ("import '", '\''),
    ] {
        for (index, _) in source.match_indices(opening) {
            let rest = &source[index + opening.len()..];
            if let Some(end) = rest.find(quote) {
                found.push(rest[..end].to_string());
            }
        }
    }
    found
}

/// Whether a specifier resolves in a directory holding only the pinned install.
fn portable(specifier: &str) -> bool {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        return true;
    }
    if let Some(builtin) = specifier.strip_prefix("node:") {
        return !builtin.is_empty();
    }
    pinned(specifier)
}

/// Whether a specifier names a pinned package, or a subpath of one.
///
/// The half of [`portable`] that is about `node_modules/` rather than about the
/// runtime, split out because
/// [`every_runner_node_spawns_resolves_the_npm_install`] wants exactly it: the
/// specifiers that have to be *installed* to resolve, as opposed to the ones a
/// runtime answers on its own.
fn pinned(specifier: &str) -> bool {
    compose_core::codegen::project::PINS
        .iter()
        .chain(compose_core::codegen::project::DEV_PINS)
        .any(|(package, _)| specifier == *package || specifier.starts_with(&format!("{package}/")))
}

/// Gate 14 reads every import, in every spelling — asserted here rather than in
/// the corpus, because the corpus cannot show it.
///
/// The whitelist above is only as wide as the scan beneath it, and a spelling
/// [`imports`] does not read is a package that is never checked at all. No
/// emitted module uses a bare side-effect `import "…";` today, so the corpus
/// would pass whether or not that form is scanned: this is the test that fails
/// when it stops being.
///
/// The `@langchain/langgraph/prebuilt` row is there because the scan hands over
/// the specifier **as written** rather than the package it belongs to, which is
/// the reading [`portable`] has to admit a subpath under.
#[test]
fn the_import_scan_reads_every_spelling_a_module_can_reach_a_package_by() {
    let source = r#"
import { StateGraph } from "@langchain/langgraph";
import type { Thing } from './state.ts';
import "@langchain/langgraph/prebuilt";
import 'node:process';
const lazy = await import("node:fs/promises");
const other = await import('./cel.ts');
"#;
    let mut found = imports(source);
    found.sort_unstable();
    assert_eq!(
        found,
        [
            "./cel.ts",
            "./state.ts",
            "@langchain/langgraph",
            "@langchain/langgraph/prebuilt",
            "node:fs/promises",
            "node:process",
        ],
        "a spelling the scan misses is an import gate 14 never whitelists"
    );
    assert!(found.iter().all(|specifier| portable(specifier)));

    // The form the `from` rows cannot see, naming a package the manifest does
    // not pin: read, and refused.
    let bare = imports("import \"js-tiktoken\";\n");
    assert_eq!(bare, ["js-tiktoken"]);
    assert!(
        !portable(&bare[0]),
        "a bare import of an unpinned package resolves in the toolchain's shared \
         install and nowhere in a reader's directory"
    );

    // A specifier is read to its own closing quote rather than to the end of the
    // line, so the second one on a line is a specifier and not a tail.
    assert_eq!(
        imports("import { a } from \"./a.ts\"; import { b } from \"./b.ts\";"),
        ["./a.ts", "./b.ts"]
    );
}

/// Gate 15: the two shared corpora, answered by the other engine.
///
/// CLAUDE.md's validation strategy keeps two pairs of implementations in lockstep
/// with a shared corpus each — grammar 3.8's schema table (gate 6) and the two
/// CEL interpreters. The *second* column of both corpora is JavaScript, and what
/// it answers with belongs to the **engine**: an emitted `format:` is a `RegExp`,
/// an emitted `max_length` is a code-point count, and `src/cel.ts` is `BigInt`
/// arithmetic, `RegExp` matching and number formatting the whole way down.
/// JavaScriptCore and V8 are two engines, so a corpus answered under one of them
/// says nothing about a reader on the other. It is the argument gate 13 makes for
/// gates 9 to 12, one layer up: there the question is what `child_process` and
/// `Headers` do, here it is what a regex and a number do.
///
/// So the Zod column of the schema corpus and the whole CEL corpus are answered
/// under Node as well, against the assertions their Bun runs make — gate 6's own
/// [`zod_column`], and the same empty divergence list the acceptance suite's
/// `the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus`
/// demands of the Bun column. Without this, a JavaScriptCore-only reading of a
/// format regex would make a compiled router or a parse accept-or-reject
/// differently for every reader on the documented fallback while CI stayed green:
/// gate 13 type-checks, constructs, reduces and launches a golden under Node, but
/// never validates a document or evaluates a guard.
///
/// One golden carries the CEL corpus, for the reason gate 13 runs one — the
/// evaluator is a compiler constant — and
/// [`the_cel_evaluator_is_one_module_every_golden_carries`] is what says so rather
/// than this comment. The schema corpus names its own goldens and reaches all of
/// them.
///
/// # What is deliberately not re-run here
///
/// The **property harness** (`tests/property_conformance.rs`) answers generated
/// documents and guards with these same two emitted modules, and it stays a
/// single-engine run. Its experiment is the Rust column against the JS one over
/// shapes nobody wrote, not one engine against another; a second runtime would
/// double a twelve-seed build-and-run and need a second dependency install in a
/// binary cargo already runs in parallel with this one — for an axis the two
/// corpora now cover on both engines, over every spelling grammar 3.8 and grammar
/// 4.1 have. The trade is the same shape as gate 5's omission from gate 13, and
/// this is the paragraph to revisit if it changes.
///
/// **Gate 7's converter probe** is not here either, and for a different reason:
/// its subject is `@langchain/core`'s own Zod-to-JSON-Schema conversion, which is
/// library code running the same way under either engine. Nothing it asserts —
/// that a `.refine` has nowhere to go, so `format`, `maxLength` and `uniqueItems`
/// do not survive — turns on a regex or on a number. It is in the list of things
/// answered once on purpose, not by omission.
#[test]
fn the_shared_corpora_answer_the_same_under_the_node_fallback() {
    let Some(root) = node_fallback() else {
        return;
    };

    // Grammar 3.8's Zod column, over every golden the corpus names.
    let cases = corpus();
    assert!(!cases.is_empty(), "the corpus is empty");
    let checked = zod_column(
        &cases,
        root,
        "schema-lowering-fallback",
        node_command,
        "the Node fallback",
    );
    assert_eq!(
        checked,
        cases.iter().map(|case| case.documents.len()).sum::<usize>(),
        "not every document reached the fallback engine"
    );

    // The CEL corpus, over the evaluator a generated project embeds. The driver
    // is the acceptance suite's, run here unchanged: two runners would be two
    // readings of the corpus, which is the drift this is written against.
    let project = staged(goldens::golden(NODE_FALLBACK_GOLDEN), root, "cel-fallback");
    let corpus_directory =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cel-conformance");
    let output = node_command("cel-conformance.mjs")
        .arg(&project)
        .arg(&corpus_directory)
        .output()
        .expect("node runs");
    assert!(
        output.status.success(),
        "the CEL corpus did not run under the Node fallback:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[]",
        "the emitted CEL evaluator answers the shared corpus differently under the Node \
         fallback than the validator does; two interpreters must not diverge (CLAUDE.md), and \
         the Bun column of this same corpus is the acceptance suite's \
         `the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus`"
    );
}

/// `src/cel.ts` is one module every golden carries, which is what lets gate 15
/// answer the CEL corpus with a single golden.
///
/// Only the provenance line differs, and only in the target it names — the
/// evaluator itself is emitted byte-identically, the way `src/runtime.ts` is. The
/// day that stops being true, running the corpus against one golden stops being a
/// statement about the others, and this fails before the gate can quietly narrow.
#[test]
fn the_cel_evaluator_is_one_module_every_golden_carries() {
    let mut evaluators: BTreeSet<String> = BTreeSet::new();
    for golden in GOLDENS {
        let project = emitted(golden);
        let module = project
            .file("src/cel.ts")
            .unwrap_or_else(|| panic!("`{}` emits no `src/cel.ts`", golden.directory));
        let (provenance, body) = module
            .contents
            .split_once('\n')
            .expect("every emitted file carries a provenance line");
        assert!(
            provenance.contains(golden.target),
            "`{}/src/cel.ts` does not name its own target: {provenance}",
            golden.directory
        );
        evaluators.insert(body.to_string());
    }
    assert_eq!(
        evaluators.len(),
        1,
        "the emitted CEL evaluator differs between goldens, so gate 15's one-golden run says \
         nothing about the others"
    );
}

/// The staged copy is the golden, byte for byte.
///
/// The gates check what they copied, so a copy that lost or rewrote a file would
/// be checking something the compiler never emitted.
#[test]
fn the_staged_copy_is_the_golden_it_came_from() {
    let Some(root) = installed() else {
        return;
    };
    for golden in GOLDENS {
        let project = staged(golden, root, "staging");
        let emitted = emitted(golden);
        assert_eq!(
            files_under(&project),
            emitted.paths().map(str::to_string).collect::<Vec<_>>()
        );
        for file in emitted.files() {
            let staged = fs::read_to_string(
                project.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            )
            .expect("a staged file is readable");
            assert_eq!(staged, file.contents, "`{}` was not copied", file.path);
        }
    }
}
