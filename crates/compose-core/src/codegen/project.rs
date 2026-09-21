//! The project skeleton: `package.json`, `tsconfig.json`, `README.md`,
//! `.gitignore`, and `src/index.ts`.
//!
//! # The pinned dependency set
//!
//! PRD 5.12: "each compiler release targets a pinned LangGraph (JS) version;
//! upgrades are explicit and versioned, like Terraform providers. DSL semantics
//! must never drift silently with upstream API churn." [`PINS`] is that pin, and
//! it is **exact** — no `^`, no `~` — for every dependency, not only LangGraph:
//! a caret on `zod` would let a patch release change what a structured-output
//! schema accepts, which is the same drift by a different door.
//!
//! An upgrade is a compiler change: bump the constants, regenerate the golden
//! corpus, and let the type gate and the acceptance suite say whether the new
//! version still honours the semantics (PRD §8's primitive-parity audit). That
//! is why the goldens carry the compiler version in their headers — a release
//! that repins *should* produce a reviewable diff over every generated file.
//!
//! # Bun by default, Node as the fallback
//!
//! PRD §9.18: **Bun is the default runtime and package manager** everywhere an
//! emitted project is installed, launched, or gated — `bun install`, `bun run
//! typecheck`, `bun src/index.ts` — and that is what the emitted `README.md`
//! documents first. `agent-compose run` and `agent-compose serve` launch an
//! emitted project the same way, so the command a reader is shown here is the
//! command those verbs run.
//!
//! **Node >= 22.18 stays a supported fallback**, and the manifest is where that
//! promise is written down: [`NODE_ENGINE`] keeps the floor in `engines`, and
//! `@types/node` is pinned to the same major (see [`DEV_PINS`]). The promise is
//! only worth the line if the emitted modules can honour it, so **no generated
//! file may reach for a Bun-only API** — no `Bun` global, no `bun:` specifier —
//! and `tests/generated_code_gates.rs` holds both ends of that: gate 13 installs
//! a golden with npm and type-checks, constructs and runs it under Node, and
//! gate 14 reads every emitted module and refuses an import that is not
//! relative, a `node:` builtin, or one of [`PINS`].
//!
//! # Package-manager neutrality
//!
//! Defaulting to Bun is a statement about the documented commands, not about the
//! manifest, which stays plain: no `packageManager` field, no lockfile, no
//! install-time scripts, no workspace protocol, and no dependency that needs a
//! native build. `bun install`, `npm install`, and `pnpm install` all resolve it
//! to the same versions, because every version is exact.
//!
//! `packageManager` is the field that would name Bun, and it is deliberately not
//! emitted. It is corepack's, corepack manages npm/pnpm/yarn and not Bun, so the
//! field would be an instruction to a tool that cannot honour it — while pinning
//! every generated project to one Bun release that nothing here has tested and
//! that a reader could not change without the compiler rewriting it. Nothing is
//! bought: what makes an install deterministic is already in the manifest, which
//! is exact versions and no install-time behaviour. The neutrality claim is about
//! what the manifest *contains*, and is checked by
//! `the_manifest_stays_package_manager_neutral` in this module's own `tests`.
//!
//! A lockfile is not emitted either, under Bun as under npm: `src/` is the
//! compiler's and the rest of the directory is the reader's (see the emitted
//! `.gitignore`), and a `bun.lock` the compiler kept rewriting would be a
//! resolution the reader could never pin. `tests/toolchain/bun.lock` is the
//! *gates'* lockfile, a different artifact for a different reason: the gates have
//! to check one fixed resolution.
//!
//! # `src/index.ts`
//!
//! A barrel over the modules, which is what makes the generated project usable
//! as a library — the eject path (PRD 5.12) and, later, what `run` and `serve`
//! import. When those verbs land they launch this module the way the README's
//! first block does, `bun src/index.ts`, and fall back to `node src/index.ts`
//! where Bun is absent; neither verb may add a launch surface the README does not
//! already document, because an emitted project has to be runnable by hand.
//!
//! It is also the one emitted module with a side effect: it calls
//! [`super::env`]'s `readEnvironment()` at module scope, which is where PRD
//! 5.9's "resolution happens at process start in generated code" happens.
//! Loading the project is the check.

use std::fmt::Write as _;

use crate::ir::Ir;

use super::names;

/// The JavaScript dependency set every generated project pins, exactly.
///
/// Each version was the current stable release when this compiler release was
/// cut, and each is pinned exactly rather than by range — see the module docs
/// for why, and for what an upgrade involves.
pub const PINS: &[(&str, &str)] = &[
    // The execution substrate (PRD §4: LangGraph owns execution). The v1 line is
    // the one that ships `Annotation`, `Send`, `interrupt`, and `RemoteGraph` —
    // the four primitives PRD §8's parity audit names.
    ("@langchain/langgraph", "1.4.10"),
    // LangGraph's own peer dependency: messages, runnables, and the chat-model
    // interface a node function calls. Pinned here rather than left to the
    // installer so two projects built by one compiler release cannot resolve
    // different ones.
    ("@langchain/core", "1.2.8"),
    // The validation type of PRD 5.2 and grammar 3.8. LangGraph 1.4 accepts
    // `^3.25.32 || ^4.2.0`; the 4 line is the one whose format constructors
    // (`z.email()`, `z.iso.datetime()`) this compiler emits.
    ("zod", "4.4.3"),
    // The HTTP framework PRD 5.11 names for the `http` trigger surface ("a
    // generated Fastify app wrapping the compiled graph"). Pinned exactly for
    // the same reason LangGraph is: what a framework answers for a body it
    // cannot decode is behaviour grammar 13.3 and Decision D117 state, so a
    // release that changed it would change what a compiled graph does.
    ("fastify", "5.12.0"),
    // SQLite, for the `kv` and `vector` backends of PRD 5.8's zero-infra
    // guarantee. A WebAssembly build with no dependencies, no native step and no
    // install script — which is what makes it the *portable* SQLite: `bun:sqlite`
    // is a Bun-only specifier PRD §9.18 forbids in generated code, and
    // `node:sqlite` is a Node builtin Bun does not implement, so neither can be
    // the one driver an emitted project uses on both runtimes. See
    // `super::stores` for the trade that buys.
    ("node-sqlite3-wasm", "0.8.60"),
];

/// The development dependencies: the type gate and the runtime's own types.
///
/// `typescript` is a checker here, never a compiler — the emitted project has no
/// build step (see [`super`]). `@types/node` is pinned to the **22** line, which
/// is the oldest runtime [`NODE_ENGINE`] admits: types from a newer major would
/// describe APIs the minimum supported Node does not have. It is the right
/// declaration under Bun too, and for the same reason: Bun implements the
/// `node:` builtins the emitted modules import, and typing them against a newer
/// Node would let a module compile against an API the fallback runtime lacks.
pub const DEV_PINS: &[(&str, &str)] = &[("@types/node", "22.20.1"), ("typescript", "7.0.2")];

/// The Node versions a generated project runs on, which is the **fallback**
/// floor rather than the default runtime — that is Bun (PRD §9.18).
///
/// 22.18 is where type stripping stopped being flagged, which is what lets
/// `node src/index.ts` run a TypeScript file with no build step. Bun needs no
/// floor declared beside it: it runs TypeScript at every release this compiler
/// has been built against, and `engines` is advice `bun install` does not
/// enforce, so a range here would constrain the fallback and nothing else.
pub const NODE_ENGINE: &str = ">=22.18.0";

/// The package name every generated project takes.
///
/// It is fixed rather than derived: the IR names the entrypoint relative to the
/// project root (`main.yml`), never the root directory itself, so there is
/// nothing composition-specific to derive from — and inventing a name from the
/// output path would make the manifest depend on where it was written, which
/// PRD 5.12's determinism rule forbids.
pub const PACKAGE_NAME: &str = "agent-compose-generated";

/// Every package the generated `package.json` declares under `dependencies`:
/// [`PINS`], plus what the composition's `module:` bindings pin (PRD resolved
/// q49, D133).
///
/// One entry per package: [`PINS`] first, in the order this module declares
/// them — which is the order the emitted `README.md` prints and the order they
/// are argued in above — and then the module bindings' own, **sorted by name**.
///
/// Two orders rather than one because they answer to different things. The
/// runtime's own set is a curated list a reader of this module walks top to
/// bottom, and reordering it would put the substrate under the store driver for
/// no reason a diff could explain. The declared set is the composition's, and it
/// has to be a function of *what* was declared rather than of which tool
/// declared it first — a package that moved between two `module:` bindings would
/// otherwise rewrite `package.json` without changing what it means.
///
/// The declared set cannot disagree with itself or with the generated project's
/// own — `parse::binding` refuses a pin that contradicts one of these, and
/// `check::modules` refuses two tools pinning one package at two versions — so
/// this is a merge with nothing to reconcile, and the assertion says so rather
/// than choosing.
///
/// **Both blocks are the project's own.** A package already held is dropped from
/// the declared set whether it is in [`PINS`] or in [`DEV_PINS`], because the
/// two checks have to answer one question: `parse::binding` accepts a pin that
/// agrees with either list, so looking only at [`PINS`] here would emit a
/// `typescript: "7.0.2"` under `dependencies` *and* under `devDependencies` —
/// one package in two blocks of one manifest, which the neutrality claim above
/// ([`super`]'s *Package-manager neutrality*) does not survive: npm warns and
/// drops one, and which one it drops is not this compiler's decision to leave
/// open.
///
/// # Panics
///
/// Panics on a package pinned at two versions, which the validator refuses with
/// a span to point at. Reaching here with one is a front-end bug, and a silent
/// last-one-wins would emit a manifest whose resolution is not the one either
/// tool asked for.
#[must_use]
pub fn dependencies(ir: &Ir) -> Vec<(String, String)> {
    // The harness SDKs, pinned per harness some `coder:` node binds (PRD
    // resolved q57, and see [`super::harness::HARNESS_PINS`]). Between [`PINS`]
    // and the declared set, and not inside either: they are the runtime's own
    // rather than a composition's declaration, but a composition that binds no
    // harness declares none of them — which is what makes "emit only the drivers
    // a composition uses" true of the manifest as well as of the module.
    let mut harness: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for bound in super::harness::bound(ir) {
        for (package, version) in super::harness::pins_of(bound) {
            let held = harness.insert((*package).to_string(), (*version).to_string());
            assert!(
                held.is_none_or(|held| held == *version),
                "two harnesses pin `{package}` at two versions"
            );
        }
    }
    // …and the journal driver, pinned where this target's `journal:` binds a
    // provider that dials out (grammar §14.7, PRD resolved q62). One provider
    // per target, so there is nothing to reconcile: the zero-infra build binds
    // `sqlite`, whose driver `super::stores` already pins for every project, and
    // adds nothing here at all.
    for (package, version) in super::journal::pins_of(crate::ir::deploy::journal_of(ir)) {
        harness.insert((*package).to_string(), (*version).to_string());
    }
    let mut declared: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    for (address, module) in crate::check::modules::bindings(ir) {
        for dependency in &module.dependencies {
            let package = dependency.package.value.as_str();
            let version = dependency.version.value.as_str();
            let held: Option<String> = PINS
                .iter()
                .chain(DEV_PINS)
                .find(|(pinned, _)| *pinned == package)
                .map(|(_, version)| (*version).to_string())
                .or_else(|| harness.get(package).cloned())
                .or_else(|| declared.get(package).cloned());
            if let Some(held) = held {
                assert!(
                    held == version,
                    "`{address}` pins `{package}` to `{version}`, which this project already \
                     holds at `{held}` — the validator refuses that (grammar 6.1)"
                );
                continue;
            }
            declared.insert(package.to_string(), version.to_string());
        }
    }
    PINS.iter()
        .map(|(package, version)| ((*package).to_string(), (*version).to_string()))
        .chain(harness)
        .chain(declared)
        .collect()
}

