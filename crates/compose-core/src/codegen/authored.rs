//! The authored side of the boundary: the stub `build` writes for a `module:`
//! binding whose file is not there, and the bytes of the ones that are there
//! (grammar 6.1, PRD resolved q47, q48, q49).
//!
//! # Written once, and then never again
//!
//! Everything else this module's neighbours produce is a *build artifact*:
//! overwritten on every build, compared byte-for-byte by `--check`, and listed
//! in the manifest that says the compiler owns it. A scaffold is the opposite of
//! all three. It is written **only when the file is absent**, it is never read
//! back, and it is not in [`EMITTED_PATHS`](super::EMITTED_PATHS) — so `--check`
//! has nothing to say about it and a rebuild leaves the author's bytes exactly
//! where they are.
//!
//! That is what makes it safe to put authored code in the same tree without a
//! marker protocol. PRD resolved q47 rejects manual sections inside generated
//! files precisely because they turn `build` into a read-modify-write of its own
//! previous output; a stub-once scaffold gives the same ergonomics — you never
//! type the signature — with none of that, because the compiler's only
//! interaction with the file is a `File::exists` test.
//!
//! # What holds the contract, since nothing here does
//!
//! `tsc`. The stub is written as the generated contract type
//! ([`super::modules`]), which is derived from the tool's own `input:` and
//! `output:`, so a change to either schema is a type error in the authored file
//! naming the field that moved — the merge tool PRD resolved q48 chooses over
//! marker comments. Nothing in this module ever looks at what the author wrote
//! there.
//!
//! # …and the bytes, which the emitter does read
//!
//! [`Authored`] is the other half, and the one place in this crate that reads a
//! file the compiler did not write. PRD resolved q49 widens the artifact's file
//! list to "what `build` wrote **plus** the authored files the spec references":
//! a referenced module is hashed into `ARTIFACT_HASH` like every other entry and
//! is carried into the emitted tree, because a worker holds nothing but the
//! artifact and a module tool executes wherever its tool executes.
//!
//! That is a read of *content*, not of ownership, and the difference is the
//! whole of what keeps [`Scaffold`] and [`Authored`] distinct types. A scaffold
//! asks `exists()` and writes when the answer is no; this reads what is there
//! and carries it. Neither ever merges, and neither is a read of the compiler's
//! own previous output.
//!
//! # Which tree the specifier is written against
//!
//! The **artifact's**. A `module:` path is project-relative — the entrypoint's
//! own directory, exactly as an `imports:` entry is (grammar 1.4, 6.1) — and it
//! is the *same* relative path the file takes inside the artifact, which is what
//! PRD resolved q49's widened file list means by "what `build` wrote plus the
//! authored files the spec references": one path space, generated modules and
//! authored ones alike, with a collision made unwritable by the rule that a
//! binding may not name a file the emitter emits.
//!
//! So [`relative`] answers the specifier for that one tree — `../modules.ts`
//! from `src/tools/sign.ts` — and it is the specifier that resolves wherever the
//! two halves sit beside each other. That the authored half is also *edited*
//! project-side, one copy shared by every target, is what keeps a composition
//! built for two targets from asking its author to write the implementation
//! twice.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use crate::ir::Ir;

use super::names::{self, Names};

/// The bytes of every authored file a composition references, by its
/// project-relative path.
///
/// What [`emit`](super::emit) needs and cannot go and get: it is a pure function
/// of its arguments (see [`super`]), and the artifact's hash and file list now
/// cover files it did not write. So the caller reads them — [`Authored::read`]
/// is that read — and hands them in.
///
/// A composition with no `module:` binding has an empty one, which is what
/// [`Authored::none`] answers and what every emission of such a composition is
/// given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Authored {
    files: BTreeMap<String, String>,
}

