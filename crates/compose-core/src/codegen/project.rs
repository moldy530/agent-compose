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
    contents.push_str(&dependency_block("dependencies", PINS, true));
    contents.push_str(&dependency_block("devDependencies", DEV_PINS, false));
    contents.push_str("}\n");

    super::GeneratedFile {
        path: "package.json".to_string(),
        contents,
    }
}

fn dependency_block(key: &str, pins: &[(&str, &str)], trailing_comma: bool) -> String {
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
#[must_use]
pub fn readme(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "> ");
    contents.push('\n');
    contents.push_str(&format!(
        "# `{PACKAGE_NAME}`\n\n\
         The LangGraph TypeScript project `agent-compose build` produced from `{}`, \
         resolved for the `{}` target.\n",
        ir.entrypoint, ir.target
    ));
    contents.push_str(README_BODY);
    contents.push_str(&host_functions(ir));

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

const README_BODY: &str = r#"
## Layout

| path | what it holds |
|---|---|
| `src/cel.ts` | the CEL evaluator the routers embed (PRD 5.5) |
| `src/env.ts` | every `${ENV}` reference the composition makes, and `readEnvironment()`, the presence check over them |
| `src/runtime.ts` | what every node does when it runs: the retry/timeout/error policy of grammar 9, the provider surfaces, the `exec`/`http` wrappers, and the router |
| `src/schemas.ts` | every schema the composition declares, as Zod |
| `src/state.ts` | the graph's state model: one channel per `state:` channel, the implicit conversation history, and `$run` — what the runtime keeps beside them |
| `src/graph.ts` | the compiled graph: one node per flow node, the `flows` registry, and `runFlow` |
| `src/index.ts` | the project's public surface, and the one caller of `readEnvironment()` |

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

A run that produces no answer throws a `FlowFailure`, and it carries that same
record: `.trace` holds every step that completed plus one final entry for the
node the run stopped at, and `.cause` is the error itself. Both ways a run can
fail raise it — one that never reached quiescence, and one that reached
quiescence holding no value for a field its `outputs:` declares — so `.trace` is
readable without asking which happened.

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

`src/` is owned by the compiler: `agent-compose build` replaces the modules it
emits, removes the ones it no longer emits, and `agent-compose build --check`
reports either as drift. `package.json`, `tsconfig.json`, `.gitignore` and this
README are generated too, and a rebuild replaces them. Everything else in this
directory — `node_modules/`, a lockfile, a `.env` — is yours and is never
removed.

Every file the compiler replaces or removes carries the header above, which is
how it tells its own work from yours: a `build` into a directory holding none of
its files refuses rather than overwriting what is there.

## Running it

There is no build step: the TypeScript in `src/` is what runs. Bun is the
default — it is the runtime and the installer this project is documented,
tested and gated against:

```sh
bun install          # installs the pinned dependency set
bun run typecheck    # tsc --noEmit, the type gate
bun src/index.ts
```

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
# that buys (PRD §8). These two are the exceptions: one is an install artifact,
# and the other is the thing the spec deliberately never contains.
node_modules/
.env
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
// The project's public surface. Everything a consumer of this graph needs —
// the schemas, the state model, the graph itself, `runFlow`, and the
// environment it requires — is re-exported here, so an ejected project has one
// entry point and `run`/`serve` have one module to import.
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

import { readEnvironment } from "./env.ts";

export * from "./env.ts";
export * from "./graph.ts";
export * from "./schemas.ts";
export * from "./state.ts";

readEnvironment();
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    fn manifest() -> String {
        package_json(&ir_of("version: \"0.1\"\n")).contents
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
        let contents = readme(&ir_of("version: \"0.1\"\n")).contents;

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

    /// The two pin tables and the README's table are one list. A dependency
    /// added to the manifest and not to the README would be a version a reader
    /// of the project could not find.
    #[test]
    fn the_readme_documents_every_pin() {
        let contents = readme(&ir_of("version: \"0.1\"\n")).contents;
        for (package, version) in PINS.iter().chain(DEV_PINS) {
            assert!(
                contents.contains(&format!("| `{package}` | `{version}` |")),
                "the README does not document `{package}`"
            );
        }
    }

    /// A composition with no grammar 6.1 binding gets no host-function section,
    /// and one with a binding gets the registration it cannot run without —
    /// under the two-argument signature `src/runtime.ts`'s `HostFunction`
    /// actually declares, because the second argument is where the node's
    /// deadline is.
    #[test]
    fn a_composition_with_a_function_binding_is_told_what_registering_one_costs() {
        let plain = readme(&ir_of("version: \"0.1\"\n")).contents;
        assert!(
            !plain.contains("## Host functions"),
            "a composition using no escape hatch is not told about one"
        );

        let contents = readme(&ir_of(
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