/// `package.json`.
#[must_use]
pub fn package_json(ir: &Ir) -> super::GeneratedFile {
    let mut contents = String::from("{\n  \"//\": [\n");
    let header = super::header_lines(ir);
    for (index, line) in header.iter().enumerate() {
        let comma = if index + 1 == header.len() { "" } else { "," };
        contents.push_str(&format!("    {}{comma}\n", names::string(line)));
    }
    contents.push_str("  ],\n");
    contents.push_str(&format!("  \"name\": {},\n", names::string(PACKAGE_NAME)));
    contents.push_str("  \"version\": \"0.0.0\",\n");
    contents.push_str("  \"private\": true,\n");
    contents.push_str("  \"type\": \"module\",\n");
    contents.push_str(&format!(
        "  \"engines\": {{\n    \"node\": {}\n  }},\n",
        names::string(NODE_ENGINE)
    ));
    contents.push_str("  \"scripts\": {\n    \"typecheck\": \"tsc --noEmit\"\n  },\n");
    contents.push_str(&dependency_block("dependencies", &dependencies(ir), true));
    contents.push_str(&dependency_block(
        "devDependencies",
        &development_dependencies(ir),
        false,
    ));
    contents.push_str("}\n");

    super::GeneratedFile {
        path: "package.json".to_string(),
        contents,
    }
}

fn dependency_block(key: &str, pins: &[(String, String)], trailing_comma: bool) -> String {
    let mut text = format!("  \"{key}\": {{\n");
    for (index, (package, version)) in pins.iter().enumerate() {
        let comma = if index + 1 == pins.len() { "" } else { "," };
        text.push_str(&format!(
            "    {}: {}{comma}\n",
            names::string(package),
            names::string(version)
        ));
    }
    text.push_str(if trailing_comma { "  },\n" } else { "  }\n" });
    text
}

/// Every package the generated `package.json` declares under
/// `devDependencies`: [`DEV_PINS`], plus what this target's journal driver needs
/// to type-check.
///
/// One entry has ever arrived from the second half and it is `@types/pg`: `pg`
/// publishes no type declarations of its own, and an emitted module importing it
/// under `strict` fails `tsc` on the import rather than on anything this
/// compiler wrote. `mysql2` ships its own, so a MySQL journal adds nothing here.
///
/// It is a **development** dependency for the reason `typescript` is: nothing at
/// run time reads a `.d.ts`, and a reader who installs with `--production` is
/// installing a project that runs.
#[must_use]
pub fn development_dependencies(ir: &Ir) -> Vec<(String, String)> {
    DEV_PINS
        .iter()
        .chain(super::journal::development_pins_of(
            crate::ir::deploy::journal_of(ir),
        ))
        .map(|(package, version)| ((*package).to_string(), (*version).to_string()))
        .collect()
}

/// `tsconfig.json`.
///
/// Written as JSONC, which is what `tsconfig.json` is: the header and the
/// rationale for the sharper options live in comments the compiler reads past.
#[must_use]
pub fn tsconfig_json(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(TSCONFIG);
    super::GeneratedFile {
        path: "tsconfig.json".to_string(),
        contents,
    }
}

const TSCONFIG: &str = r#"//
// `tsc --noEmit` is a gate, not a build: Bun runs the TypeScript in `src/`
// directly, and so does Node (see README.md), so nothing here emits.
{
  "compilerOptions": {
    "target": "ES2023",
    "lib": ["ES2023"],
    "module": "nodenext",
    "moduleResolution": "nodenext",
    "types": ["node"],

    // PRD 5.12 asks for a strict target. `strict` is the whole family;
    // the three below it are the module-hygiene options that keep emitted
    // code honest without constraining how ejected code may be written.
    "strict": true,
    "noImplicitOverride": true,
    "isolatedModules": true,
    "verbatimModuleSyntax": true,

    // Relative imports name `.ts` files, which is what both supported runtimes
    // resolve: Bun takes the extension as written, and it is what Node resolves
    // when it strips types. `rewriteRelativeImportExtensions` is what keeps
    // them buildable by anyone who later chooses to emit JavaScript.
    "allowImportingTsExtensions": true,
    "rewriteRelativeImportExtensions": true,

    "skipLibCheck": true,
    "noEmit": true
  },
  "include": ["src/**/*.ts"]
}
"#;

/// `README.md`.
///
/// Takes the environment partition because one of its sections turns on it: the
/// paragraph about a pause a **worker** opens is emitted exactly when a
/// placement's process can reach a `human:` node (`docs/distributed.md` §9.1,
/// and see [`human_waits`]). Handed in rather than recomputed, so the README and
/// the manifests describe one deployment.
#[must_use]
pub fn readme(ir: &Ir, partition: &super::env::Partition) -> super::GeneratedFile {
    let mut contents = super::header(ir, "> ");
    contents.push('\n');
    contents.push_str(&format!(
        "# `{PACKAGE_NAME}`\n\n\
         The LangGraph TypeScript project `agent-compose build` produced from `{}`, \
         resolved for the `{}` target.\n",
        ir.entrypoint, ir.target
    ));
    contents.push_str(README_LAYOUT);
    contents.push_str(&installer_layout_rows(ir));
    contents.push_str(README_BODY);
    contents.push_str(&private_registry(ir));
    contents.push_str(&journal_home(ir));
    contents.push_str(&human_waits(ir, partition));
    contents.push_str(&store_data(ir));
    contents.push_str(&builtin_workspaces(ir));
    contents.push_str(&fresh_workspaces(ir));
    contents.push_str(&route_timeouts(ir));
    contents.push_str(&host_functions(ir));
    contents.push_str(README_PINS);

    let mut pins = String::from("\n| package | version |\n|---|---|\n");
    for (package, version) in PINS.iter().chain(DEV_PINS) {
        pins.push_str(&format!("| `{package}` | `{version}` |\n"));
    }
    contents.push_str(&pins);
    contents.push_str(README_TAIL);

    super::GeneratedFile {
        path: "README.md".to_string(),
        contents,
    }
}

/// The Layout table's opening rows, up to where the **target**-dependent ones
/// go (see [`installer_layout_rows`]).
const README_LAYOUT: &str = r#"
## Layout

| path | what it holds |
|---|---|
| `.gitignore` | the three things a checkout of this directory leaves out: the install artifact, the `.env` the spec deliberately never contains, and the data this project's own stores keep |
| `README.md` | this file |
| `package.json` | the dependency set, each package pinned to the version this compiler release was built against — plus whatever a `module:` binding declared under `dependencies:` — the `typecheck` script, and the Node floor |
"#;

/// The two rows a target that declares a `package_registry:` adds, and nothing
/// where it does not (grammar §14.6).
///
/// The table is **normative** in the emitted project — the paragraph under it
/// says the file list is the boundary — so a row for a file this build did not
/// write would tell a reader the compiler owns a name it never touches, and a
/// missing row would tell the owner of a `bunfig.toml` that it is theirs right
/// up until the next build replaces it.
fn installer_layout_rows(ir: &Ir) -> String {
    if ir.deploy.package_registry.is_none() {
        return String::new();
    }
    "| `bunfig.toml` | where `bun install` resolves this project's packages from, written from the target's `package_registry:`. A credential in it is the *name* of an environment variable, expanded at install time |\n\
     | `.npmrc` | the same configuration in npm's own spelling, so `npm install` and `pnpm install` reach the same registry |\n"
        .to_string()
}

const README_BODY: &str = r#"| `tsconfig.json` | the type checker's settings: strict, `noEmit`, and the `.ts` import extensions both supported runtimes resolve |
| `manifest.json` | what a **worker** reads out of this tree before it can run anything: the node runner's path, which files here the compiler did not write, and each placement's environment as `docs/distributed.md` §9.1 partitions it. The same partition `src/deployment.ts` carries, in the format the `agent-compose worker` binary can read without a JavaScript runtime |
| `src/artifact.ts` | what this tree **is**: a content hash over its own files — the emitted ones and the authored ones the composition references — the file list a worker fetch is served from, and the compiler release that wrote it (`docs/distributed.md` §4) |
| `src/cel.ts` | the CEL evaluator the routers embed (PRD 5.5) |
| `src/delivery.ts` | what a settled execution owes the outside world: the bounded, journaled schedule a lifecycle `callback:` and a `trace_sink:` export are both worked on, and the one place this project POSTs one (`docs/durability.md` §3.7) |
| `src/deployment.ts` | what the deploy layer declares: the placements a worker may claim, the per-process environment partition the hub and a worker both read out of it (`docs/distributed.md` §9.1), and the address every settled trace ships to (grammar §14.5) |
| `src/env.ts` | every `${ENV}` reference **this process** needs — the hub's own list, which is the whole composition's unless a placement takes something off it — and `readEnvironment()`, the presence check over them |
| `src/journal.ts` | the execution journal: every effect a run issues, written as it happens, and what a resumed execution consumes instead of re-issuing it (`docs/durability.md`) |
| `src/mesh.ts` | the hub half of the worker protocol: the `/workers/*` routes a serve mounts where the target declares `placements:`, the journaled dispatch board behind them, and the seam a placed node reaches a worker through (`docs/distributed.md`) |
| `src/modules.ts` | the generated half of every `module:` binding: one contract type per module-bound tool, written from that tool's own `input:`/`output:`, the type of the `env:` that binding declared, and the typed `const` holding the authored implementation. The **only** generated module that imports code you wrote |
| `src/otlp.ts` | the OTLP/JSON span exporter: one settled trace envelope mapped to an `ExportTraceServiceRequest`, hand-written and with no OpenTelemetry dependency behind it (`docs/trace.md` §12) |
| `src/runtime.ts` | what every node does when it runs: the retry/timeout/error policy of grammar 9, the provider surfaces, the model failover ladder, the `exec`/`http` wrappers, and the router |
| `src/stores.ts` | the local store backends: SQLite for `kv` and `vector`, a directory of files for `blob` (PRD 5.8) |
| `src/schemas.ts` | every schema the composition declares, as Zod |
| `src/state.ts` | the graph's state model: one channel per `state:` channel, the implicit conversation history, and `$run` — what the runtime keeps beside them |
| `src/graph.ts` | the compiled graph: one node per flow node, the `flows` registry, and `runFlow` |
| `src/harness.ts` | the coding-harness drivers a `coder:` node reaches its SDK through, one per harness this composition binds, plus a scripted driver for tests. The one generated module whose *imports* depend on the composition (grammar 8.9) |
| `src/triggers.ts` | the composition's declared `http` triggers: their routes, their response modes, and the CEL that reads a request payload |
| `src/serve.ts` | the app over those triggers: start, status and resume (PRD 5.11) |
| `src/cli.ts` | this project's own command line, which `agent-compose run`, `agent-compose resume` and `agent-compose serve` launch |
| `src/index.ts` | the project's public surface, the one caller of `readEnvironment()`, and the entry point the command line hangs off |
| `src/worker-node.ts` | the worker half of the worker protocol: one placed node, executed out of this tree by `agent-compose worker`, with the dispatch on standard input and its effects and result as NDJSON on standard output (`docs/distributed.md` §3.2) |

## Running a flow

```ts
import { runFlow } from "./src/index.ts";

const run = await runFlow("flow.<name>", { /* the flow's declared inputs */ });
console.log(run.outputs); // its `outputs:`, materialized at quiescence
console.log(run.trace);   // every routing decision the run made, as data
```

Every flow is runnable whether or not a `manual` trigger names it (PRD 5.11),
so `flows` holds them all. The inputs are parsed against the flow's own
`inputs:` schema before anything runs, and the trace is the routing record
PRD 5.3 asks for: one entry per node execution, carrying the guards that were
evaluated, what they answered, which edges were taken, and the state of any
`max_iterations` budget they spent.

The trace is a **versioned, documented format**, so a reader may be written
against it rather than against whatever this release happened to record. Every
machine surface that carries one carries its version beside it — the
`trace_version` key of `run --format json`, of the trace file's envelope, and of
the status route's report — and `runtime.TRACE_VERSION` is what a compiled
project spells it with. What each field means, and what a bump to that number
does and does not signal, is the compiler's `docs/trace.md`.

