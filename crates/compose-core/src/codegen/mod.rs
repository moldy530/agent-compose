//! Codegen: the flat IR in, a TypeScript project out (PRD 5.12, §7 M1).
//!
//! # The shape of the pass
//!
//! [`emit`] is a **pure function**. It takes an [`Ir`] and the bytes of the
//! authored files that IR references ([`authored::Authored`]), answers a
//! [`GeneratedProject`] — a set of `(path, contents)` pairs — and it touches no
//! filesystem, reads no environment, and consults no clock. The authored bytes
//! are an *argument* precisely so that stays true: PRD resolved q49 puts files
//! the compiler did not write inside the artifact it describes, and a pass that
//! went and read them would be a pass that could answer differently twice. The
//! CLI is what writes the bytes ([`agent-compose build`]), and `build --check`
//! is the same function run against what is already on disk. Three things
//! follow, and each is a requirement rather than a convenience:
//!
//! * **Determinism is structural.** PRD 5.12 asks for byte-identical output from
//!   byte-identical input. A pass that cannot observe anything but its argument
//!   cannot be non-deterministic for any reason but its own ordering, and the
//!   ordering rules below are the whole of that surface.
//! * **`--check` is not a second implementation.** The command that verifies a
//!   committed project is up to date runs the emitter and compares bytes, so
//!   there is no drift between what `build` writes and what `build --check`
//!   expects.
//! * **The emitter is testable without a directory.** The golden corpus
//!   (`tests/generated_project_goldens.rs`) compares `emit`'s answer against
//!   committed files; nothing has to be written to compare it.
//!
//! # Ordering — where determinism actually comes from
//!
//! The IR is already canonical about everything that could come from file layout
//! (see [`crate::ir`]): definitions are a `BTreeMap` keyed by address, sections
//! are `BTreeMap`s keyed by name, and everything below that level keeps the
//! order it was written in. The emitter inherits that and adds one rule of its
//! own:
//!
//! > Every list this module builds is either sorted by a key it derives from the
//! > IR, or is in the IR's own order. Nothing is ordered by a `HashMap`, by
//! > insertion into a set, or by anything a future refactor could reorder for
//! > free.
//!
//! [`GeneratedProject::files`] is sorted by path. Schema declarations inside
//! `src/schemas.ts` are in **surface order** ([`schema::surfaces`]), which walks
//! `definitions` in address order and each flow's nodes in declaration order —
//! the order the author wrote, under a canonical roof.
//!
//! # The emitted project
//!
//! ```text
//! package.json          the pinned dependency set (see `project::pins`)
//! tsconfig.json         strict, NodeNext, no build step
//! README.md             what this directory is, how to run it, how to eject
//! .gitignore            the two paths a generated project acquires
//! manifest.json         what a worker reads out of the tree (distributed §9.1)
//! src/artifact.ts       this tree's content hash and file list (distributed §4)
//! src/cel.ts            the CEL evaluator the routers embed (PRD 5.5)
//! src/deployment.ts     the placements, and the env partition (distributed §9.1)
//! src/env.ts            the hub's `${ENV}` references, and the launch check
//! src/mesh.ts           the hub half of the worker protocol (distributed §3)
//! src/modules.ts        the `module:` bindings' contracts, and the one seam
//! src/runtime.ts        what a node does when it runs (grammar 8, 9)
//! src/stores.ts         the local store backends (PRD 5.8, grammar 11)
//! src/schemas.ts        every schema in the composition, as Zod (grammar 3.8)
//! src/state.ts          the LangGraph state model (grammar 10)
//! src/graph.ts          the compiled graph, and `runFlow`
//! src/triggers.ts       the declared `http` triggers (grammar 13.3)
//! src/serve.ts          the generated app over them (PRD 5.11)
//! src/cli.ts            the project's own `run`/`serve` command line
//! src/index.ts          the project's public surface, and its entry point
//! src/worker-node.ts    one placed node, run on a worker (distributed §3.2)
//! ```
//!
//! Seven of those are **constants**: `src/cel.ts`, `src/mesh.ts`,
//! `src/runtime.ts`, `src/stores.ts`, `src/serve.ts`, `src/cli.ts` and
//! `src/worker-node.ts` are byte-identical in every project a compiler release
//! builds, which is what keeps a golden diff about the composition rather than
//! about the machinery beside it. The rest are the composition, lowered.
//!
//! `src/harness.ts` is the one that is *nearly* a constant and deliberately is
//! not (PRD resolved q57): it carries a driver per harness some `coder:` node
//! binds and nothing for the ones none does, so a project with no coder node
//! imports no harness SDK — which is what keeps `package.json`'s pins and this
//! module's imports the same set (see [`harness`]).
//!
//! `src/artifact.ts` is emitted **last and over the rest**, because what it
//! carries is a hash of them (`docs/distributed.md` §4, and see [`artifact`]).
//!
//! **The manifest is the boundary** (PRD resolved q47). [`EMITTED_PATHS`] is
//! this list and is the compiler's whole claim on an output directory: `build`
//! overwrites exactly it, `build --check` compares exactly it, and nothing else
//! under the output directory is written, removed, or reported — not a
//! `node_modules/`, not a `.env`, and not the hand-authored TypeScript a
//! `module:` binding names (grammar 6.1). A file this compiler does not emit is
//! not this compiler's, wherever it sits; `src/tools/` ([`AUTHORED_ZONE`]) is
//! where the scaffold puts an authored implementation and where the docs teach
//! it, but that is a convention and the manifest is the answer.
//!
//! # The tree is wider than the emission set
//!
//! PRD resolved q49: **the artifact carries what the spec references.** A
//! module-bound tool executes wherever its tool executes — on a worker, when
//! placed or reached through attachment — and a worker holds nothing but the
//! artifact, so the file the binding names has to be *in* the tree the hub
//! serves. [`GeneratedProject`] therefore holds two lists:
//!
//! * [`GeneratedProject::files`] — the emission set, [`EMITTED_PATHS`] exactly.
//!   Generated, header-carrying, byte-identical for byte-identical input.
//! * [`GeneratedProject::carried`] — the authored files the composition
//!   references, at the same project-relative path they are edited at. Their
//!   bytes are the author's; the compiler copies them into the tree and never
//!   reads them for anything else.
//!
//! [`GeneratedProject::artifact`] is the union, sorted, and it is what
//! `ARTIFACT_FILES` lists and what `ARTIFACT_HASH` covers — so editing a tool
//! implementation is a new artifact, and every worker is handed it through the
//! join handshake with no new machinery (`docs/distributed.md` §4). A file under
//! `src/` that nothing references ships nowhere.
//!
//! There are exactly **two** notions of file identity and this widens the second
//! rather than adding a third: `build --check` compares the tree the build
//! produced — both lists — and the artifact hash covers the tree it serves,
//! which is the same tree. What the author edits is the project-side original;
//! the copy in the output directory is a build artifact like everything else
//! beside it, and a stale one is drift.
//!
//! What that costs, stated so it is not discovered: a module an older compiler
//! release wrote and this one no longer emits is **left where it is**. It is
//! not drift, because drift is a disagreement about a file the compiler claims,
//! and it does not claim that one any more. The alternative — a walk of `src/`
//! that deletes what carries a generated-file header — is what made `build` a
//! read of its own previous output rather than a pure function of the spec, and
//! it is exactly the machinery resolved q47 removes so that authored code can
//! live in the same tree without a marker protocol guarding it.
//!
//! ## Why there is no build step
//!
//! The emitted project is run directly from source: `bun src/index.ts` under the
//! default runtime (PRD §9.18), `node src/index.ts` under the fallback. Bun runs
//! TypeScript natively and Node has stripped types since 22.18, so a `.ts` file
//! is a runnable module on both, and TypeScript is a **checker** here rather than
//! a compiler. Consequences, all of them deliberate:
//!
//! * relative imports are written with their real `.ts` extension, which is what
//!   both runtimes resolve and what `allowImportingTsExtensions` makes legal;
//! * `agent-compose run` and `serve` spawn the **default runtime** on the
//!   sources — the same `bun src/index.ts` the emitted `README.md` documents
//!   first — so there is no stale-`dist/` failure mode and no second artifact to
//!   keep in step;
//! * `tsc --noEmit` is the type gate and nothing else, so a project that fails to
//!   type-check still fails loudly in CI while never being on a run's critical
//!   path;
//! * ejecting (PRD 5.12) is copying the directory: it is already a plain
//!   TypeScript project with no toolchain of ours in it, and no module in it
//!   reaches for an API only one runtime has (`generated_code_gates.rs` gate 14).
//!
//! # Generated-file headers
//!
//! Every emitted file opens with [`header`] — the tool, its version, the spec
//! source it was built from, and the target. PRD §8 lists "hand-edited generated
//! code forks the source of truth" as a risk whose mitigation is exactly this
//! plus `build --check` in CI. The version is in the header on purpose: a
//! compiler release pins a LangGraph version (PRD 5.12), so a release *should*
//! rewrite every generated file, and a golden diff that says so is the review
//! surface that pinning is supposed to have.
//!
//! # What is not here yet
//!
//! The `human` node runtime, which PRD §9's resolved question 4 schedules for
//! M2. A composition using one still **builds**, and its topology is still
//! emitted in full — the edges, the budgets, the node's place in the graph —
//! with an activity that throws naming the construct and the milestone that
//! lands it ([`graph`]). Nothing answers a plausible value.
//!
//! The same posture covers the one place the deploy layer still reaches past
//! this release: a store bound to a production backend (grammar 14.3's `redis`,
//! `pgvector`, `s3`, …) says so at the op rather than answering out of the wrong
//! store.
//!
//! `hub:`/`placements:` are **not** on that list any more. This pass emits the
//! hub `docs/distributed.md` fixes: [`mesh`] is the five `/workers/*` routes and
//! the dispatch board behind them, [`deployment`] the placements and the
//! environment partition of §9.1, [`artifact`] the tree's content hash and file
//! list, and [`graph`] lowers a placed component's node to a dispatch and its
//! activity into the registry `src/worker-node.ts` runs on the other side.
//! `tests/placement_surface_landing.rs` is what says so in executable form.