impl Authored {
    /// The empty set: what a composition binding no module is emitted with.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Read every `module:` binding's file out of the project root.
    ///
    /// `root` is the entrypoint's own directory, which is what a `module:` path
    /// is relative to (grammar 1.4, 6.1).
    ///
    /// # Errors
    ///
    /// The filesystem's, with the path folded into the message — a build whose
    /// artifact cannot include a file the artifact is defined to include has no
    /// answer to give. An absent file is one of these: a plain `build` scaffolds
    /// before it reads, and `validate` and `build --check` refuse before they
    /// reach here ([`crate::check_modules`]).
    pub fn read(ir: &Ir, root: &Path) -> io::Result<Self> {
        let mut files = BTreeMap::new();
        for (_, module) in crate::check::modules::bindings(ir) {
            let path = module.path.value.as_str();
            if files.contains_key(path) {
                continue;
            }
            let mut full = root.to_path_buf();
            full.extend(path.split('/'));
            let bytes = std::fs::read(&full).map_err(|error| {
                io::Error::new(error.kind(), format!("cannot read `{path}`: {error}"))
            })?;
            let text = String::from_utf8(bytes).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "`{path}` is not UTF-8: an authored module is TypeScript source, and the \
                         artifact's file list carries it as text"
                    ),
                )
            })?;
            files.insert(path.to_string(), text);
        }
        Ok(Self { files })
    }

    /// Build one from `(path, contents)` pairs, for a caller that already holds
    /// the bytes — the golden corpus reads them out of a committed fixture
    /// rather than off a project it built.
    #[must_use]
    pub fn of(files: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            files: files.into_iter().collect(),
        }
    }

    /// The bytes at one project-relative path.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// Every path, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// Whether nothing was referenced.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}

/// One authored file `build` writes when it is absent.
///
/// Not a [`GeneratedFile`](super::GeneratedFile), and the distinct type is the
/// point: a generated file is written every build and compared by `--check`,
/// and this is written once and never looked at again. Two names for two
/// contracts keeps a caller from passing one where the other belongs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scaffold {
    /// Where it goes, relative to the **project root** — the entrypoint's own
    /// directory — `/`-separated, which is the binding's own normalized path. Not
    /// relative to `--out`: a `module:` path is project-relative like an
    /// `imports:` entry, and `agent-compose validate`, which has no `--out`, is
    /// the pass that refuses one whose file is missing.
    pub path: String,
    /// Its bytes.
    pub contents: String,
    /// The tool this file implements, for the report a build writes.
    pub tool: String,
}

/// The stub for every `module:` binding the composition declares, in tool
/// address order.
///
/// Pure, like [`emit`](super::emit): the caller decides which of these are
/// missing on disk and writes only those.
#[must_use]
pub fn scaffolds(ir: &Ir) -> Vec<Scaffold> {
    let names = super::registry(ir);
    crate::check::modules::bindings(ir)
        .into_iter()
        .map(|(address, module)| Scaffold {
            path: module.path.value.clone(),
            contents: stub(ir, &names, address, &module.path.value),
            tool: address.to_string(),
        })
        .collect()
}

/// One stub's bytes.
fn stub(ir: &Ir, names: &Names, address: &str, path: &str) -> String {
    let tool = match ir.definition(address).map(|definition| &definition.body) {
        Some(crate::ir::definition::DefinitionBody::Tool(tool)) => tool,
        _ => unreachable!("`bindings` yields tool definitions"),
    };
    let contract = names.ty(&format!("{address}.module"));
    let seam = relative(path, super::modules::PATH);
    let function = names::camel(address);

    let doc: Vec<String> = vec![
        format!("`{address}` — {}", tool.description.value),
        String::new(),
        format!(
            "`agent-compose build` wrote this file once, because `{path}` was not there. It is \
             yours from here: the compiler never writes it again, never reads it to re-emit it, \
             and `agent-compose build --check` never compares it against a template — what a \
             build owns is the file list it emits, and this is not on it (PRD resolved q47)."
        ),
        String::new(),
        format!(
            "`{contract}` is the contract, generated from this tool's own `input:` and `output:` \
             and always current: change a schema and this file stops type-checking, naming the \
             field that moved (PRD resolved q48). Read it in `{seam}` for what the arguments and \
             the result are."
        ),
        String::new(),
        format!(
            "The file travels with the artifact, because the spec references it (PRD resolved \
             q49): editing it moves the artifact hash, and every worker is handed the new one."
        ),
    ];

    // Imports first and the doc comment directly above the function it
    // documents, which is where a TypeScript author expects to find both — the
    // stub is read as source rather than as generated output.
    let mut text = format!(
        "import type {{ {contract} }} from {};\n\n",
        names::string(&seam)
    );
    text.push_str(&names::doc("", &doc));
    text.push_str(&format!(
        "const {function}: {contract} = async (input) => {{\n"
    ));
    text.push_str(&format!(
        "  // TODO: implement `{address}`, and delete the throw below.\n  void input;\n  throw new Error({});\n}};\n",
        names::string(&format!("`{address}` is not implemented yet: `{path}`"))
    ));
    text.push_str(&format!("\nexport default {function};\n"));
    text
}