A run that produces no answer throws a `FlowFailure`, and it carries that same
record: `.trace` holds every step that completed, plus one final entry for the
node the run stopped at where there was one, and `.cause` is the error itself.
Both ways a run can fail raise it — one that never reached quiescence, and one
that reached quiescence holding no value for a field its `outputs:` declares —
so `.trace` is readable without asking which happened. The failure with no final
entry is the `SuperstepCeiling` below: the ceiling stops a run *between*
supersteps, so no node aborted it and the error itself is the account.

`recursionLimit` is the one option that is not about identity: it raises the
superstep ceiling for a single run. The ceiling is a safety net rather than one
of the composition's own bounds, sized from the `max_iterations` budgets a flow
declares plus an allowance for every cycle bounded only by a CEL exit condition,
and a run that reaches it fails with a `SuperstepCeiling` saying so.

`src/index.ts` calls `readEnvironment()` at module scope, so loading this project
is what checks its environment: a missing variable throws before anything runs,
naming every variable that is missing rather than the first (PRD 5.9, grammar
4.3). `agent-compose build` itself reads no environment — no value is resolved at
compile time, which is what keeps this directory committable and free of
credentials.

**The file list above is the boundary.** `agent-compose build` replaces exactly
the files in that table — every one of them, `package.json` and this README
included — plus a copy of each implementation the composition references, which
is the section below. Nothing else in this directory is written, removed, or
reported. A `node_modules/`, a lockfile, a `.env` — and any TypeScript you wrote
that the composition does not reference — are yours, wherever they sit. `src/` is
not a compiler-only directory, and the root is not a yours-only one: what makes a
file the compiler's is being on that list.

Every file the compiler **generates** carries the header above, which is how it
tells its own work from yours: a `build` into a directory holding none of its
files refuses rather than overwriting what is there, naming the ones it would
have replaced. A build writes one other kind of file here and it carries no
header — a copy of an implementation you wrote, which the section below is
about — so the two sentences are one rule: what a build writes is the table
above plus the authored files the composition references, and it is the only
thing it writes.

`src/tools/` **in the project** — beside `main.yml`, where a `module:` path is
resolved — is where your own code goes. A `tool.*` in the composition may bind
`module: ./src/tools/<name>.ts`, and `build` writes that file **once**, there —
the typed signature, the contract as a doc comment, a body that throws — then
never writes it again and never reads it to re-emit it. Fill it in and commit it
*there* as well: what a build leaves at that path in an output directory —
including this one — is a copy (the section below), replaced with whatever the
project holds on every build, so the project's file is the one an edit survives
in. The tool's declared `input:` and `output:` are its contract,
`src/modules.ts` is where that contract is written down, and `bun run typecheck`
is what holds the file to it. The directory is a convention rather than a rule;
the file list is the rule.

What such a file reads out of the environment is its binding's `env:`, and it
arrives as the **third argument** rather than through `process.env`: the values
belong to the call, so nothing they hold reaches a later `exec:` child or a
module running beside it, and `src/modules.ts` types the argument from the names
the composition declared — a variable the YAML does not list does not compile.

The **second** is the invocation context, and it carries what a host function's
does. `context.signal` aborts when the node's `timeout:` budget runs out — the
runtime races that deadline whether or not the implementation looks, so one that
ignores it keeps running after the node it belonged to has failed, and whatever
it returns is discarded. `context.idempotency_key` is set on exactly one call: a
**detached** `map` dispatch that reached this tool as its sink, which is
delivered at-least-once (grammar 9.4) and is the one delivery a sink has to
dedupe. It is absent everywhere else, where a repeated call is what the
composition asked for — and that same call is the one whose `signal` is not the
map node's, because a detached delivery is off that node's clock.

A build copies each implementation the composition references into this
directory at the same relative path, because the artifact a worker fetches has
to carry it: `src/artifact.ts` lists it and hashes it like every other entry, so
editing an implementation is a new artifact hash and reaches every worker
through the join handshake. `agent-compose build --check` compares those copies
too — a copy that no longer matches what you wrote is a build to re-run, exactly
like a generated file that drifted. Files under `src/` that the composition does
not reference ship nowhere.

## Running it

There is no build step: the TypeScript in `src/` is what runs. Bun is the
default — it is the runtime and the installer this project is documented,
tested and gated against:

```sh
bun install          # installs the pinned dependency set
bun run typecheck    # tsc --noEmit, the type gate
bun src/index.ts
```

`bun src/index.ts` with no arguments starts nothing: loading the project is the
environment check, and there is nothing else a bare launch could mean. With a
verb it is this project's command line, which is exactly what `agent-compose
run` and `agent-compose serve` launch:

```sh
bun src/index.ts run flow.<name> --input goal=... [--session <key>] [--format json]
bun src/index.ts resume <execution-id> [--format json]
bun src/index.ts serve --port 8787
```

`run` prints the flow's `outputs:` as one JSON object on **stdout** and its
report — what ran, which model served each call, what each store did, which
edges were taken — on **stderr**, with the path of the file the whole trace was
written to. `--format json` folds both into one document on stdout instead.
`--session` is the session identity of PRD 5.8: a flow that reaches a
`scope: session` store needs one, and a `run` without it is refused before
anything starts, naming the store — an argument to add rather than a run to
retry, so it exits `2` like an `--input` the flow does not declare. A declared
`manual` trigger may remap it — its `session_key:` is CEL over
a payload whose one member is this argument, and what that expression answers is
the partition the run addresses (grammar 13.2). Declared on no trigger, the
argument is the identity.

`resume` carries on an execution this project's journal holds open — a run the
machine lost, or one that stopped at a `human` pause with nobody to ask. Every
run is journaled as it goes, and the first line `run` writes to stderr is the id
this verb takes (`execution: exec_…`); under `--format json` the same value is
`execution_id`. The graph is re-executed from its entry with every recorded
effect **consumed** — the model answers it got, the results its tools produced,
what its stores read, what a person answered — and only the frontier, the first
effect the journal does not hold, reaches the network. It takes no `--input` and
no `--session`: the invocation it replays is the one the journal recorded.

`serve` starts the app over the composition's declared `http` triggers and
announces where it is listening as one JSON line on stdout. Beside them it
mounts two routes of its own — `GET /executions/:id` for an execution's status
and `POST /executions/:id/resume` — so those two are the app's and a trigger
cannot declare either: the compiler refuses one that does. On start it
**recovers** every execution the journal holds open, before it accepts a
connection, so a pause comes back under the same wait id and a resume URL
prepared against the process that died still finds it; triggers are not
re-fired, and a recovered execution that finishes delivers the `settled` webhook
its request asked for, because the caller who was handed a `202` is waiting to
be told rather than polling. Every callback delivery the journal still holds
pending is picked up in the same pass. What is still tracked in the process
alone is the *report*: a status
route answers `404` for an id neither this process nor the journal knows, and
every execution it has finished, with its outputs and its trace, is held for the
life of the process — so a long-running `serve` grows with the number of
requests it has answered, and restarting it is what reclaims that.

**Who may call, and who a delivery says it is.** A trigger declaring `auth:`
verifies its caller before it reads a payload, and the execution it starts
carries that guard: its `GET /executions/:id` and `POST /executions/:id/resume`
enforce the auth of the trigger that started it, in this process and in the one
that recovers it after a restart. An execution a trigger with no `auth:` began
keeps open routes. A trigger's `callback:` is a subscription to the execution's
**lifecycle** rather than only to its end: one webhook per parking, listing every
pause then open, and one when the execution settles, each carrying the report
the status route serves under the `X-AgentCompose-*` headers the grammar
tabulates. Deliveries are journaled and at-least-once — dedupe on
`X-AgentCompose-Delivery`, order by `X-AgentCompose-Ordinal` — and a delivery
that exhausts its bounded retry schedule is recorded and shown on the status
route rather than failing the run. `AGENT_COMPOSE_CALLBACK_RETRY` shortens that
schedule for a diagnostic run: a comma-separated list of durations
(`0s,1s,2s`), refused at launch if it is not one.

Stopping it stops the graph: `agent-compose serve` passes `SIGINT` and `SIGTERM`
on to this project, which closes the app and exits, so a supervisor that signals
the command is not left with a listener behind it.

### On Node instead

Node **>=22.18.0** is a supported fallback, and nothing here is written for one
runtime: no emitted module reaches for a `Bun` global or a `bun:` import, which
is a gate on the compiler rather than a promise in a README. 22.18 is the floor
because that is where Node stopped flagging type stripping, and it is what
`engines.node` in `package.json` declares:

```sh
npm install          # or: pnpm install
npm run typecheck
node src/index.ts
```

The manifest pins every dependency exactly and asks for nothing
installer-specific — no `packageManager` field, no lockfile, no install-time
script — so bun, npm and pnpm all resolve it to the same versions. The lockfile
your installer writes is yours: `agent-compose build` never writes or removes
one.
"#;

/// The section a **target** declaring a `package_registry:` gets (grammar
/// §14.6, PRD resolved q59).
///
/// A reader of such a project meets two things the install blocks above do not
/// explain: why there are two configuration files they did not write, and what
/// happens when the variable one of them names is not set. The second is the
/// one worth writing down, because neither installer treats it as an error —
/// the reference goes out as text and the registry answers `401`.
fn private_registry(ir: &Ir) -> String {
    let Some(registry) = ir.deploy.package_registry.as_ref() else {
        return String::new();
    };
    let mut section = format!(
        "\n## Installing through a private registry\n\n\
         This project was built for a target whose deploy file declares a\n\
         `package_registry:`, so `bunfig.toml` and `.npmrc` beside this README point both\n\
         installers at `{}`. They are generated files like every other one in the table\n\
         above: edit the deploy file and rebuild, rather than editing them.\n",
        registry.url.value
    );
    if !registry.scopes.is_empty() {
        section.push_str(
            "\nThese scopes resolve somewhere of their own:\n\n| scope | registry |\n|---|---|\n",
        );
        for scope in registry.scopes.values() {
            let _ = writeln!(
                section,
                "| `{}` | `{}` |",
                scope.name.value, scope.url.value
            );
        }
    }
    let mut credentials: Vec<&str> = std::iter::once(registry.token.as_ref())
        .chain(registry.scopes.values().map(|scope| scope.token.as_ref()))
        .flatten()
        .map(|token| token.value.name.as_str())
        .collect();
    credentials.sort_unstable();
    credentials.dedup();
    if credentials.is_empty() {
        section.push_str(
            "\nNeither file carries a credential: this registry is read through without one.\n",
        );
        return section;
    }
    let _ = write!(
        section,
        "\nThe credential is the **name** of an environment variable — {} — and the\n\
         installer expands it when it runs. No secret is in this directory, and rotating\n\
         the token does not change what `agent-compose build` writes here.\n\n\
         **An unset variable is not an install-time error.** Both installers send the\n\
         reference as text and the registry refuses it, so a `401` from the registry is\n\
         what a missing variable looks like at `install`. What catches it earlier is the\n\
         environment check this project already makes: the variable is in\n\
         `environmentReferences`, so loading `src/index.ts` names it, and a worker that\n\
         does not have it is refused at join rather than after a failed install.\n",
        credentials
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    section
}

/// The section a composition declaring a `human` node gets.
///
/// A pause is the one thing an emitted project asks a *person* for, so the
/// reader who has to answer one needs the whole loop written down — and there
/// are **two** ways to answer, so it is written twice over: where the app
/// publishes the question and what a resume's refusals mean, and what a `run` at
/// a terminal shows, how a line answers it, and what decides whether it asks at
/// all (grammar 8.7, PRD 5.11, §9.21). A composition with no `human` node gets
/// none of it, exactly as one with no store gets no store section — the resume
/// route is still mounted, and the paragraph above already says so.
fn human_waits(ir: &Ir, partition: &super::env::Partition) -> String {
    let mut pauses = false;
    let mut placed = false;
    for (address, definition) in &ir.definitions {
        let crate::ir::definition::DefinitionBody::Flow(flow) = &definition.body else {
            continue;
        };
        if !flow
            .nodes
            .iter()
            .any(|node| matches!(node.kind, crate::ir::flow::NodeKind::Human { .. }))
        {
            continue;
        }
        pauses = true;
        // **Which processes can run this flow's instance**, read off the
        // environment partition rather than walked again here: `docs/distributed.md`
        // §9.1's closure is already the answer to that question, attachment
        // included, and it is the same closure grammar 14.1 rule 5 refuses a
        // process-local store on. A placement in the list is a worker that can
        // open this question.
        placed |= partition
            .processes_of(address)
            .any(|process| matches!(process, super::env::Process::Placement(_)));
    }
    if !pauses {
        return String::new();
    }
    let mut section = String::from(HUMAN_WAITS);
    // The one paragraph a **mesh** adds, and only one whose mesh can reach a
    // question: a deployment where no placement's process runs a flow with a
    // `human:` node in it has no worker for a pause to be opened on, and telling
    // its reader what happens when one is would be a paragraph about a shape
    // this deployment cannot produce.
    if placed {
        section.push_str(PLACED_WAITS);
    }
    section
}

const HUMAN_WAITS: &str = r##"
## Answering a `human` node

A flow that reaches a `human` node stops there and its execution reports
`status: "interrupted"`. The status route is where the question is: an
interrupted report carries an `interrupts` array, one entry per pause the
execution is holding, and each entry has everything needed to ask a person and
take their answer.

```json
{
  "execution_id": "exec_0f1e…",
  "flow": "flow.review",
  "trigger": "on_request",
  "status": "interrupted",
  "interrupts": [
    {
      "wait_id": "approve/0",
      "flow": "flow.review",
      "node": "approve",
      "paused_at": "2025-01-01T12:00:00.000Z",
      "expires_at": "2025-01-02T12:00:00.000Z",
      "input": { "draft": "…" },
      "output_schema": { "type": "object", "properties": { "decision": { "enum": ["approve", "reject"] } }, "required": ["decision"], "additionalProperties": false },
      "resume_url": "/executions/exec_0f1e…/resume?wait=approve%2F0"
    }
  ]
}
```

`input` is the node's own `input:`, evaluated — what the human is shown.
`output_schema` is the published JSON Schema of its `output:`, which is exactly
what an answer is validated against — at this route and at the terminal below —
so a form can be built from the report rather than from the composition. `expires_at` is present only where the
node declares a `timeout:`.

POST the answer to `resume_url` as the JSON body:

```sh
curl -X POST "http://127.0.0.1:8787/executions/exec_0f1e…/resume?wait=approve%2F0" \
  -H 'content-type: application/json' \
  -d '{"decision":"approve"}'