pub mod artifact;
pub mod authored;
pub mod cel;
pub mod cli;
pub mod delivery;
pub mod deployment;
pub mod env;
pub mod graph;
pub mod harness;
pub mod journal;
pub mod mesh;
pub mod modules;
pub mod names;
pub mod otlp;
pub mod pattern;
pub mod policy;
pub mod project;
pub mod runtime;
pub mod schema;
pub mod serve;
pub mod state;
pub mod stores;
pub mod trigger;
pub mod worker;

use crate::diag::{Diagnostic, DiagnosticCode};
use crate::ir::Ir;
use crate::ir::schema::TypeForm;

/// The compiler release this build is, as it appears in every generated header.
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Every path [`emit`] writes, sorted — the compiler's whole claim on an output
/// directory (PRD resolved q47).
///
/// The list is a **constant** rather than a walk because it is the same for
/// every composition: the layout is fixed, and what varies is the contents of
/// each file rather than which files there are.
/// [`the_file_set_is_the_documented_layout_for_every_composition`] is what holds
/// that, over two compositions with nothing in common.
///
/// It is a constant here so that the *front end* can read it: a `module:`
/// binding naming a path this list carries is refused where it is written
/// (grammar 6.1), rather than at a build that would have to choose between
/// overwriting the author's file and refusing to emit its own.
///
/// [`the_file_set_is_the_documented_layout_for_every_composition`]: tests::the_file_set_is_the_documented_layout_for_every_composition
pub const EMITTED_PATHS: &[&str] = &[
    ".gitignore",
    "README.md",
    "manifest.json",
    "package.json",
    "src/artifact.ts",
    "src/cel.ts",
    "src/cli.ts",
    "src/delivery.ts",
    "src/deployment.ts",
    "src/env.ts",
    "src/graph.ts",
    "src/harness.ts",
    "src/index.ts",
    "src/journal.ts",
    "src/mesh.ts",
    "src/modules.ts",
    "src/otlp.ts",
    "src/runtime.ts",
    "src/schemas.ts",
    "src/serve.ts",
    "src/state.ts",
    "src/stores.ts",
    "src/triggers.ts",
    "src/worker-node.ts",
    "tsconfig.json",
];

/// The directory a scaffolded module implementation is conventionally written
/// under (PRD resolved q47).
///
/// A **convention**, not a rule: ownership of a file is membership in
/// [`EMITTED_PATHS`], so a `module:` binding may name any project-relative path
/// that list does not carry. This is where the docs point and where an author
/// who follows them ends up, which is what makes `src/tools/` worth naming once
/// rather than leaving each reader to invent.
pub const AUTHORED_ZONE: &str = "src/tools";

