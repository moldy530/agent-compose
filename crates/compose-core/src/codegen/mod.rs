//! Codegen: the flat IR in, a TypeScript project out (PRD 5.12, §7 M1).
//!
//! # The shape of the pass
//!
//! [`emit`] is a **pure function**. It takes an [`Ir`] and answers a
//! [`GeneratedProject`] — a set of `(path, contents)` pairs — and it touches no
//! filesystem, reads no environment, and consults no clock. The CLI is what
//! writes the bytes ([`agent-compose build`]), and `build --check` is the same
//! function run against what is already on disk. Three things follow, and each
//! is a requirement rather than a convenience:
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
//! .gitignore            the one directory a generated project acquires
//! src/env.ts            every `${ENV}` reference, and the process-start check
//! src/schemas.ts        every schema in the composition, as Zod (grammar 3.8)
//! src/state.ts          the LangGraph state model (grammar 10)
//! src/graph.ts          the graph module
//! src/index.ts          the project's public surface
//! ```
//!
//! `src/` is **compiler-owned**: `build` removes files under it that it did not
//! emit, and `build --check` reports them as drift. Nothing outside `src/` is
//! ever removed, because that is where a user's `node_modules`, lockfile, and
//! `.env` live — a directory the compiler writes into is not a directory it owns
//! outright.
//!
//! ## Why there is no build step
//!
//! The emitted project is run by Node directly: `node src/index.ts`. Node has
//! stripped types natively since 22.18, so a `.ts` file is a runnable module,
//! and TypeScript is a **checker** here rather than a compiler. Consequences,
//! all of them deliberate:
//!
//! * relative imports are written with their real `.ts` extension, which is what
//!   Node resolves and what `allowImportingTsExtensions` makes legal;
//! * `agent-compose run` and `serve` spawn `node` on the sources, so there is no
//!   stale-`dist/` failure mode and no second artifact to keep in step;
//! * `tsc --noEmit` is the type gate and nothing else, so a project that fails to
//!   type-check still fails loudly in CI while never being on a run's critical
//!   path;
//! * ejecting (PRD 5.12) is copying the directory: it is already a plain Node
//!   project with no toolchain of ours in it.
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
//! Node functions, routers, bounded-cycle counters, `map`→`Send`, subgraphs,
//! store ops, and model routing are the later M1 bullets. [`graph`] emits the
//! module they will land in and the state model they will build on; what it does
//! not do is pretend to a topology it cannot execute.

pub mod env;
pub mod graph;
pub mod names;
pub mod pattern;
pub mod project;
pub mod schema;
pub mod state;

use crate::diag::{Diagnostic, DiagnosticCode};
use crate::ir::Ir;
use crate::ir::schema::TypeForm;

/// The compiler release this build is, as it appears in every generated header.
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The directory inside a generated project that the compiler owns outright.
///
/// `build` removes files under it that it did not emit and `build --check`
/// reports them; see the module docs.
pub const OWNED_DIRECTORY: &str = "src";

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
/// The files are sorted by path and the set is complete — there is no "and also
/// copy these" step anywhere. A consumer writes them, compares them, or hashes
/// them; nothing else is needed to have the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeneratedProject {
    files: Vec<GeneratedFile>,
}

impl GeneratedProject {
    /// Build a project from its files, sorting them by path.
    ///
    /// # Panics
    ///
    /// Panics when two files claim one path. That is an emitter bug rather than
    /// a composition error — every path here is derived from a fixed layout, not
    /// from user input — and a silent last-one-wins would ship a project missing
    /// a module.
    fn new(mut files: Vec<GeneratedFile>) -> Self {
        files.sort_by(|left, right| left.path.cmp(&right.path));
        for pair in files.windows(2) {
            assert!(
                pair[0].path != pair[1].path,
                "two generated files claim `{}`",
                pair[0].path
            );
        }
        Self { files }
    }

    /// Every file, sorted by path.
    #[must_use]
    pub fn files(&self) -> &[GeneratedFile] {
        &self.files
    }

    /// The file at this path, if the project has one.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&GeneratedFile> {
        self.files.iter().find(|file| file.path == path)
    }

    /// Every path, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|file| file.path.as_str())
    }
}

/// Lower one resolved composition to a TypeScript project.
///
/// The IR must have passed [`crate::check`]: this pass reports nothing and
/// refuses nothing, because everything it could refuse the validator has already
/// refused with a span to point at. `build` runs the validator first and emits
/// only on a clean report (PRD §7 M1).
#[must_use]
pub fn emit(ir: &Ir) -> GeneratedProject {
    let names = names::Names::of(ir);
    let environment = env::References::of(ir);

    GeneratedProject::new(vec![
        project::package_json(ir),
        project::tsconfig_json(ir),
        project::readme(ir),
        project::gitignore(ir),
        env::module(ir, &environment),
        schema::module(ir, &names),
        state::module(ir, &names),
        graph::module(ir),
        project::index(ir),
    ])
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
        let project = emit(&ir);
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

    #[test]
    fn the_file_set_is_sorted_and_is_the_documented_layout() {
        let ir = ir_of("version: \"0.1\"\n");
        let project = emit(&ir);
        assert_eq!(
            project.paths().collect::<Vec<_>>(),
            [
                ".gitignore",
                "README.md",
                "package.json",
                "src/env.ts",
                "src/graph.ts",
                "src/index.ts",
                "src/schemas.ts",
                "src/state.ts",
                "tsconfig.json",
            ]
        );
    }

    /// The one property the whole pass exists to have.
    #[test]
    fn emitting_twice_answers_the_same_bytes() {
        let ir = ir_of(crate::codegen::test_support::EVERY_FORM);
        assert_eq!(emit(&ir), emit(&ir));
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
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let directory = std::env::temp_dir().join(format!(
            "agent-compose-codegen-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).expect("a scratch directory");
        let entrypoint = directory.join("main.yml");
        std::fs::write(&entrypoint, source).expect("the entrypoint is writable");
        let resolution = crate::resolve(&entrypoint);
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