```

A `202` means the answer was taken and the graph has gone back to work; poll the
status route for the rest. A payload that does not fit the node's `output:` is a
`400` and **does not consume the wait** — the execution is still interrupted and
the corrected answer can be sent to the same URL. So is a request carrying
`?wait=` more than once, and for the same reason: it names two pauses where a
resume answers one, so it is refused as that — rather than joined into an id
nothing is holding — and consumes neither. A `409` is about *which* pause
rather than about the body: the execution has already completed or failed, so
there is no run left to be waiting; the run is still going and nothing in it is
waiting; the wait already expired and `on_timeout:` has routed the execution on;
the execution is holding more than one pause and the request named none; or
`?wait=` named a pause this execution is not holding — a stale id from an
earlier poll. The last two carry a `pending` array of the ids that *are*
waiting, and `?wait=` is how one of them is named. That id is a pause's
`wait_id`: its instance path, which is stable across runs of one composition —
`approve/0` at the top level of a flow, `review/0/2/approve/0` for the pause
inside the third instance a `map` dispatched. `interrupts` is ordered by
`wait_id`, and so is the list a `409` gives, so two runs of one composition
publish the same questions in the same order however their instances happened
to be scheduled.

**A node above a pause does not spend its budget waiting.** A `timeout:` on the
`flow:` node or `map` that dispatched the flow the pause is in — including one
resolved from `defaults:` — bounds the work that node does, and the wait is not
work it is doing: its clock is held still while a pause below it is open and
resumes with the time it had left. This is what makes the rule "a `human` node
resolves no `timeout` at any level" mean what it says for a pause that is not at
the top level of the triggered flow.

**A retry asks again.** A `retry:` on the node that dispatched the flow a pause
is in re-executes the whole instance from its entry as a fresh attempt (grammar
8.5), so an attempt that fails while somebody is still thinking takes its
question with it: an answer arriving after that is a `409` saying the wait is no
longer held, and the next attempt asks again at the same `wait_id`. Poll the
status route for the question rather than holding on to an `interrupts` entry
from an earlier poll.

**A wait survives the process that opened it.** It is a parked promise rather
than a checkpoint, so a restarted `serve` does not *hold* it — it **replays** the
execution out of this project's journal and parks again, under the same
`wait_id`, because the id is the node's instance path and no process generation
is part of it. So a resume URL handed out by the process that died answers in
the one that replaced it. What the journal does not hold is a wait nobody
answered — there is nothing to record about it — which is exactly what makes
re-parking the right thing to do with one.

**Replaying back to that pause takes as long as it takes**, and the app serves
its other executions meanwhile rather than holding the port until every replay
has landed. So an answer can arrive before the wait is back on the board, and
that request is refused with a refusal of its own: a `409` carrying `recovering:
true` and saying to send it again, rather than one of the ones above that mean
the pause is over.

The same is true of a `run`: a pause it could not ask leaves its execution open
in the journal, and `bun src/index.ts resume <execution-id>` picks it up,
prompting at the terminal exactly as an interactive `run` does.

## Answering a pause at the terminal

The resume route is one way to answer a pause. The other is `run` itself: a run
whose **standard input is a terminal** asks each pause it reaches, right there,
and carries on with the answer. So a flow with a `human` node in it is runnable
without serving anything.

```text
$ bun src/index.ts run flow.review --input goal=ship

pause `approve/0` — flow.review node `approve`
  shown:
    {
      "draft": "the drafted answer"
    }
  answer: { decision: "approve" | "reject", note?: string }
  expires: 2025-01-02T12:00:00.000Z