/// One file of a generated project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedFile {
    /// Where it goes, relative to the output directory, `/`-separated.
    pub path: String,
    /// Its bytes, as text. Every file this compiler emits is UTF-8 and ends in a
    /// newline.
    pub contents: String,
}

/// A whole generated project: what [`emit`] answers.
///
/// Two lists, both sorted by path, and between them the whole tree a build
/// produces — there is no "and also copy these" step anywhere. A consumer
/// writes them, compares them, or hashes them; nothing else is needed to have
/// the project. See *The tree is wider than the emission set* above for why the
/// second list exists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedProject {
    files: Vec<GeneratedFile>,
    carried: Vec<GeneratedFile>,
}

impl GeneratedProject {
    /// Build a project from its emitted files and the authored ones it carries,
    /// sorting both by path.
    ///
    /// # Panics
    ///
    /// Panics when two files claim one path. That is an emitter bug rather than
    /// a composition error — every emitted path is derived from a fixed layout,
    /// and a `module:` binding may not name one of them (grammar 6.1) — and a
    /// silent last-one-wins would ship a project missing a module.
    fn new(mut files: Vec<GeneratedFile>, mut carried: Vec<GeneratedFile>) -> Self {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        carried.sort_by(|left, right| left.path.cmp(&right.path));
        let mut every: Vec<&str> = files
            .iter()
            .chain(&carried)
            .map(|file| file.path.as_str())
            .collect();
        every.sort_unstable();
        for pair in every.windows(2) {
            assert!(pair[0] != pair[1], "two files claim `{}`", pair[0]);
        }
        Self { files, carried }
    }

    /// Every **emitted** file, sorted by path — [`EMITTED_PATHS`] exactly.
    #[must_use]
    pub fn files(&self) -> &[GeneratedFile] {
        &self.files
    }

    /// Every **authored** file this tree carries, sorted by path.
    ///
    /// The bytes are the author's, read from the project the composition was
    /// resolved out of and written into the output directory unchanged (PRD
    /// resolved q49).
    #[must_use]
    pub fn carried(&self) -> &[GeneratedFile] {
        &self.carried
    }

    /// Every file of the tree — emitted and carried — sorted by path.
    ///
    /// The artifact: what `ARTIFACT_FILES` lists, what `ARTIFACT_HASH` covers,
    /// and what a build writes.
    pub fn artifact(&self) -> impl Iterator<Item = &GeneratedFile> {
        Merged {
            left: self.files.iter().peekable(),
            right: self.carried.iter().peekable(),
        }
    }

    /// The **emitted** file at this path, if the project has one.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&GeneratedFile> {
        self.files.iter().find(|file| file.path == path)
    }

    /// Every path of the tree, emitted and carried, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.artifact().map(|file| file.path.as_str())
    }
}

/// Two path-sorted runs of files, read as one path-sorted run.
///
/// A merge rather than a concatenate-and-sort so that [`GeneratedProject`] can
/// answer its whole tree by reference — the caller of
/// [`artifact`](GeneratedProject::artifact) is usually hashing or writing, and
/// neither wants the clone a rebuilt vector would cost.
struct Merged<'a, L: Iterator<Item = &'a GeneratedFile>, R: Iterator<Item = &'a GeneratedFile>> {
    left: std::iter::Peekable<L>,
    right: std::iter::Peekable<R>,
}

impl<'a, L: Iterator<Item = &'a GeneratedFile>, R: Iterator<Item = &'a GeneratedFile>> Iterator
    for Merged<'a, L, R>
{
    type Item = &'a GeneratedFile;

    fn next(&mut self) -> Option<Self::Item> {
        match (self.left.peek(), self.right.peek()) {
            (Some(left), Some(right)) if right.path < left.path => self.right.next(),
            (Some(_), _) => self.left.next(),
            (None, _) => self.right.next(),
        }
    }
}

/// Every name the emitted project's one module namespace holds.
///
/// Built in one order, in one place, because two passes over the same IR have to
/// answer the same names: [`emit`] writes `src/modules.ts` and `src/graph.ts`
/// from it, and [`authored::scaffolds`] writes a stub that imports what
/// `src/modules.ts` exports. A registry built differently on either side would
/// scaffold a file importing a type nothing exports.
fn registry(ir: &Ir) -> names::Names {
    let mut names = names::Names::of(ir);
    graph::declare(&mut names, ir);
    modules::declare(&mut names, ir);
    names
}

/// Lower one resolved composition to a TypeScript project.
///
/// The IR must have passed [`crate::check`]: this pass reports nothing and
/// refuses nothing, because everything it could refuse the validator has already
/// refused with a span to point at. `build` runs the validator first and emits
/// only on a clean report (PRD §7 M1).
///
/// `authored` is the bytes of every file a `module:` binding names, which the
/// caller reads (see [`authored::Authored`]): this pass is pure, and PRD
/// resolved q49 puts files it did not write inside the artifact it describes. A
/// composition with no binding is emitted with [`authored::Authored::none`].
///
/// # Panics
///
/// Panics when `authored` is missing a file the composition references. Every
/// caller builds it from the same enumeration this one reads, so a gap is a
/// caller bug — and an artifact whose hash silently omitted an entry would be
/// served under a name a worker's own verification then refuses.
#[must_use]
pub fn emit(ir: &Ir, authored: &authored::Authored) -> GeneratedProject {
    let names = registry(ir);
    // The environment `readEnvironment()` checks is the **hub's own list**, which
    // is the whole composition's exactly when nothing is placed
    // (`docs/distributed.md` §9.1): a variable a placement takes off it is one
    // this process cannot leak, because it never held it.
    let partition = env::Partition::of(ir);
    let environment = env::References::for_process(ir, &partition, &env::Process::Hub);

    let mut files = vec![
        project::package_json(ir),
        project::tsconfig_json(ir),
        project::readme(ir, &partition),
        project::gitignore(ir),
        cel::module(ir),
        delivery::module(ir),
        deployment::module(ir, &partition),
        env::module(ir, &environment),
        journal::module(ir),
        mesh::module(ir),
        modules::module(ir, &names),
        otlp::module(ir),
        runtime::module(ir),
        stores::module(ir),
        schema::module(ir, &names),
        state::module(ir, &names),
        graph::module(ir, &names),
        harness::module(ir),
        trigger::module(ir),
        serve::module(ir),
        cli::module(ir),
        worker::module(ir),
        worker::manifest(ir, &partition),
        project::index(ir),
    ];
    // The authored half of the tree, at the same project-relative path it is
    // edited at (PRD resolved q49). It goes in before the artifact module,
    // because the hash is over the whole tree and a module tool that did not
    // move it would let an edited implementation reach a worker under the hash
    // of the tree that did not have it.
    let carried: Vec<GeneratedFile> = crate::check::modules::bindings(ir)
        .into_iter()
        .map(|(address, module)| {
            let path = module.path.value.as_str();
            GeneratedFile {
                path: path.to_string(),
                contents: authored
                    .get(path)
                    .unwrap_or_else(|| {
                        panic!("`{address}` is implemented by `{path}`, which was not read")
                    })
                    .to_string(),
            }
        })
        .collect();

    // **Last, and over everything above.** The artifact's identity is a hash of
    // the tree, so the file that carries it is the one file the hash cannot
    // cover and the one file that has to be written after the rest exists (see
    // [`artifact`]).
    files.push(artifact::module(ir, &files, &carried));
    GeneratedProject::new(files, carried)
}