/// One doc-comment line per declared field, or the sentence an empty map gets.
///
/// Shared with [`super::modules`], which writes the same list into the contract
/// type's own doc comment: one description of a tool's surface, in the two
/// places an author meets it.
pub(super) fn fields(map: &crate::ir::schema::FieldMap, empty: &str) -> Vec<String> {
    if map.fields.is_empty() {
        return vec![format!("  {empty}.")];
    }
    map.fields
        .iter()
        .map(|field| format!("  `{}` — {}", field.name.value.as_str(), form(&field.ty)))
        .collect()
}

/// The one-word name of a type node's form, for a doc comment.
fn form(ty: &crate::ir::schema::TypeNode) -> &'static str {
    use crate::ir::schema::TypeForm;
    match &ty.form {
        TypeForm::Scalar(scalar) => scalar.kind.as_str(),
        TypeForm::Enum(_) => "enum",
        TypeForm::Object(_) => "object",
        TypeForm::Array(_) => "array",
        TypeForm::Union(_) => "union",
    }
}

/// The specifier one project-relative module writes to import another.
///
/// Both paths are project-relative and `/`-separated, so this is the ordinary
/// lexical answer: drop the shared prefix, climb once per remaining directory of
/// the source, then descend. A specifier that would not start with `.` is given
/// one, because a bare `schemas.ts` is a *package* name to every resolver there
/// is.
pub(super) fn relative(from: &str, to: &str) -> String {
    let source: Vec<&str> = from.split('/').collect();
    let target: Vec<&str> = to.split('/').collect();
    // Every segment but the last is a directory.
    let directories = &source[..source.len() - 1];
    let shared = directories
        .iter()
        .zip(&target[..target.len() - 1])
        .take_while(|(left, right)| left == right)
        .count();
    let mut segments: Vec<String> = Vec::new();
    for _ in shared..directories.len() {
        segments.push("..".to_string());
    }
    for segment in &target[shared..] {
        segments.push((*segment).to_string());
    }
    let joined = segments.join("/");
    if joined.starts_with('.') {
        joined
    } else {
        format!("./{joined}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn a_specifier_climbs_out_of_the_authors_directory_and_back_in() {
        assert_eq!(
            relative("src/tools/sign.ts", "src/schemas.ts"),
            "../schemas.ts"
        );
        assert_eq!(relative("src/sign.ts", "src/schemas.ts"), "./schemas.ts");
        assert_eq!(relative("sign.ts", "src/schemas.ts"), "./src/schemas.ts");
        assert_eq!(
            relative("tools/deep/sign.ts", "src/schemas.ts"),
            "../../src/schemas.ts"
        );
    }

    const ONE_MODULE_TOOL: &str = r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
    rounds: { type: integer, default: 1 }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts
"#;

    /// The stub is a whole TypeScript module: it imports the generated contract
    /// type, declares itself as one, and throws.
    #[test]
    fn the_stub_types_itself_against_the_generated_contract() {
        let ir = ir_of(ONE_MODULE_TOOL);
        let written = scaffolds(&ir);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].path, "src/tools/sign.ts");
        assert_eq!(written[0].tool, "tool.sign");
        let contents = &written[0].contents;
        assert!(
            contents.contains("import type { ToolSignModule } from \"../modules.ts\";"),
            "{contents}"
        );
        assert!(
            contents.contains("const toolSign: ToolSignModule = async (input) => {"),
            "the annotation is what `tsc` holds the file to: {contents}"
        );
        assert!(
            contents.contains("export default toolSign;\n"),
            "{contents}"
        );
        assert!(
            contents.contains("`tool.sign` — Sign a payload."),
            "{contents}"
        );
        assert!(
            contents.contains("`ToolSignModule` is the contract"),
            "{contents}"
        );
        assert!(
            contents.contains("`tool.sign` is not implemented yet: `src/tools/sign.ts`"),
            "{contents}"
        );
        assert!(contents.ends_with(";\n"), "{contents}");
        assert!(
            contents.starts_with("import type"),
            "the stub reads as source: imports first, the contract directly above the function \
             it documents — {contents}"
        );
    }

    /// The contract's own doc comment carries the field list, and the stub
    /// points at it rather than restating it — a stub is written once and never
    /// rewritten, so a schema list inside it would go stale the moment the
    /// composition moved.
    #[test]
    fn the_field_list_lives_where_it_stays_current() {
        let ir = ir_of(ONE_MODULE_TOOL);
        let stub = &scaffolds(&ir)[0].contents;
        assert!(!stub.contains("`payload` — string"), "{stub}");

        let contract =
            crate::codegen::modules::module(&ir, &crate::codegen::registry(&ir)).contents;
        assert!(contract.contains("`payload` — string"), "{contract}");
        assert!(contract.contains("`rounds` — integer"), "{contract}");
        assert!(contract.contains("`signature` — string"), "{contract}");
    }

    /// The one property that makes it an *authored* file: nothing in it claims
    /// the compiler owns it, because the marker is what `build` reads to decide
    /// whether a file at an emitted path is its own to replace.
    #[test]
    fn the_stub_carries_no_generated_file_header() {
        let ir = ir_of(ONE_MODULE_TOOL);
        assert!(
            !scaffolds(&ir)[0]
                .contents
                .contains("generated by agent-compose")
        );
    }

    #[test]
    fn a_composition_with_no_module_binding_scaffolds_nothing() {
        assert_eq!(
            scaffolds(&ir_of(crate::codegen::test_support::EVERY_FORM)),
            []
        );
    }

    /// [`Authored::read`] reads exactly what the composition references, out of
    /// the project root, and says which file it could not (PRD resolved q49).
    #[test]
    fn the_reader_takes_the_referenced_files_and_nothing_beside_them() {
        let root = std::env::temp_dir().join(format!(
            "agent-compose-authored-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src/tools")).expect("a scratch directory");
        let ir = ir_of(ONE_MODULE_TOOL);

        let missing = Authored::read(&ir, &root).expect_err("the file is not there");
        assert!(
            missing.to_string().contains("src/tools/sign.ts"),
            "{missing}"
        );

        std::fs::write(root.join("src/tools/sign.ts"), "// yours\n").expect("writable");
        // A file under `src/` nothing references ships nowhere, which is the
        // half a directory walk would get wrong.
        std::fs::write(root.join("src/tools/scratch.ts"), "// mine\n").expect("writable");
        let read = Authored::read(&ir, &root).expect("the file is there now");
        assert_eq!(read.paths().collect::<Vec<_>>(), ["src/tools/sign.ts"]);
        assert_eq!(read.get("src/tools/sign.ts"), Some("// yours\n"));
        assert!(!read.is_empty());

        assert!(
            Authored::read(&ir_of("version: \"0.1\"\n"), &root)
                .expect("nothing to read")
                .is_empty()
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