answer `approve/0` with one line of JSON: {"decision":"approve"}
taken.
```

The prompt goes to **stderr**, so stdout is still only the flow's outputs and
`--format json` still prints exactly the document it always did. `shown` is the
node's `input:`, evaluated; `answer` is a sketch of its `output:` — the full JSON
Schema is what the status route publishes, for a program rather than a person —
and `expires` appears only where the node declares a `timeout:`.

**One JSON value per line.** A value spanning lines has no terminator a prompt
could recognize without either guessing or hanging on a malformed one, so an
answer is a line. A line that is not JSON, and one the node's `output:` refuses,
are both refused and the question is asked again — the wait is not consumed, the
same rule the resume route's `400` follows. A blank line is not an answer at all
and just re-prompts.

**One question at a time.** An execution holding several pauses — a `map` over a
flow that pauses — is asked them one after another, and each question is the
lowest `wait_id` **open when it is asked**: the order the status route publishes
them in, so pauses waiting together are asked in the composition's order rather
than the one the scheduler parked them in. A pause that opens while a question
is on the screen is asked after it, whatever its id sorts as — the question in
front of you is never taken back to make room for it. Each prompt names its own
wait id.

**A budget keeps running while you think.** Nothing about being asked at a
terminal holds a `timeout:` still: a wait that runs out while its question is on
the screen routes through `on_timeout:` exactly as it would under `serve`, and
the prompt is withdrawn saying so before the next question is asked. A line typed
for a question that has just been withdrawn is read as the next question's
answer — a stream of typed lines carries no addressing — which is why every
prompt names the pause it belongs to.

**Standard input ending ends the run.** Close it, or answer fewer questions than
the run asks, and there is nothing left that could answer the rest: the run stops
where it stood, with `status: "interrupted"` and exit `3`, exactly as a run with
no terminal does.

### When there is no terminal

`AGENT_COMPOSE_INTERACTIVE` decides the surface where standard input cannot:

| value | what a `run` does |
|---|---|
| `1` | asks at standard input whatever it is — which is how a **script** answers a pause: `printf '%s\n' '{"decision":"approve"}' \| AGENT_COMPOSE_INTERACTIVE=1 bun src/index.ts run flow.review --input goal=ship` |
| `0` | never asks, even at a terminal — which is how a supervisor keeps a run on the exit-`3` path below |
| unset | asks when standard input is a terminal |

Any other value is refused before the run starts, naming the variable: a command
that could not be run (exit `2`), rather than a setting nothing read.

A run that is not asking reports the pause and exits **`3`**, its own code beside
`1` for a run that produced no answer and `2` for a command that could not be
run. The trace document is still written, with `status: "interrupted"` and the
pause on the entry of the node it stopped at, and the answer goes to `serve`'s
resume route instead.
"##;

/// The paragraph a composition whose **placements can reach a pause** gets.
///
/// A reader of such a project meets a real question the section above does not
/// answer: a placed agent's tool loop can reach a `human` node, so the question
/// is asked in a process the resume route is not served from. The answer is
/// "nothing changes", and that is worth writing down precisely because it is the
/// answer somebody would not assume (`docs/distributed.md` §3.4, PRD resolved
/// q46).
///
/// Emitted on the **closure** rather than on the mere presence of `placements:`,
/// which is the difference between a paragraph about this deployment and a
/// paragraph about meshes in general: a project whose placements hold only
/// components that reach no `human:` node can never produce the shape described
/// here, and telling its reader otherwise is a page of documentation for a
/// question that will not be asked.
const PLACED_WAITS: &str = r##"
## A pause a worker opens

A placed component can reach a `human` node — an agent placed on a machine, with
a `flow.*` in its `tools:` that asks somebody a question — and everything above
is still true of it. The wait board is the hub's, so the question comes home: the
worker settles its dispatch *paused*, the hub puts the wait on the same board a
local pause goes on, and the status route publishes it with the same `wait_id`,
the same `output_schema` and the same `resume_url`. Answer it at that URL.

Two things are worth knowing about what happens next, because they are visible in
the status route while they happen. The worker is **free** the moment it asks —
the dispatch is over, and that machine may be given other work, or be switched
off, while a person thinks. And the answer sends the node **back through
dispatch**: it is queued to its placement again, and if nothing is claiming that
placement at that moment the execution shows a `placement_waits` entry until a
worker joins. What the node does *not* do is start over — the effects it had
already journaled, a model call above all, are replayed rather than re-issued,
and it goes live at the point it stopped.

Both instants on such an entry are **this** machine's, exactly as they are on a
pause opened here: the question is dated when the wait lands on this board and
`expires_at` is that instant plus the node's whole `timeout:`, so the gap between
the two is the budget you wrote, whichever machine reached the node. The worker's
own clock does not reach the status route — a deadline shown to a person has to be
the deadline this process will fire, and two machines' clocks disagree.
"##;

/// The section a composition holding a **built-in tool** gets
/// (grammar §5.5, §6.1, PRD resolved q54).
///
/// Two things a reader of the generated project has to be told and cannot read
/// off the spec: where a workspace nobody named lives — because a run writes
/// there and something has to say what is safe to delete — and what these tools
/// are, which is the one binding whose program the *model* writes. Emitted
/// exactly when the composition holds one, in either spelling, for the reason
/// [`store_data`] is emitted exactly when there is a store.
fn builtin_workspaces(ir: &Ir) -> String {
    let held = ir
        .definitions
        .values()
        .any(|definition| match &definition.body {
            crate::ir::definition::DefinitionBody::Agent(agent) => !agent.builtins.is_empty(),
            crate::ir::definition::DefinitionBody::Tool(tool) => matches!(
                tool.implementation,
                crate::ir::flow::ToolImplementation::Builtin { .. }
            ),
            _ => false,
        });
    if !held {
        return String::new();
    }
    String::from(BUILTIN_WORKSPACES)
}

const BUILTIN_WORKSPACES: &str = r#"
## Where a built-in tool works, and what it may run

This composition holds `builtin.bash` or `builtin.files` — the one binding whose
program the **model** writes rather than the author. Every other tool fixes what
runs at build time and lets the model fill parameters a schema constrains; these
two run the command or the edit a model composes at the moment it composes it.
That is the trust level an `exec:` tool already extends to author-chosen
binaries, extended to the model: an agent holding `builtin.bash` can run anything
the process running this graph can run. There is no container and no syscall
filter in this release; what bounds a call is the workspace, the command
`timeout:`, and the environment the binding declared.

A binding that names a `workspace:` works there. One that does not works here:

```text
.agent-compose/workspaces/<execution id>/     one directory per execution
```

It is made fresh when the execution's first built-in call runs, shared by every
built-in of that execution that took the default, and **removed when the run
settles** — kept only while the execution's journal row stays open, so that a
`resume` finds the files the run had written. A `workspace:` you named is never
removed by this project.
"#;

/// The section a composition holding a `workspace: fresh` coder node gets
/// (grammar §8.9, PRD resolved q61 ruling c).
///
/// [`builtin_workspaces`]' reason, one construct along and for a directory that
/// is not the same directory: a run writes into this one, nothing in the spec
/// names it, and something has to say where it is and what is safe to delete.
/// The two sections are separate because the compositions are — a project may
/// hold either, both, or neither — and because what the reader is told differs:
/// a built-in's default workspace is one per *execution* and a `fresh` one is
/// one per *dispatch*, remade at every attempt.
fn fresh_workspaces(ir: &Ir) -> String {
    let held = ir.definitions.values().any(|definition| {
        let crate::ir::definition::DefinitionBody::Flow(flow) = &definition.body else {
            return false;
        };
        flow.nodes.iter().any(|node| match &node.kind {
            crate::ir::flow::NodeKind::Coder { coder } => {
                coder.workspace.value.expression().is_none()
            }
            _ => false,
        })
    });
    if !held {
        return String::new();
    }
    String::from(FRESH_WORKSPACES)
}

const FRESH_WORKSPACES: &str = r#"
## Where a `workspace: fresh` harness run works

A `coder:` node names the directory its run is contained by, and one of this
composition's writes `workspace: fresh` — which asks this project for a
directory instead of naming one. It is made here:

```text
.agent-compose/workspaces/<execution id>/<instance path>/
```

The instance path is the node's own address inside the run — the node id, its
traversal ordinal, and a frame for every `flow:` node and `map` dispatch above
it — so **two dispatches of one node are two directories**, which is what makes
a fan-out over a coding agent safe to run concurrently.

It is **emptied and remade at the start of every attempt**, so a `retry:` never
meets what the attempt it is retrying left behind, and it is removed when the
run settles, exactly as the built-in workspace above it is — kept only while the
execution's journal row stays open, so a `resume` finds what the run had
written.

The traversal ordinal is in that path, so **a second pass round a cycle is a
second directory**: a coder node a loop sends back to gets `.../0` on the first
traversal and an empty `.../1` on the second, with the first pass's work left in
the directory before it. A `retry:` is not that — it re-runs one attempt at one
ordinal. A loop whose second pass has to build on the first names a directory of
its own instead of taking `fresh`.

What it does **not** contain is a checkout: this project makes the directory and
nothing else. A run that needs source control needs a step in the graph that
puts it there.
"#;

/// The section a composition declaring a `store.*` gets.
///
/// A store is the one construct that leaves something behind on disk, and where
/// it leaves it is a promise this project keeps rather than a detail of
/// `src/stores.ts`: a reader who wants to inspect, back up or delete what a run
/// stored has to be told the layout. A composition with no store gets no
/// section, exactly as one using no `function:` binding gets no host-function
/// section.
/// Which journal this target bound, and where it lives (grammar §14.7, PRD
/// resolved q62).
///
/// Emitted for **every** project, unlike the sections around it, and that is the
/// point: durability is unconditional (Decision D121), so "where does this
/// project's record survive a restart" always has an answer and a reader should
/// not have to know what the default is to find it. What varies is which answer.
///
/// The address is the **variable's name**, never what it holds: `docs/trace.md`
/// §11.1 keeps a resolved `${ENV}` out of every artifact this project writes,
/// and a README is one of them.
fn journal_home(ir: &Ir) -> String {
    let provider = crate::ir::deploy::journal_of(ir);
    let mut section = String::from(JOURNAL_HOME);
    if provider.opens_in_process() {
        section.push_str(JOURNAL_HOME_SQLITE);
        return section;
    }
    let variable = ir
        .deploy
        .journal
        .as_ref()
        .and_then(|journal| journal.url.as_ref())
        .map(|url| url.value.name.clone())
        .unwrap_or_default();
    let mut drivers = String::from("\n| package | version |\n|---|---|\n");
    for (package, version) in super::journal::pins_of(provider)
        .iter()
        .chain(super::journal::development_pins_of(provider))
    {
        let _ = writeln!(drivers, "| `{package}` | `{version}` |");
    }
    let _ = write!(
        section,
        "This target binds a **`{}`** journal, at the address `${{{variable}}}` holds.\n\
         The variable is named here and never its value: `{variable}` is read at\n\
         process start, and `run`, `serve` and `resume` all fail before the graph is\n\
         invoked when it is unset.\n\
         \n\
         The record lives on that server rather than in this directory, which is what\n\
         lets a `serve` restarted on another machine recover every execution this one\n\
         left open — and what makes retention a `DELETE` rather than removing a file.\n\
         This project therefore pins the driver its journal is reached through, in\n\
         `package.json` beside every other pin; a target that binds the default\n\
         journal pins none of these:\n\
         {}\n\
         **One process at a time writes this journal**, and the server holds a\n\
         session-scoped lock saying which. A second process that opens it is refused\n\
         by name rather than left to interleave; the lock goes with the connection, so\n\
         a process that died is never what is holding it.\n\
         \n\
         Because the journal holds what a trace deliberately does not — completions,\n\
         tool results, a person's answer — it is private recovery data with the same\n\
         sensitivity as this project's stores. Binding it into a shared database is\n\
         this deploy file's explicit choice about where those payloads live.\n",
        provider.as_str(),
        drivers,
    );
    section
}

/// The heading both arms of [`journal_home`] open with.
const JOURNAL_HOME: &str = r#"
## Where this project's journal lives

Every invocation of every flow is **journaled** — every model answer, every tool
result, every store op, every answer a person gave a `human:` node — and a
resumed execution consumes that record rather than re-issuing it, up to the
frontier. Nothing turns it on and nothing turns it off. What the deploy layer
chooses is only where the record goes.

"#;

/// …and the paragraph a target that bound the default gets.
const JOURNAL_HOME_SQLITE: &str = r#"This target binds the default: one **SQLite** file beside this project's stores,
at `.agent-compose/journal.sqlite`, moved as a whole by `AGENT_COMPOSE_DATA_DIR`
and deleted by deleting it. It costs no configuration and no dependency this
project did not already have. A target that needs the record to outlive this
machine binds `journal: { provider: postgres | mysql, url: ${SOME_VAR} }` in its
deploy file instead.

Because the journal holds what a trace deliberately does not — completions, tool
results, a person's answer — it is private recovery data with the same
sensitivity as this project's stores. Nothing uploads it and no command prints
it.
"#;

fn store_data(ir: &Ir) -> String {
    let stores = ir.definitions.values().any(|definition| {
        matches!(
            definition.body,
            crate::ir::definition::DefinitionBody::Store(_)
        )
    });
    if !stores {
        return String::new();
    }
    String::from(STORE_DATA)
}

const STORE_DATA: &str = r#"
## Where a store keeps its data

`--target local` substitutes SQLite and local disk for every store
unconditionally, so a composition with a `store.*` in it runs with nothing
installed (PRD 5.8). What it writes lives under this directory:

```text
.agent-compose/stores/<name>.sqlite                      a `kv` or `vector` store
.agent-compose/blobs/<name>/<partition>/values/<key>     a `blob` store
.agent-compose/journal.sqlite                            every effect every execution issued
.agent-compose/traces/<flow>-<execution id>.json         what `run` wrote out
```

`<partition>` is the store's declared `scope:` made concrete — `global`,
`session/<session key>`, or `execution/<execution id>` — so one file holds every
session and a read never sees another's. A `scope: execution` `kv` or `vector`
store is held in memory and a `blob` one is a partition beside the others, and
both go when the run **ends**, which is what "dies with the run" means. A run
that stopped at a `human` pause nobody answered has not ended — its journal row
is open and `resume` picks it up — so what it wrote is still there for the
generation that finishes it.
`AGENT_COMPOSE_DATA_DIR` moves the whole directory; the paths under it stay the
same. It is derived from this project's own location rather than from the
working directory, so a graph reads the same store wherever it was launched from.

Two things under the directory are not a store's. The **traces** above, which
`agent-compose run` writes and names on stderr: each is one JSON object — the
trace envelope, carrying `trace_version`, the flow, the execution id, how the run
ended, and the run's `entries` — rather than a bare list, so a file found on its
own says which format it is in. The whole directory is listed in `.gitignore` —
what a run produced is not what a build emitted.

And the **journal**, which is what makes an execution survive the process that
started it: every effect a run issued, written as it happened, so a `resume` (or
a restarted `serve`) can hand the graph back what it already got instead of
asking for it again. Read it as **private recovery data with the same
sensitivity as your stores** rather than as a log: unlike a trace, which
deliberately carries no completion, no tool result and no human's answer, the
journal carries all three in full, because a replay cannot work without them.
Nothing sends it anywhere and no command prints it. Deleting it costs exactly
the ability to resume the executions it holds open.

### One process at a time

**Run one of these at a time against one project.** The local backends are the
zero-infra ones: `kv` and `vector` are a SQLite database opened through a
WebAssembly build over `node:fs`, whose locking is a directory beside the file
rather than the operating system's — so two `agent-compose run`s sharing a
`session` or `global` store contend for it, and the loser waits and then fails
the node with `SQLite3Error: database is locked`. It fails loudly rather than
corrupting anything, and a `serve` process — which runs its executions in **one**
process — is not affected. Concurrency across processes arrives with the
production `storage_backends:` of a later milestone; until then `--target local`
means one process, which is the same boundary the target draws everywhere else.

**Opening** is the concession to that, and both artifacts make it: the journal
and the stores wait for a lock rather than failing on one, so an
`agent-compose resume` typed beside a live `serve` gets in. Waiting is also what
makes a **crash** survivable here. That lock directory does not die with the
process holding it, so a run killed mid-write leaves one nothing else would ever
remove — and left standing it would refuse every later open of that file, for
ever. So a lock still there after the wait is treated as its dead owner's and
removed. Two processes *running* against one project is still the case above.

**Nothing here is pruned.** A store keeps what was written to it until you
delete the file, and that includes the idempotency ledger a keyed write leaves
beside its effect (the `applied` table of a SQLite store, the `applied`
directory of a blob one — one entry per key) — so a long-lived
`global` store's ledger grows with the number of writes ever made to it, and so
do the journal and the traces directory. Retention is yours: everything under
`.agent-compose/` is safe to remove between runs, and removing it is what "start
clean" means — at the cost, for the journal, of the executions it was holding
open.
"#;

/// The section a composition with a `route_on: [timeout]` route gets.
///
/// Three of grammar 12.2's conditions are answers a provider sends, and the
/// runtime classifies each from what came back. The fourth is the absence of an
/// answer, and it is the only one whose behaviour depends on something the
/// author writes somewhere else: a node's `timeout:`, which is the budget the
/// ladder divides between the members that still have a successor (see
/// `callModel` in `src/runtime.ts`). A composition with no `timeout:` anywhere
/// has declared no wall-clock bound — grammar 9.3's built-in level is "no
/// timeout" — so nothing measures the silence and the condition never fires.
///
/// That is not something the signature of a `route:` shows, and it is exactly
/// the kind of thing a reader discovers at three in the morning, so it is
/// written where they meet the key. Emitted only for a route that declares the
/// condition, because for every other composition it would be advice about a
/// key it does not have.
fn route_timeouts(ir: &Ir) -> String {
    let declared = ir.definitions.values().any(|definition| {
        let crate::ir::definition::DefinitionBody::Model(crate::ir::definition::Model::Route(
            route,
        )) = &definition.body
        else {
            return false;
        };
        // An absent `route_on:` is grammar 12.2's default, which names
        // `timeout` — so the section belongs to a route that said nothing as
        // much as to one that said this.
        route.route_on.as_ref().is_none_or(|conditions| {
            conditions.iter().any(|condition| {
                matches!(
                    condition.value,
                    crate::ast::definition::RouteCondition::Timeout
                )
            })
        })
    });
    if !declared {
        return String::new();
    }
    String::from(ROUTE_TIMEOUTS)
}

const ROUTE_TIMEOUTS: &str = r#"
## `route_on: [timeout]` needs a `timeout:`

A model route fails over on the conditions its `route_on:` names, and three of
them are things a provider says: a 429 is `rate_limit`, a 529 or a 503 is
`overloaded`, any other 5xx is `server_error`. `timeout` is the one that is not.
A provider that accepted the request and answers nothing says nothing at all, so
the runtime has to decide when to stop waiting — and what it decides that
against is the **node's own `timeout:`** (grammar 9.2), divided between the
members that still have one after them. The first member of a two-member route
under `timeout: 30s` is given 15 seconds; a member that does not answer inside
its share fails over, and the last member keeps whatever is left, so the ladder
never outlives the node's budget.

A node that resolves **no** `timeout:` — none of its own, none from an
instantiating `policy:`, none from `defaults:` — has declared no wall-clock bound
at all (grammar 9.3's built-in level is "no retry, no timeout"), so there is
nothing for a share to be a share of. Such a node waits on a silent provider for
as long as it stays silent, exactly as it would with a single model, and the
`timeout` in its `route_on:` never fires. A dropped connection, a refused socket
and a name that does not resolve are a different case: the socket reports those,
so they classify as `timeout` and fail over whether or not a budget is declared.
"#;

/// The pins table's own heading, emitted after every conditional section.
const README_PINS: &str = r#"
## Pinned versions

A compiler release targets one LangGraph release (PRD 5.12). Upgrading is a
change to the compiler, not to this directory: bump the pins there, rebuild, and
review the diff.
"#;

/// The section a composition using grammar 6.1's `function:` binding gets.
///
/// The escape hatch is the one construct that makes a composition non-portable
/// (PRD 5.5), and the shape of that cost is concrete: the project does not run
/// until the host has registered an implementation. A reader of the generated
/// project finds the list here rather than in a runtime error.
///
/// The section also states what a `timeout:` means over code the compiler did
/// not write. The runtime **races** the node's deadline (see `runActivity` in
/// `src/runtime.ts`), so the node fails on time whatever the implementation does
/// with `context.signal` — but nothing can unschedule the abandoned call, and a
/// host that wants the work itself to stop has to observe the signal. That is
/// the one thing about registering a function which is neither in the grammar
/// nor visible from the signature.
///
/// The idempotency section states the exception to it, because the two are the
/// same call: a function reached as the sink of a **detached** dispatch runs on
/// a signal nothing aborts (see `runMap` in `src/runtime.ts`), so a host that
/// read the paragraph above and returned early on `signal.aborted` would be
/// writing dead code in the one implementation where a lost call is a lost
/// message.
fn host_functions(ir: &Ir) -> String {
    let registered = super::graph::host_functions(ir);
    if registered.is_empty() {
        return String::new();
    }
    let mut text = String::from(
        "\n## Host functions\n\n\
         This composition uses grammar 6.1's `function:` binding, which is the escape hatch\n\
         that puts an implementation outside the spec (PRD 5.5). Each one below has to be\n\
         registered before a graph that reaches it runs:\n\n\
         ```ts\n\
         import { registerFunction } from \"./src/runtime.ts\";\n\n",
    );
    for (name, address) in &registered {
        text.push_str(&format!(
            "registerFunction({name:?}, async (args, context) => {{\n  \
             // the implementation of `{address}`; its arguments have already been\n  \
             // parsed against that tool's declared `input:` schema\n  \
             return {{ /* … its declared `output:` … */ }};\n\
             }});\n"
        ));
    }
    text.push_str(
        "```\n\n\
         ### What a node's `timeout:` means here\n\n\
         The second argument carries `context.signal`, which aborts when the node's\n\
         `timeout:` budget runs out — on every call but one, and that exception is the\n\
         next section's. Observing it is **optional for the node and necessary for the\n\
         work**: the runtime races the deadline against the call, so the node fails on\n\
         time and the run moves on whether or not the implementation looks — but nothing\n\
         can unschedule a call already in flight. An implementation that ignores the\n\
         signal keeps running after the node it belonged to has failed, and whatever it\n\
         eventually returns is discarded.\n\n\
         ### When `context.idempotency_key` is set\n\n\
         A function reached as the sink of a **detached** `map` dispatch is delivered\n\
         at-least-once: the fan-out never waits for its outcome, so a retried map node\n\
         issues the delivery again. `context.idempotency_key` is that dispatch's key\n\
         (grammar 9.4) and it is stable across every attempt of the same item, so an\n\
         implementation that records what it has already done under this key can drop a\n\
         repeat. It is **absent on every other call**, where repeating a call is what a\n\
         composition asked for.\n\n\
         This is also the one call whose `context.signal` is not the map node's: a\n\
         detached delivery is off that node's clock, because the fan-out never waited\n\
         for it and a delivery the node's `max_concurrency` had merely delayed past the\n\
         budget would otherwise be dropped instead of sent. A sink is cancelled by\n\
         nothing, and its signal is there so that it has the same shape as every other\n\
         invocation.\n",
    );
    text
}