/// What this target cannot express, over a composition the validator accepted.
///
/// [`emit`] is total: it answers a project for every artifact, and it has no
/// span to report against anyway. But not every legal composition *has* a
/// TypeScript project. Two things it cannot hold:
///
/// * **A `pattern:` no JavaScript regular expression can hold.** `pattern:` is
///   RE2 (Decision D12) and RE2 is not a subset of ECMAScript (see [`pattern`]).
///   Emitting it anyway produces a `src/schemas.ts` that fails to parse: not a
///   wrong schema, an unloadable module, and `build` exiting `0` over it.
/// * **A `matches()` pattern in the same position.** Grammar 4.1 puts CEL's
///   standard `matches` on the expression surface, and the CEL specification
///   defines it over RE2 — the *same* second language `pattern:` is written in,
///   reached through an expression instead of a schema. The emitted evaluator has
///   only `new RegExp(…)`, so the mismatch runs both ways and both are reachable:
///   `state.a.matches('(?i)urgent')` validates and dies at the guard with
///   `SyntaxError: Invalid group`, and `state.a.matches('a(?=b)')` runs happily in
///   a compiled router while the Rust column refuses to compile it at all. Two
///   evaluators answering differently is the exact drift PRD 5.5's corpus exists
///   to prevent, so the target refuses the pattern rather than shipping it — and
///   refuses a **computed** one too, because a pattern the compiler cannot read is
///   one it cannot make that promise about (see [`cel::matches_patterns`]).
/// * **A name every JavaScript object already answers to.** `constructor` is a
///   legal grammar 2.1 identifier and grammar 2.5 reserves only the seven roots,
///   so the validator accepts it wherever an identifier goes — and three of
///   those positions become a **key read off a plain object** in the emitted
///   project (see [`state::INHERITED_PROPERTY_NAMES`]):
///   * a **state channel** name, which `StateGraph` looks up in its channel
///     table before installing the channel, so the graph cannot be constructed;
///   * a **schema property** name, which `z.object({…})` reads off the value it
///     is parsing, so an omitted property arrives holding `Object` and the parse
///     refuses a document the published JSON Schema accepts;
///   * a **`discriminator`** name, which `z.discriminatedUnion` indexes its
///     variants by, so the parse throws (`propValues[key].add is not a
///     function`) rather than answering.
///
///   All three type-check, and the first two also load; each fails at a point no
///   gate a build has would reach.
///
/// So this is the pass between the validator and the emitter. It is the same
/// shape as the validator — an [`Ir`] in, [`Diagnostic`]s out, spans included —
/// and `build` folds its report into the one the validator produced, refusing to
/// emit when either has anything to say. It is **not** a static check:
/// `validate` answers "is this composition well formed", which does not depend
/// on which target it is later compiled for, and this answers "can *this* target
/// express it".
///
/// **One declaration, one diagnostic.** A type node is reachable from more than
/// one surface — a store's `value_schema` is a schema in its own right *and* is
/// embedded in the row every `get` on that store derives (grammar 11.4), so a
/// composition with three such nodes reaches one `pattern:` four times. The
/// subject of the message is the declaration, and repeating it once per use
/// would be three copies of one problem with one span (PRD G3).
#[must_use]
pub fn diagnostics(ir: &Ir) -> Vec<Diagnostic> {
    let mut found = crate::diag::Diagnostics::new();
    for (_, channel) in schema::channels(ir) {
        let name = channel.name.value.as_str();
        if !state::inherited_property_name(name) {
            continue;
        }
        found.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                channel.name.span.clone(),
                format!(
                    "a state channel named `{name}` is one this target cannot hold: LangGraph \
                     keeps its channels in a plain object and asks it for `{name}` before \
                     installing the channel, which every JavaScript object answers from \
                     `Object.prototype` — so the graph fails to construct before a node runs"
                ),
            )
            .with_help(format!(
                "rename the channel — grammar 10.3 wires a write by the channel's own name, so \
                 `{name}` is the key the emitted state model has to use and there is no second \
                 spelling of it"
            )),
        );
    }

    // The same name, one level down: every key of an emitted object shape. The
    // dedup is by span rather than by name, because a store's `value_schema` is
    // a surface in its own right *and* is embedded in the row every `get` on it
    // derives (grammar 11.4) — one declaration, one diagnostic.
    let mut keys: std::collections::BTreeSet<(String, usize, usize)> =
        std::collections::BTreeSet::new();
    for surface in schema::surfaces(ir) {
        surface.walk_keys(&mut |key, kind| {
            let name = key.value.as_str();
            if !state::inherited_property_name(name) {
                return;
            }
            if !keys.insert((
                key.span.source.as_str().to_string(),
                key.span.bytes.start,
                key.span.bytes.end,
            )) {
                return;
            }
            found.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    key.span.clone(),
                    format!(
                        "a {} named `{name}` is one this target cannot hold: the emitted schema \
                         reads `{name}` off the value it is parsing with a plain property lookup, \
                         which every JavaScript object answers from `Object.prototype` — so {}",
                        kind.as_str(),
                        match kind {
                            schema::KeyKind::Property =>
                                "a document that leaves it out is read as carrying a function \
                                 there, and the parse refuses what the published JSON Schema \
                                 accepts",
                            schema::KeyKind::Discriminator =>
                                "the union indexes its variants by a function and the parse throws \
                                 instead of answering",
                        }
                    ),
                )
                .with_help(format!(
                    "rename it — grammar 3.8 lowers this schema to an object keyed by the names it \
                     declares, so `{name}` is the key the emitted parse has to use and there is no \
                     second spelling of it"
                )),
            );
        });
    }

    let mut reported: std::collections::BTreeSet<(String, usize, usize, String)> =
        std::collections::BTreeSet::new();
    for surface in schema::surfaces(ir) {
        surface.walk(&mut |ty| {
            let TypeForm::Scalar(scalar) = &ty.form else {
                return;
            };
            let Some(source) = &scalar.pattern else {
                return;
            };
            // The span identifies the declaration: a synthesized copy of a
            // field map keeps the spans of the text it was copied from.
            let declaration = (
                ty.span.source.as_str().to_string(),
                ty.span.bytes.start,
                ty.span.bytes.end,
                source.clone(),
            );
            if !reported.insert(declaration) {
                return;
            }
            if let Err(unsupported) = pattern::javascript(source) {
                found.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        ty.span.clone(),
                        format!("{} (`{source}`)", unsupported.message),
                    )
                    .with_help(unsupported.help),
                );
            }
        });
    }

    // The same question, asked of the regular expressions an *expression*
    // carries. One diagnostic per `matches()` call rather than per expression:
    // two bad patterns in one guard are two things to fix.
    for expression in cel::expressions(ir) {
        let site = &expression.site;
        for found_pattern in crate::cel::matches_patterns(expression.source.value.as_str()) {
            let source = match found_pattern {
                crate::cel::MatchesPattern::Computed => {
                    found.push(
                        Diagnostic::error(
                            DiagnosticCode::InvalidValue,
                            expression.source.span.clone(),
                            format!(
                                "{site} calls `matches()` with a pattern this target cannot read: \
                                 the argument is computed rather than written down, and a regular \
                                 expression this compiler never sees is one it cannot check \
                                 against either engine"
                            ),
                        )
                        .with_help(
                            "write the pattern as a string literal — `matches('^a[0-9]$')` — so \
                             the compiler can decide it: CEL defines `matches` over RE2 and the \
                             emitted evaluator has JavaScript's `RegExp`, and the two are not the \
                             same language (grammar 4.1, Decision D12)",
                        ),
                    );
                    continue;
                }
                crate::cel::MatchesPattern::Literal(source) => source,
            };
            let Err(unsupported) = pattern::javascript(&source) else {
                continue;
            };
            found.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    expression.source.span.clone(),
                    format!(
                        "{} (`{source}`)",
                        unsupported.about(&format!("{site} calls `matches()` with a pattern that"))
                    ),
                )
                .with_help(format!(
                    "{} — CEL defines `matches` over RE2 and the emitted evaluator has \
                     JavaScript's `RegExp`, so a pattern only one of them reads is one the two \
                     interpreters answer differently about (grammar 4.1, PRD 5.5)",
                    unsupported.help
                )),
            );
        }
    }
    found.sort();
    found.into_vec()
}