const README_TAIL: &str = r#"
## Ejecting

Copy this directory somewhere else and stop regenerating it. It is a plain
TypeScript project that runs on Bun and on Node — no toolchain of ours is
required to build, run, or publish it — which is the eject path PRD 5.12 asks
for.
"#;

/// `.gitignore`.
#[must_use]
pub fn gitignore(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "# ");
    contents.push_str(GITIGNORE);
    super::GeneratedFile {
        path: ".gitignore".to_string(),
        contents,
    }
}

const GITIGNORE: &str = "\
#
# A generated project is meant to be committed — `build --check` in CI is what
# that buys (PRD §8). These three are the exceptions: an install artifact, the
# thing the spec deliberately never contains, and the data this project's own
# stores keep (PRD 5.8).
node_modules/
.env
.agent-compose/
";

/// `src/index.ts`.
#[must_use]
pub fn index(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(INDEX);
    super::GeneratedFile {
        path: "src/index.ts".to_string(),
        contents,
    }
}

const INDEX: &str = r#"//
// The project's public surface, and its entry point. Everything a consumer of
// this graph needs — the schemas, the state model, the graph itself, `runFlow`,
// and the environment it requires — is re-exported here, so an ejected project
// has one entry point and `run`/`serve` have one module to launch.
//
// It is also where the env-ref presence check of PRD 5.9 runs. `readEnvironment`
// is called at module scope, so loading this module is what "process start"
// means for this project: any `bun src/index.ts` or `node src/index.ts`, and any
// import of it, throws naming every missing variable before a graph is built or
// a model is called.
// The compiler never runs it — `agent-compose build` reads no environment, which
// is what keeps a build on one machine reproducible on another and keeps a
// credential out of every file it writes (PRD 5.9: refs "survive into the IR
// unresolved").
//
// # Running it, rather than importing it
//
// Launched **as a program** it is this project's command line (`./cli.ts`):
//
// ```sh
// bun src/index.ts run flow.<name> --input k=v
// bun src/index.ts serve --port 8787
// ```
//
// `agent-compose run` and `agent-compose serve` launch exactly that, so the
// command a reader is given in README.md is the command the compiler runs. With
// no verb it starts nothing: loading the project is the environment check and
// there is nothing else for a bare launch to mean.
//
// The dispatch is guarded on this being the **entry** module: importing the
// barrel from a host script must not turn that script's own arguments into a
// verb. The guard compares `import.meta.url` against the entry path through
// `pathToFileURL`, which both supported runtimes answer the same way — rather
// than through the one-word property Bun has had for longer than Node, which a
// generated module may not reach for (PRD §9.18). `./cli.ts` is loaded only when
// there is a verb, so an import pays for neither the command line nor the HTTP
// framework behind `serve`.

import process from "node:process";
import { pathToFileURL } from "node:url";

import { readEnvironment } from "./env.ts";

export * from "./deployment.ts";
export * from "./env.ts";
export * from "./graph.ts";
export * from "./schemas.ts";
export * from "./state.ts";

readEnvironment();

const entry = process.argv[1];
if (entry !== undefined && pathToFileURL(entry).href === import.meta.url && process.argv.length > 2) {
  const { runMain } = await import("./cli.ts");
  await runMain(process.argv.slice(2));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::{ir_of, ir_of_mesh};

    fn manifest() -> String {
        package_json(&ir_of("version: \"0.1\"\n")).contents
    }

    /// The README one composition emits, over the partition [`readme`] reads its
    /// placement-dependent sections off — the one [`super::super::emit`] hands
    /// it, derived here rather than passed in because a test builds one IR.
    fn readme_of(ir: &Ir) -> crate::codegen::GeneratedFile {
        readme(ir, &crate::codegen::env::Partition::of(ir))
    }

    /// A `module:` binding's `dependencies:` are folded into the generated
    /// manifest, which stays pure-generated (PRD resolved q49, D133).
    ///
    /// Three claims in one composition, because each is a different way the
    /// fold can be wrong: the runtime's own pins keep their curated order and
    /// their versions; two tools naming one package at one version fold into
    /// **one** entry rather than two; and the declared set is sorted by package
    /// name, so a package that moved between two bindings does not rewrite the
    /// file.
    #[test]
    fn a_module_bindings_dependencies_are_folded_into_the_generated_manifest() {
        let ir = ir_of(
            r#"version: "0.1"

tool.verify:
  description: Verify a signature.
  input: {}
  output: {}
  module:
    path: ./src/tools/verify.ts
    dependencies:
      "@noble/hashes": "1.4.0"

tool.sign:
  description: Sign a payload.
  input: {}
  output: {}
  module:
    path: ./src/tools/sign.ts
    dependencies:
      "@noble/hashes": "1.4.0"
      tweetnacl: "1.0.3"
"#,
        );
        assert_eq!(
            dependencies(&ir),
            PINS.iter()
                .map(|(package, version)| ((*package).to_string(), (*version).to_string()))
                .chain([
                    ("@noble/hashes".to_string(), "1.4.0".to_string()),
                    ("tweetnacl".to_string(), "1.0.3".to_string()),
                ])
                .collect::<Vec<_>>(),
            "the pins keep their order, the declared set is sorted after them, \
             and one package agreed on twice is one entry"
        );

        let contents = package_json(&ir).contents;
        let parsed: serde_json::Value = serde_json::from_str(&contents).expect("strict JSON");
        let block = parsed["dependencies"]
            .as_object()
            .expect("a dependency block");
        assert_eq!(block["@noble/hashes"], "1.4.0", "{contents}");
        assert_eq!(block["tweetnacl"], "1.0.3", "{contents}");
        assert_eq!(block.len(), PINS.len() + 2, "{contents}");
        assert!(
            contents.contains("generated by agent-compose"),
            "the manifest is still wholly the compiler's: {contents}"
        );
    }

    /// A package the generated project already holds is folded away whichever
    /// block holds it — `dependencies` or `devDependencies`.
    ///
    /// The parser accepts a pin that **agrees** with the generated project's own
    /// (`parse::binding`, which chains both lists), so a module may perfectly
    /// well declare `typescript` at the version this release pins. What must not
    /// follow is the same package under two keys of one manifest: `npm` warns
    /// and drops one of them, and a manifest whose two blocks disagree about a
    /// package is exactly what the exact-pin rule exists to prevent.
    #[test]
    fn a_package_the_generated_project_already_holds_is_not_declared_twice() {
        let ir = ir_of(
            r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input: {}
  output: {}
  module:
    path: ./src/tools/sign.ts
    dependencies:
      typescript: "7.0.2"
      zod: "4.4.3"
"#,
        );
        assert_eq!(
            dependencies(&ir),
            PINS.iter()
                .map(|(package, version)| ((*package).to_string(), (*version).to_string()))
                .collect::<Vec<_>>(),
            "a dev pin and a runtime pin agreed with are both already held"
        );

        let contents = package_json(&ir).contents;
        let parsed: serde_json::Value = serde_json::from_str(&contents).expect("strict JSON");
        assert!(
            parsed["dependencies"]["typescript"].is_null(),
            "`typescript` is a devDependency of the generated project: {contents}"
        );
        assert_eq!(
            parsed["devDependencies"]["typescript"], "7.0.2",
            "{contents}"
        );
        assert_eq!(
            parsed["dependencies"]
                .as_object()
                .expect("a dependency block")
                .len(),
            PINS.len(),
            "{contents}"
        );
    }

    /// A composition binding no module emits the pins and nothing else, which is
    /// what makes the fold above a *fold* rather than a rewrite.
    #[test]
    fn a_composition_with_no_module_binding_pins_exactly_the_runtimes_own_set() {
        assert_eq!(
            dependencies(&ir_of("version: \"0.1\"\n")),
            PINS.iter()
                .map(|(package, version)| ((*package).to_string(), (*version).to_string()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_manifest_is_json_and_carries_the_header_in_a_comment_key() {
        let contents = manifest();
        let parsed: serde_json::Value =
            serde_json::from_str(&contents).expect("the manifest is strict JSON");
        let header = parsed["//"]
            .as_array()
            .expect("the header is an array of lines");
        assert!(
            header[0]
                .as_str()
                .is_some_and(|line| line.contains("generated by agent-compose")),
            "{contents}"
        );
    }

    #[test]
    fn every_dependency_is_pinned_exactly() {
        let contents = manifest();
        let parsed: serde_json::Value = serde_json::from_str(&contents).expect("strict JSON");
        for section in ["dependencies", "devDependencies"] {
            let block = parsed[section].as_object().expect("a dependency block");
            assert!(!block.is_empty(), "`{section}` is empty");
            for (package, version) in block {
                let version = version.as_str().expect("a version string");
                assert!(
                    version
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_digit()),
                    "`{package}` is not pinned exactly: `{version}`"
                );
            }
        }
    }

    /// The neutrality claim of the module docs, as an assertion: nothing in the
    /// manifest names an installer or asks one to run anything at install time.
    ///
    /// `packageManager` is the interesting one now that Bun is the default
    /// (PRD §9.18). Defaulting to Bun is a claim about the documented commands,
    /// not about the manifest: the field is corepack's, corepack does not manage
    /// Bun, and emitting `packageManager: "bun@x.y.z"` would pin every generated
    /// project to a Bun release nothing here tests while buying no determinism
    /// the exact version pins do not already give. So the list below is not a
    /// leftover from a neutral era — it is what keeps the default a default.
    #[test]
    fn the_manifest_stays_package_manager_neutral() {
        let parsed: serde_json::Value = serde_json::from_str(&manifest()).expect("strict JSON");
        let object = parsed.as_object().expect("an object");
        for forbidden in [
            "packageManager",
            "workspaces",
            "resolutions",
            "overrides",
            "pnpm",
            "bundledDependencies",
        ] {
            assert!(
                !object.contains_key(forbidden),
                "`{forbidden}` names one installer"
            );
        }
        let scripts = parsed["scripts"].as_object().expect("a scripts block");
        for lifecycle in ["preinstall", "install", "postinstall", "prepare"] {
            assert!(
                !scripts.contains_key(lifecycle),
                "`{lifecycle}` runs at install time"
            );
        }
    }

    /// The emitted README launches with Bun and keeps Node as the fallback
    /// (PRD §9.18).
    ///
    /// Which command a reader is shown *first* is the whole of what "default"
    /// means for a directory that carries no launcher of its own, so the order
    /// is asserted rather than the mere presence of both. The Node half is
    /// asserted too: it is the promise gate 13 of `tests/generated_code_gates.rs`
    /// keeps, and a README that quietly dropped it would leave that gate
    /// checking something nobody had been told about.
    #[test]
    fn the_readme_launches_with_bun_and_documents_the_node_fallback() {
        let contents = readme_of(&ir_of("version: \"0.1\"\n")).contents;

        let bun = contents
            .find("bun src/index.ts")
            .expect("the default launch");
        let node = contents
            .find("node src/index.ts")
            .expect("the fallback launch");
        assert!(
            bun < node,
            "the fallback is documented before the default: {contents}"
        );

        let install = contents.find("bun install").expect("the default install");
        assert!(
            install < contents.find("npm install").expect("the fallback install"),
            "{contents}"
        );

        assert!(contents.contains("### On Node instead"), "{contents}");
        assert!(
            contents.contains(&format!("Node **{NODE_ENGINE}** is a supported fallback")),
            "the README's floor is not the one `engines.node` declares: {contents}"
        );
        // The one sentence that makes the fallback checkable rather than
        // aspirational, and the reason gate 14 exists.
        assert!(
            contents.contains("no emitted module reaches for a `Bun` global or a `bun:` import"),
            "{contents}"
        );
        // …and the manifest field that would contradict all of it.
        assert!(contents.contains("no `packageManager` field"), "{contents}");
    }

    /// The paragraph about a pause a **worker** opens is emitted for a
    /// deployment whose placements can actually reach a `human:` node, and for
    /// no other (`docs/distributed.md` §3.4, §9.1, PRD resolved q46).
    ///
    /// The gate is the environment partition's closure rather than the presence
    /// of a `placements:` block, and the pair below differ in exactly one token
    /// — which agent is the placement's member — so what this asserts is the
    /// closure and not the section's prose. A deployment whose placement holds
    /// only an agent that attaches nothing can never produce a pause on a
    /// worker, and a paragraph about one would be documentation for a shape it
    /// cannot reach; the composition still pauses on the hub, so the section
    /// about answering a question is in both.
    #[test]
    fn only_a_deployment_whose_placement_can_reach_a_pause_is_told_about_one() {
        const ASKS: &str = r#"version: "0.1"

state:
  approval: { type: string, default: "" }
  filed: { type: string, default: "" }

provider.p:
  kind: anthropic
  api_key: ${MODEL_KEY}

model.m:
  provider: provider.p
  id: some-model

agent.escalator:
  model: model.m
  prompt: Ask a person about it.
  tools: [flow.approve]
  input:
    goal: { type: string }
  output:
    approval: { type: string }

agent.filer:
  model: model.m
  prompt: File it.
  input:
    goal: { type: string }
  output:
    filed: { type: string }

flow.approve:
  description: Ask a person whether it may go ahead.
  inputs:
    goal: { type: string }
  outputs:
    approval: { type: string }
  nodes:
    ask:
      human:
        input:
          goal: { type: string }
        output:
          decision: { enum: [approve, reject] }
      input:
        goal: "input.goal"
      writes:
        decision: approval
  edges:
    - { from: start, to: ask }
    - { from: ask, to: end }

flow.work:
  inputs:
    goal: { type: string }
  outputs:
    filed: { type: string }
  nodes:
    escalate: { agent: agent.escalator }
    file: { agent: agent.filer }
  edges:
    - { from: start, to: escalate }
    - { from: escalate, to: file }
    - { from: file, to: end }
"#;
        let placing = |member: &str| {
            readme_of(&ir_of_mesh(
                ASKS,
                &format!(
                    "version: \"0.1\"\n\n\
                     hub:\n  join_token: ${{MESH_JOIN_TOKEN}}\n\n\
                     placements:\n  mac:\n    members: [{member}]\n"
                ),
            ))
            .contents
        };

        let unreachable = placing("agent.filer");
        assert!(
            unreachable.contains("## Answering a `human` node"),
            "a composition that pauses is still told how a pause is answered: {unreachable}"
        );
        assert!(
            !unreachable.contains("## A pause a worker opens"),
            "a placement that reaches no question is told what happens when its worker asks \
             one: {unreachable}"
        );

        let reachable = placing("agent.escalator");
        assert!(
            reachable.contains("## A pause a worker opens"),
            "the placed agent attaches the flow holding the question, so its worker can open \
             one: {reachable}"
        );
    }

    /// A composition with a `store.*` is told where its data goes, and one
    /// without gets no section — the same rule the host-function section
    /// follows.
    ///
    /// A store is the one construct that leaves something behind on disk, so a
    /// reader who wants to inspect, back up or delete what a run stored has to
    /// be told the layout. The ordering is asserted too: the section is the
    /// project's, and it belongs with the rest of what the project does rather
    /// than after the version table nobody reads to the end of.
    #[test]
    fn a_composition_with_a_store_is_told_where_its_data_goes() {
        let plain = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            !plain.contains("## Where a store keeps its data"),
            "a composition with no store keeps nothing"
        );

        let contents = readme_of(&ir_of(
            r#"version: "0.1"

store.prefs:
  kind: kv
  scope: session
  description: What this session was told.
  value_schema:
    theme: { type: string }
"#,
        ))
        .contents;
        let section = contents
            .find("## Where a store keeps its data")
            .expect("the section is emitted");
        assert!(
            contents.contains(".agent-compose/stores/<name>.sqlite"),
            "{contents}"
        );
        assert!(
            contents.contains("session/<session key>"),
            "the partition a `scope:` becomes: {contents}"
        );
        assert!(
            contents.contains("AGENT_COMPOSE_DATA_DIR"),
            "…and the one variable that moves it: {contents}"
        );
        assert!(
            section
                < contents
                    .find("## Pinned versions")
                    .expect("the pin table has a heading"),
            "the section sits with the rest of what the project does"
        );
        assert!(
            section > contents.find("### On Node instead").expect("the fallback"),
            "…and after the launch instructions it is about"
        );
    }

    /// A composition holding a built-in is told where it works and what it may
    /// run (grammar §5.5, §6.1, PRD resolved q54).
    ///
    /// Two things the spec cannot tell the reader of a *project*: where a
    /// workspace nobody named lives, which is a directory a run writes into and
    /// something has to say is safe to delete; and the trust level, which is the
    /// one thing about these tools that is not a key. Both spellings reach it —
    /// the shorthand on an agent and a `tool.*` with a `builtin:` binding — and
    /// a composition holding neither is not told about a directory it has no
    /// tool to fill.
    #[test]
    fn a_composition_with_a_built_in_is_told_where_it_works() {
        let plain = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            !plain.contains("## Where a built-in tool works"),
            "a composition with no built-in has no workspace to describe"
        );

        let agent = r#"version: "0.1"

provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.p
  id: claude-sonnet-4-6

agent.worker:
  description: Works in a directory.
  model: model.smart
  prompt: Do the thing.
  tools: [builtin.bash]
  output:
    summary: { type: string }
"#;
        let shorthand = readme_of(&ir_of(agent)).contents;
        assert!(
            shorthand.contains(".agent-compose/workspaces/<execution id>/"),
            "the directory a defaulted workspace is: {shorthand}"
        );
        assert!(
            shorthand.contains(
                "removed when the run
settles"
            ),
            "…and its lifetime, which is what makes it safe to delete: {shorthand}"
        );
        assert!(
            shorthand.contains(
                "can run anything
the process running this graph can run"
            ),
            "…and the trust level, said rather than implied (D135): {shorthand}"
        );

        // The other spelling reaches the same section: a `tool.*` carrying a
        // `builtin:` binding, attached by nothing at all.
        let configured = readme_of(&ir_of(
            r#"version: "0.1"

tool.sandbox:
  builtin: files
  workspace: "${WORK_DIR}"
"#,
        ))
        .contents;
        assert!(
            configured.contains("## Where a built-in tool works"),
            "{configured}"
        );
    }

    /// A composition holding a `workspace: fresh` coder node is told where that
    /// directory is (grammar §8.9, PRD resolved q61 ruling c).
    ///
    /// The built-in section's reason for a directory that is not the same
    /// directory: this one is per **dispatch** rather than per execution and is
    /// remade at every attempt, and a reader of the project has no other way to
    /// learn either — the spec says `fresh` and nothing else. A coder node that
    /// names its own workspace gets no section, because the composition named
    /// the directory and its lifetime is the composition's.
    #[test]
    fn a_composition_taking_a_fresh_workspace_is_told_where_it_is() {
        let coder = |workspace: &str| {
            format!(
                r#"version: "0.1"

provider.p:
  kind: anthropic
  api_key: ${{K}}

model.smart:
  provider: provider.p
  id: claude-sonnet-4-5

flow.main:
  outputs:
    summary: {{ type: string }}
  nodes:
    build:
      coder:
        harness: cc
        model: model.smart
        workspace: {workspace}
        prompt: Do the work.
        output:
          summary: {{ type: string }}
      input: "'go'"
  edges:
    - {{ from: start, to: build }}
    - {{ from: build, to: end }}
"#
            )
        };

        let named = readme_of(&ir_of(&coder("\"'/srv/checkout'\""))).contents;
        assert!(
            !named.contains("## Where a `workspace: fresh` harness run works"),
            "a node that named its own directory is not told about one this \
             project makes: {named}"
        );

        let provisioned = readme_of(&ir_of(&coder("fresh"))).contents;
        assert!(
            provisioned.contains(".agent-compose/workspaces/<execution id>/<instance path>/"),
            "the directory `fresh` derives: {provisioned}"
        );
        assert!(
            provisioned.contains("two dispatches of one node are two directories"),
            "…and the property that makes a fan-out safe: {provisioned}"
        );
        assert!(
            provisioned.contains("emptied and remade at the start of every attempt"),
            "…and what a retry finds there: {provisioned}"
        );
        assert!(
            provisioned.contains("a second pass round a cycle is"),
            "…and what a *cycle* finds there, which is the other reading of the \
             same path and the one a review loop meets: {provisioned}"
        );
        assert!(
            provisioned.contains("does **not** contain is a checkout"),
            "…and the thing it deliberately does not do: {provisioned}"
        );
    }

    /// A route that can fail over on `timeout` is told what measures one.
    ///
    /// The condition is the only one whose behaviour depends on a key written
    /// somewhere else — a node's `timeout:` — so a reader of a project with a
    /// route in it finds that here rather than in a run that waited forever. A
    /// project with no such route is not told about a key it does not have.
    #[test]
    fn a_route_that_fails_over_on_timeout_is_told_what_measures_one() {
        let models = |route_on: &str| {
            format!(
                r#"version: "0.1"

provider.p:
  kind: anthropic
  api_key: ${{K}}

model.smart:
  provider: provider.p
  id: one

model.fast:
  provider: provider.p
  id: two

model.default:
  route: [model.smart, model.fast]{route_on}
"#
            )
        };

        let plain = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            !plain.contains("## `route_on: [timeout]` needs a `timeout:`"),
            "a composition with no route is not told about one"
        );
        let narrowed = readme_of(&ir_of(&models("\n  route_on: [rate_limit]"))).contents;
        assert!(
            !narrowed.contains("## `route_on: [timeout]` needs a `timeout:`"),
            "…nor is a route that declared the condition away: {narrowed}"
        );

        for route_on in ["", "\n  route_on: [overloaded, timeout]"] {
            let contents = readme_of(&ir_of(&models(route_on))).contents;
            let section = contents
                .find("## `route_on: [timeout]` needs a `timeout:`")
                .unwrap_or_else(|| {
                    panic!("the section is emitted for `route_on:{route_on:?}`: {contents}")
                });
            assert!(
                contents.contains("grammar 9.3's built-in level is \"no retry, no timeout\""),
                "…and says what a node with no budget does: {contents}"
            );
            assert!(
                section
                    < contents
                        .find("## Pinned versions")
                        .expect("the pin table has a heading"),
                "the section sits with the rest of what the project does"
            );
        }
    }

    /// The README's Layout table and [`crate::codegen::emitted_paths`] are one
    /// list, in both directions.
    ///
    /// That table is **normative** in the emitted project rather than a summary
    /// of it: the paragraph under it says "the file list above is the boundary",
    /// so a reader decides whether a file is theirs by looking for its name
    /// there. A row missing for a name the build replaces tells the owner of a
    /// `package.json` they edited that it is theirs, right up until the next
    /// build overwrites it; a row with nothing behind it promises a file the
    /// project does not have. Neither is visible to a golden diff, which shows
    /// what the README says and never what the emitter does — which is why this
    /// reads the table back rather than trusting a reviewer to.
    ///
    /// Run under **both** shapes of the table, because two of its rows are the
    /// target's rather than the layout's (grammar §14.6): a build with no
    /// `package_registry:` writes no installer configuration and must document
    /// none, and a build with one must document both files it wrote.
    #[test]
    fn the_readme_documents_every_emitted_file() {
        for ir in [
            ir_of("version: \"0.1\"\n"),
            ir_of_mesh(
                "version: \"0.1\"\n",
                "version: \"0.1\"\npackage_registry:\n  url: \"https://npm.internal.example/mirror/\"\n",
            ),
        ] {
            let contents = readme_of(&ir).contents;
            let below = contents
                .split_once("## Layout")
                .expect("the layout table has a heading")
                .1;
            let table = below.split_once("\n## ").map_or(below, |(above, _)| above);
            let listed: std::collections::BTreeSet<&str> = table
                .lines()
                .filter_map(|line| line.strip_prefix("| `"))
                .filter_map(|row| row.split_once('`'))
                .map(|(path, _)| path)
                .collect();
            let emitted: std::collections::BTreeSet<&str> =
                crate::codegen::emitted_paths(&ir).into_iter().collect();
            assert_eq!(
                listed, emitted,
                "the README's layout table and the file list `build` replaces disagree"
            );
        }
    }

    /// A target with a mirror gets the section about it; one without gets
    /// nothing — and the section says the thing a reader cannot guess, which is
    /// that an unset variable is not an install-time error.
    #[test]
    fn a_target_with_a_private_registry_is_told_what_an_unset_variable_costs() {
        let plain = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            !plain.contains("## Installing through a private registry"),
            "a project with no mirror is not told about one"
        );

        let contents = readme_of(&ir_of_mesh(
            "version: \"0.1\"\n",
            r#"version: "0.1"

package_registry:
  url: "https://npm.internal.example/repository/npm-group/"
  token: ${NPM_MIRROR_TOKEN}
  scopes:
    "@corp":
      url: "https://npm.internal.example/repository/corp/"
"#,
        ))
        .contents;
        assert!(
            contents.contains("## Installing through a private registry"),
            "{contents}"
        );
        assert!(
            contents.contains("https://npm.internal.example/repository/npm-group/"),
            "{contents}"
        );
        assert!(
            contents.contains("| `@corp` | `https://npm.internal.example/repository/corp/` |"),
            "a scope that resolves elsewhere is named: {contents}"
        );
        assert!(contents.contains("`NPM_MIRROR_TOKEN`"), "{contents}");
        assert!(
            contents.contains("**An unset variable is not an install-time error.**"),
            "{contents}"
        );
    }

    /// The two pin tables and the README's table are one list. A dependency
    /// added to the manifest and not to the README would be a version a reader
    /// of the project could not find.
    #[test]
    fn the_readme_documents_every_pin() {
        let contents = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        for (package, version) in PINS.iter().chain(DEV_PINS) {
            assert!(
                contents.contains(&format!("| `{package}` | `{version}` |")),
                "the README does not document `{package}`"
            );
        }
    }

    /// **Every project is told where its journal lives, and which one it is**
    /// (grammar §14.7, PRD resolved q62).
    ///
    /// Emitted unconditionally, unlike the sections around it, because
    /// durability is unconditional (Decision D121): "where does this project's
    /// record survive a restart" always has an answer, and a reader should not
    /// have to know what the default is to find it.
    ///
    /// The half that is easy to get wrong is the address. A remote journal's is
    /// a credential, and `docs/trace.md` §11.1 keeps a resolved `${ENV}` out of
    /// every artifact this project writes — a README included — so what a reader
    /// meets is the variable's **name**. A README that printed the value would
    /// put a database password in a file every worker unpacks.
    #[test]
    fn every_project_is_told_which_journal_its_target_bound() {
        let local = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            local.contains("## Where this project's journal lives"),
            "a project is not told where its journal is: {local}"
        );
        assert!(
            local.contains(".agent-compose/journal.sqlite"),
            "a target that bound the default is not told it is a file beside the project"
        );
        for package in ["`pg`", "`mysql2`", "`@types/pg`"] {
            assert!(
                !local.contains(package),
                "the zero-infra build's README names {package}, which its `package.json` does \
                 not pin and its `src/journal.ts` does not import"
            );
        }

        let remote = readme_of(&crate::codegen::test_support::ir_of_mesh(
            "version: \"0.1\"\n",
            "version: \"0.1\"\njournal:\n  provider: postgres\n  url: ${JOURNAL_URL}\n",
        ))
        .contents;
        assert!(
            remote.contains("binds a **`postgres`** journal"),
            "a target that bound a remote journal is not told which: {remote}"
        );
        assert!(
            remote.contains("`${JOURNAL_URL}`") && remote.contains("| `pg` | "),
            "the README names neither the variable holding the address nor the driver the \
             manifest pins for it: {remote}"
        );
        assert!(
            !remote.contains(".agent-compose/journal.sqlite"),
            "a target that dials a server is told its record is in a file beside the project"
        );
        assert!(
            remote.contains("One process at a time writes this journal"),
            "the README does not state the one-writer rule the guard is under \
             (`docs/durability.md` §2)"
        );
    }

    /// A composition with no grammar 6.1 binding gets no host-function section,
    /// and one with a binding gets the registration it cannot run without —
    /// under the two-argument signature `src/runtime.ts`'s `HostFunction`
    /// actually declares, because the second argument is where the node's
    /// deadline is.
    #[test]
    fn a_composition_with_a_function_binding_is_told_what_registering_one_costs() {
        let plain = readme_of(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            !plain.contains("## Host functions"),
            "a composition using no escape hatch is not told about one"
        );

        let contents = readme_of(&ir_of(
            r#"version: "0.1"

tool.rank:
  description: Rank the candidates by the host's own rule.
  input:
    text: { type: string }
  output:
    ranked: { type: string }
  function:
    name: rank_candidates
"#,
        ))
        .contents;
        assert!(contents.contains("## Host functions"), "{contents}");
        assert!(
            contents.contains("registerFunction(\"rank_candidates\", async (args, context) => {"),
            "the snippet takes the `RunContext` the registry passes: {contents}"
        );
        assert!(
            contents.contains("the implementation of `tool.rank`"),
            "{contents}"
        );
        // What racing the deadline (`runActivity`) does and does not promise. A
        // host reading only the signature would take the signal for the whole
        // mechanism, which is the reading this paragraph exists to refuse.
        assert!(
            contents.contains("### What a node's `timeout:` means here"),
            "{contents}"
        );
        assert!(
            contents.contains("the runtime races the deadline against the call"),
            "{contents}"
        );
        assert!(
            contents.contains("keeps running after the node it belonged to has failed"),
            "{contents}"
        );
        // The other thing on that second argument a host cannot learn from the
        // signature: grammar 9.4's delivery surface for a `function:`-bound
        // target. A sink that is delivered at-least-once and told nothing about
        // it has no way to dedupe, which is the whole of what the key is for.
        assert!(
            contents.contains("### When `context.idempotency_key` is set"),
            "{contents}"
        );
        assert!(
            contents.contains("stable across every attempt of the same item"),
            "{contents}"
        );
        assert!(
            contents.contains("**absent on every other call**"),
            "{contents}"
        );
        // …and the exception the paragraph above would otherwise state wrongly.
        // A detached delivery runs on a signal nothing aborts (`runMap`), so a
        // sink written to the timeout section's advice — return early when
        // `signal.aborted` — would be dead code in the one implementation where
        // a call not made is a message lost.
        assert!(
            contents.contains("on every call but one, and that exception is the"),
            "{contents}"
        );
        assert!(
            contents.contains("`context.signal` is not the map node's"),
            "{contents}"
        );
    }

    #[test]
    fn the_tsconfig_is_strict_and_emits_nothing() {
        let contents = tsconfig_json(&ir_of("version: \"0.1\"\n")).contents;
        assert!(contents.contains("\"strict\": true"));
        assert!(contents.contains("\"noEmit\": true"));
        assert!(contents.contains("\"include\": [\"src/**/*.ts\"]"));
    }
}