/// The header every generated file opens with.
///
/// `comment` is the line-comment marker of the file's own language, so the one
/// header reads the same in TypeScript, in JSON-with-comments (which
/// `package.json` is not — see [`project::package_json`]), and in a `.gitignore`.
///
/// The exact phrase `generated by agent-compose` is load-bearing: the acceptance
/// suite greps for it (`build_writes_a_typescript_project_for_the_target`), which
/// is what keeps a future rewording from quietly removing the one marker that
/// tells a reader — and a reviewer looking at a diff — that a file is not
/// theirs.
#[must_use]
pub fn header(ir: &Ir, comment: &str) -> String {
    let mut text = String::new();
    for line in header_lines(ir) {
        if line.is_empty() {
            text.push_str(comment.trim_end());
        } else {
            text.push_str(comment);
            text.push_str(&line);
        }
        text.push('\n');
    }
    text
}

/// The header as bare lines, for a file whose language has no line comment.
///
/// `package.json` is the case: JSON has no comments, so the header goes in a
/// `"//"` key — a spelling npm documents as ignored — and it needs the lines
/// rather than a rendered block.
#[must_use]
pub fn header_lines(ir: &Ir) -> Vec<String> {
    vec![
        format!(
            "This file was generated by agent-compose {COMPILER_VERSION} from `{}` (target `{}`).",
            ir.entrypoint, ir.target
        ),
        String::new(),
        "It is a build artifact, not source: `agent-compose build` overwrites it,".to_string(),
        "and `agent-compose build --check` fails when it has been edited. The spec".to_string(),
        "is the single source of truth (PRD 5.12); to own this code instead, copy".to_string(),
        "the whole directory out and stop regenerating it.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    /// The target check reaches every type node, not only the top-level ones,
    /// and reports each pattern once against its own span.
    #[test]
    fn a_pattern_the_target_cannot_express_is_reported_wherever_it_is_nested() {
        let ir = ir_of(
            r#"version: "0.1"

state:
  fine: { type: string, pattern: "^(?<word>[a-z]+)\\d*$" }
  deep:
    type: array
    max_items: 4
    items:
      type: object
      properties:
        inner: { type: string, pattern: "(?P<word>[a-z]+)" }
  flagged: { type: string, pattern: "(?i)abc" }
"#,
        );
        let reported = diagnostics(&ir);
        assert_eq!(
            reported.len(),
            2,
            "one per unrepresentable pattern, and none for the representable one: {reported:#?}"
        );
        assert!(reported.iter().all(Diagnostic::is_error));
        assert!(
            reported[0].message.contains("(?P<word>…)"),
            "the nested one is found: {:?}",
            reported[0].message
        );
        assert!(
            reported[1].message.contains("inline"),
            "{:?}",
            reported[1].message
        );
        assert!(
            reported[0].span.bytes.start < reported[1].span.bytes.start,
            "the report is in source order"
        );
    }

    /// A declaration two surfaces reach is reported once.
    ///
    /// A `kv` `get` derives `{value, found}` with the store's own `value_schema`
    /// inside it (grammar 11.4), so the pattern below is walked three times —
    /// once as `store.s.value_schema` and once per node — and the reader is told
    /// about it once, against the span where it is written.
    #[test]
    fn a_pattern_reached_by_more_than_one_surface_is_reported_once() {
        let ir = ir_of(
            r#"version: "0.1"

store.s:
  kind: kv
  scope: global
  description: Holds a slug.
  value_schema:
    slug: { type: string, pattern: "(?i)^abc$" }

flow.f:
  outputs: {}
  nodes:
    look: { store: store.s, op: get, key: "'k'" }
    again: { store: store.s, op: get, key: "'j'" }
  edges:
    - { from: start, to: look }
    - { from: look, to: again }
    - { from: again, to: end }
"#,
        );
        // The surfaces that hold it, so a change to either half of the claim
        // shows up here rather than in the count alone.
        let surfaces = schema::surfaces(&ir);
        let holders: Vec<&str> = surfaces
            .iter()
            .filter(|surface| {
                let mut holds = false;
                surface.walk(&mut |ty| {
                    if let TypeForm::Scalar(scalar) = &ty.form
                        && scalar.pattern.is_some()
                    {
                        holds = true;
                    }
                });
                holds
            })
            .map(|surface| surface.path.as_str())
            .collect();
        assert_eq!(
            holders,
            [
                "flow.f.node.look.output",
                "flow.f.node.again.output",
                "store.s.value_schema",
            ],
            "the pattern is reachable from three surfaces"
        );

        let reported = diagnostics(&ir);
        assert_eq!(reported.len(), 1, "{reported:#?}");
        assert!(reported[0].message.contains("inline"));
    }

    /// Every `pattern:` in a composition the rest of the suite compiles is one
    /// this target can express, so the check is not a blanket refusal.
    #[test]
    fn the_worked_shapes_have_nothing_the_target_cannot_express() {
        assert_eq!(diagnostics(&ir_of(test_support::EVERY_FORM)), []);
    }

    /// The same question, asked of the regular expression a `matches()` carries.
    ///
    /// One guard writes four patterns: one both engines read alike, one only the
    /// crate reads (`(?i)`), one only `RegExp` reads (look-ahead), and one the
    /// compiler never sees. Three are reported, **each on its own**, because two
    /// bad patterns in one expression are two things to fix — and the fourth is
    /// not, which is what keeps this from being a check that refuses `matches()`.
    #[test]
    fn a_matches_pattern_the_target_cannot_express_is_reported_once_per_call() {
        let ir = ir_of(
            r#"version: "0.1"

state:
  note: { type: string, default: "" }
  needle: { type: string, default: "^a$" }

flow.f:
  outputs: {}
  nodes:
    read: { exec: { command: printf, args: ["x"] } }
    act: { exec: { command: printf, args: ["y"] } }
  edges:
    - { from: start, to: read }
    - from: read
      to: act
      when: >-
        state.note.matches('^[a-z]+$') || state.note.matches('(?i)urgent') ||
        matches(state.note, 'a(?=b)') || state.note.matches(state.needle)
    - { from: read, to: end, else: true }
    - { from: act, to: end }
"#,
        );
        let reported = diagnostics(&ir);
        let messages: Vec<&str> = reported
            .iter()
            .map(|diagnostic| diagnostic.message.as_str())
            .collect();
        assert_eq!(
            messages.len(),
            3,
            "one per undecidable call, none for the shared-subset one: {messages:#?}"
        );
        assert!(reported.iter().all(Diagnostic::is_error));
        for expected in [
            "sets flags inline",
            "look-around, including look-ahead and look-behind, is not supported",
            "the argument is computed rather than written down",
        ] {
            assert!(
                messages.iter().any(|message| message.contains(expected)),
                "no diagnostic carries `{expected}`: {messages:#?}"
            );
        }
        assert!(
            messages
                .iter()
                .all(|message| message.contains("`flow.f`'s edge `read` → `act` guard")),
            "each names the surface, which a span alone does not: {messages:#?}"
        );
        assert!(
            !messages.iter().any(|message| message.contains("^[a-z]+$")),
            "the control pattern transfers and is not reported: {messages:#?}"
        );
    }

    /// A `matches()` this target *can* express is silent at every surface that
    /// carries one — the over-refusal guard for the check above.
    #[test]
    fn a_matches_pattern_both_engines_read_alike_is_not_reported_anywhere() {
        let ir = ir_of(
            r#"version: "0.1"

state:
  note: { type: string, default: "" }
  items:
    type: array
    max_items: 4
    items: { type: string }
    default: []

flow.f:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    read:
      exec:
        command: printf
        args: ["x"]
      input:
        slug: "input.goal.matches('^[a-z]+-[0-9]{4}$') ? input.goal : ''"
    act: { exec: { command: printf, args: ["y"] } }
  edges:
    - { from: start, to: read }
    - { from: read, to: act, when: "state.items.exists(i, matches(i, '^(?<w>[a-z]+)$'))" }
    - { from: read, to: end, else: true }
    - { from: act, to: end }
"#,
        );
        assert_eq!(diagnostics(&ir), [], "the shared vocabulary is accepted");
    }

    /// A channel named after a property every JavaScript object carries is
    /// refused, and the names that merely *look* like it are not.
    ///
    /// `constructor` validates, emits a project that type-checks and loads, and
    /// then fails at `new StateGraph(State)` — past every gate a build has, which
    /// is why the refusal is here rather than left to a runtime nobody runs at
    /// build time. The three neighbours are the over-refusal guard: a check that
    /// matched on a prefix, or on "looks like a JavaScript thing", would take
    /// them too.
    #[test]
    fn a_channel_named_after_a_property_every_object_carries_is_refused() {
        let ir = ir_of(
            r#"version: "0.1"

state:
  construct: { type: string }
  constructors: { type: string }
  constructor_name: { type: string }
  constructor: { type: string, default: "ctor" }
"#,
        );
        let reported = diagnostics(&ir);
        assert_eq!(reported.len(), 1, "{reported:#?}");
        assert!(reported[0].is_error());
        assert!(
            reported[0].message.contains("`constructor`"),
            "{:?}",
            reported[0].message
        );
        assert!(
            reported[0]
                .message
                .contains("every JavaScript object answers from `Object.prototype`"),
            "the message says why, not just that: {:?}",
            reported[0].message
        );
        assert!(
            reported[0]
                .help
                .as_ref()
                .is_some_and(|help| help.contains("rename the channel")),
            "{:?}",
            reported[0].help
        );

        // The span is the channel's own name, on its own line, rather than the
        // whole `state:` section: the reader has to be pointed at the key they
        // have to change.
        assert_eq!(
            (
                reported[0].span.start.line,
                reported[0].span.start.column,
                reported[0].span.bytes.len()
            ),
            (7, 3, "constructor".len()),
        );
    }

    /// The same name one level down: a **schema property** and a
    /// **`discriminator`** are keys of an emitted object shape too, and the
    /// prototype lookup does not care which of the three positions it is in.
    ///
    /// Each of these is a project the compiler used to emit happily. The
    /// property one type-checks, loads, and then refuses `{"a": "x"}` — a
    /// document its own published JSON Schema accepts, because grammar 3.6 makes
    /// a defaulted property optional — with `expected string, received
    /// function`. The discriminator one gets further still: it type-checks,
    /// constructs its graph, and throws `propValues[key].add is not a function`
    /// out of the first `safeParse`.
    #[test]
    fn a_schema_key_named_after_a_property_every_object_carries_is_refused() {
        let ir = ir_of(
            r#"version: "0.1"

state:
  proto:
    type: object
    properties:
      a: { type: string }
      constructor: { type: string, default: "d" }
  tagged:
    type: array
    max_items: 4
    items:
      discriminator: constructor
      variants:
        one: { x: { type: string } }
        two: { y: { type: integer } }
  fine:
    type: object
    properties:
      constructors: { type: string }
      construct: { type: string }
"#,
        );
        let reported = diagnostics(&ir);
        assert_eq!(reported.len(), 2, "{reported:#?}");
        assert!(reported.iter().all(Diagnostic::is_error));

        assert!(
            reported[0]
                .message
                .contains("a schema property named `constructor`"),
            "{:?}",
            reported[0].message
        );
        assert!(
            reported[0]
                .message
                .contains("the parse refuses what the published JSON Schema accepts"),
            "the message says which way it fails: {:?}",
            reported[0].message
        );
        // The span is the property key itself, not the channel above it.
        assert_eq!(
            (
                reported[0].span.start.line,
                reported[0].span.start.column,
                reported[0].span.bytes.len()
            ),
            (8, 7, "constructor".len()),
        );

        assert!(
            reported[1]
                .message
                .contains("a discriminator named `constructor`"),
            "{:?}",
            reported[1].message
        );
        assert!(
            reported[1].message.contains("throws instead of answering"),
            "{:?}",
            reported[1].message
        );
        assert_eq!(
            (
                reported[1].span.start.line,
                reported[1].span.start.column,
                reported[1].span.bytes.len()
            ),
            (13, 22, "constructor".len()),
        );
    }

    /// A key two surfaces reach is reported once, and a variant *tag* — which is
    /// a value rather than a key — is not reported at all.
    #[test]
    fn a_schema_key_reached_by_more_than_one_surface_is_reported_once() {
        let ir = ir_of(
            r#"version: "0.1"

store.s:
  kind: kv
  scope: global
  description: Holds a row.
  value_schema:
    constructor: { type: string }

flow.f:
  outputs: {}
  nodes:
    look: { store: store.s, op: get, key: "'k'" }
    again: { store: store.s, op: get, key: "'j'" }
  edges:
    - { from: start, to: look }
    - { from: look, to: again }
    - { from: again, to: end }
"#,
        );
        let reported = diagnostics(&ir);
        assert_eq!(reported.len(), 1, "{reported:#?}");
        assert!(reported[0].message.contains("a schema property"));
    }

    /// Grammar 2.1's identifier admits exactly one of the inherited property
    /// names, so `constructor` is the whole of what this check can ever report —
    /// in any of the three positions, since a channel name, a field-map property
    /// name and a `discriminator` are all that one identifier class. It is why
    /// it is the only name the corpus and the fixtures carry.
    #[test]
    fn constructor_is_the_only_inherited_property_name_an_identifier_can_spell() {
        let reachable: Vec<&str> = state::INHERITED_PROPERTY_NAMES
            .iter()
            .copied()
            // Grammar 2.1: `lower , { lower | digit | "_" }`.
            .filter(|name| {
                let mut characters = name.chars();
                characters
                    .next()
                    .is_some_and(|first| first.is_ascii_lowercase())
                    && characters.all(|rest| {
                        rest.is_ascii_lowercase() || rest.is_ascii_digit() || rest == '_'
                    })
            })
            .collect();
        assert_eq!(reachable, ["constructor"]);
    }

    #[test]
    fn every_generated_file_carries_the_header_and_ends_in_a_newline() {
        let ir = ir_of("version: \"0.1\"\n");
        let project = emit(&ir, &authored::Authored::none());
        assert!(!project.files().is_empty());
        for file in project.files() {
            assert!(
                file.contents.contains("generated by agent-compose"),
                "`{}` carries no generated-file header",
                file.path
            );
            assert!(
                file.contents.ends_with('\n'),
                "`{}` does not end in a newline",
                file.path
            );
            assert!(
                !file.contents.contains('\r'),
                "`{}` carries a carriage return",
                file.path
            );
        }
    }

    /// [`EMITTED_PATHS`] is the compiler's whole claim on an output directory
    /// (PRD resolved q47), and everything downstream reads it as a constant: the
    /// front end refuses a `module:` binding that names one of these, `build`
    /// writes and `--check`s exactly this list, and nothing else under the
    /// output directory is touched. So the constant has to be the emitter's own
    /// answer for **every** composition, not for the one a test happened to
    /// pick — two with nothing in common is the cheapest way to say so.
    #[test]
    fn the_file_set_is_the_documented_layout_for_every_composition() {
        for source in [
            "version: \"0.1\"\n",
            crate::codegen::test_support::EVERY_FORM,
        ] {
            let project = emit(&ir_of(source), &authored::Authored::none());
            assert_eq!(project.paths().collect::<Vec<_>>(), EMITTED_PATHS);
        }
    }

    /// Every emitted name fits the tar header the artifact is served in.
    ///
    /// `docs/distributed.md` §3.5 packs [`EMITTED_PATHS`] and the authored files
    /// beside them into a ustar archive, whose name field holds 100 bytes. The
    /// authored half is bounded where it is written (`parse::binding`), and this
    /// is the other half: a name added here that did not fit would make the
    /// hub's artifact route throw for **every** composition, with nothing else in
    /// the suite reaching it — the mesh tests run a build whose paths are all
    /// short, and would go on passing until the day one was not.
    #[test]
    fn every_emitted_name_fits_the_header_the_artifact_is_served_in() {
        for path in EMITTED_PATHS {
            assert!(
                path.len() <= 100,
                "`{path}` is {} bytes, and a ustar header holds 100",
                path.len()
            );
        }
    }

    /// A composition that binds a module emits **more** than the constant list:
    /// the tree is the emission set plus what the spec references (PRD resolved
    /// q49), and the two halves are separable.
    #[test]
    fn a_module_binding_widens_the_tree_without_moving_the_emission_set() {
        let ir = ir_of(
            r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input: {}
  output: {}
  module: ./src/tools/sign.ts
"#,
        );
        let project = emit(
            &ir,
            &authored::Authored::of([("src/tools/sign.ts".to_string(), "// yours\n".to_string())]),
        );
        assert_eq!(
            project
                .files()
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            EMITTED_PATHS,
            "the compiler's claim on a directory does not depend on the spec"
        );
        assert_eq!(
            project
                .carried()
                .iter()
                .map(|file| (file.path.as_str(), file.contents.as_str()))
                .collect::<Vec<_>>(),
            [("src/tools/sign.ts", "// yours\n")]
        );
        let mut every = EMITTED_PATHS.to_vec();
        every.push("src/tools/sign.ts");
        every.sort_unstable();
        assert_eq!(project.paths().collect::<Vec<_>>(), every);
        assert!(
            project.file("src/tools/sign.ts").is_none(),
            "`file` answers the emission set, which is what `--check`'s \
             generated-file questions are asked of"
        );
    }

    /// A caller that did not read what the composition references is a bug, and
    /// it is a loud one: an artifact whose hash silently omitted an entry would
    /// be served under a name a worker's own verification then refuses.
    #[test]
    #[should_panic(expected = "which was not read")]
    fn emitting_without_the_authored_bytes_is_refused_rather_than_guessed() {
        let ir = ir_of(
            r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input: {}
  output: {}
  module: ./src/tools/sign.ts
"#,
        );
        let _ = emit(&ir, &authored::Authored::none());
    }

    /// The one property the whole pass exists to have.
    #[test]
    fn emitting_twice_answers_the_same_bytes() {
        let ir = ir_of(crate::codegen::test_support::EVERY_FORM);
        let authored = authored::Authored::none();
        assert_eq!(emit(&ir, &authored), emit(&ir, &authored));
    }

    #[test]
    fn the_header_names_the_tool_the_version_and_the_spec_source() {
        let ir = ir_of("version: \"0.1\"\n");
        let header = header(&ir, "// ");
        assert!(header.contains(&format!("generated by agent-compose {COMPILER_VERSION}")));
        assert!(header.contains("from `main.yml` (target `local`)"));
        assert!(header.lines().all(|line| line.starts_with("//")));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::atomic::{AtomicU32, Ordering};

    use crate::ir::Ir;

    /// Resolve a one-file composition written inline, and hand back its IR.
    ///
    /// The emitter's unit tests are about the *mapping*, so they are written
    /// against the smallest composition that exercises one rule rather than
    /// against a fixture project. The composition still goes through the real
    /// parser and resolver — an IR built by hand could hold a shape the front
    /// end cannot produce, and a mapping tested against one of those proves
    /// nothing.
    ///
    /// # Panics
    ///
    /// Panics when the composition does not resolve, naming what was reported:
    /// a test whose input is invalid is a broken test, not a finding.
    pub fn ir_of(source: &str) -> Ir {
        placed(source, None)
    }

    /// The same, with a deploy layer, resolved under the target `mesh`.
    ///
    /// What the environment partition's tests need and [`ir_of`] cannot give:
    /// `placements:` is a deploy-layer section, so a composition without one
    /// resolves to an artifact with nothing to partition (grammar §14.1).
    ///
    /// # Panics
    ///
    /// Panics when the composition does not resolve **or does not validate**.
    /// The deploy layer's own rules — disjointness, the colocation of an
    /// attached tool — are what a partition is computed over, so a fixture the
    /// validator would refuse is a fixture whose answer means nothing.
    pub fn ir_of_mesh(source: &str, deploy: &str) -> Ir {
        let ir = placed(source, Some(deploy));
        let diagnostics = crate::check(&ir);
        assert!(
            diagnostics.is_empty(),
            "the test composition does not validate: {diagnostics:#?}"
        );
        ir
    }

    fn placed(source: &str, deploy: Option<&str>) -> Ir {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "agent-compose-codegen-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        let entrypoint = directory.join("main.yml");
        std::fs::write(&entrypoint, source).expect("the entrypoint is writable");
        let resolution = if let Some(deploy) = deploy {
            std::fs::create_dir_all(directory.join("deploy")).expect("a deploy directory");
            std::fs::write(directory.join("deploy/mesh.yml"), deploy)
                .expect("the deploy file is writable");
            crate::resolve_with_target(&entrypoint, "mesh")
        } else {
            crate::resolve(&entrypoint)
        };
        let _ = std::fs::remove_dir_all(&directory);
        assert!(
            resolution.diagnostics.is_empty(),
            "the test composition does not resolve: {:#?}",
            resolution.diagnostics
        );
        resolution.ir.expect("a clean resolution has an artifact")
    }

    /// A composition that reaches every schema form and every reduce policy, for
    /// the tests whose subject is the emitter as a whole rather than one rule.
    pub const EVERY_FORM: &str = r#"version: "0.1"

state:
  draft: { type: string, default: "" }
  round: { type: integer, default: 1 }
  notes:
    type: array
    max_items: 8
    items: { type: string }
    reduce: append
  totals:
    type: object
    properties:
      fixed: { type: integer }
      skipped: { type: integer }
    reduce: merge
  latest: { type: string, reduce: last_wins }
  unset: { type: boolean }

provider.p:
  kind: anthropic
  api_key: ${SOME_KEY}

model.m:
  provider: provider.p
  id: some-model

tool.lookup:
  description: Look something up.
  input:
    query: { type: string, min_length: 1 }
  output:
    snippet: { type: string }
  exec:
    command: printf
    args: ["a snippet"]

agent.triage:
  model: model.m
  prompt: Triage the report.
  input:
    report: { type: string }
  output:
    findings:
      description: Every distinct defect, tagged for routing.
      type: array
      max_items: 50
      items:
        discriminator: kind
        variants:
          auto_fixable:
            file: { type: string, pattern: "^[a-z/]+$" }
            confidence: { type: number, minimum: 0, maximum: 1 }
          needs_human:
            summary: { type: string }
            severity: { enum: [low, high, critical] }
    author:
      type: object
      properties:
        name: { type: string }
        email: { type: string, format: email }
      optional: [email]

flow.triage:
  outputs:
    draft: { type: string }
  nodes:
    plan: { agent: agent.triage, input: { report: "input.report" } }
    ask:
      human:
        input: { question: { type: string } }
        output: { decision: { enum: [approve, reject] } }
  inputs:
    report: { type: string }
  edges:
    - { from: start, to: plan }
    - { from: plan, to: ask }
    - { from: ask, to: end }
"#;
}
