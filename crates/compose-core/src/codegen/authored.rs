//! The authored side of the boundary: the stub `build` writes for a `module:`
//! binding whose file is not there (grammar 6.1, PRD resolved q47, q48).
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
//! `tsc`. The stub's signature is written against the tool's own generated
//! schemas in `src/schemas.ts`, so a change to the tool's `input:` or `output:`
//! is a type error in the authored file naming the field that moved — which is
//! the merge tool PRD resolved q48 chooses over marker comments. Nothing in this
//! module ever looks at what the author wrote there.
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
//! So [`relative`] answers the specifier for that one tree — `../schemas.ts`
//! from `src/tools/sign.ts` — and it is the specifier that resolves wherever the
//! two halves sit beside each other. That the authored half is also *edited*
//! project-side, one copy shared by every target, is what keeps a composition
//! built for two targets from asking its author to write the implementation
//! twice.

use crate::ir::Ir;

use super::names::{self, Names};

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
    let names = Names::of(ir);
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
    let input = names.value(&format!("{address}.input"));
    let output = names.value(&format!("{address}.output"));
    let schemas = relative(path, "src/schemas.ts");
    let function = names::camel(address);

    let mut doc: Vec<String> = vec![
        format!("`{address}` — {}", tool.description.value),
        String::new(),
        format!(
            "`agent-compose build` wrote this file once, because `{path}` was not there. It is \
             yours from here: the compiler never writes it again, never reads it back, and \
             `agent-compose build --check` never compares it — what a build owns is the file list \
             it emits, and this is not on it (PRD resolved q47)."
        ),
        String::new(),
        "The contract is the tool's own `input:` and `output:` in the composition, and `tsc` is \
         what holds this file to it: change a schema and this signature stops type-checking, \
         naming the field that moved (PRD resolved q48)."
            .to_string(),
        String::new(),
    ];
    doc.push("Takes:".to_string());
    doc.extend(fields(&tool.input, "takes no arguments"));
    doc.push("Answers:".to_string());
    doc.extend(fields(&tool.output, "answers nothing"));

    // Imports first and the doc comment directly above the function it
    // documents, which is where a TypeScript author expects to find both — the
    // stub is read as source rather than as generated output.
    let mut text = String::from("import type { z } from \"zod\";\n\n");
    text.push_str(&format!(
        "import type {{ {input}, {output} }} from {};\n\n",
        names::string(&schemas)
    ));
    text.push_str(&names::doc("", &doc));
    text.push_str(&format!(
        "export default async function {function}(\n  input: z.infer<typeof {input}>,\n): Promise<z.infer<typeof {output}>> {{\n"
    ));
    text.push_str(&format!(
        "  // TODO: implement `{address}`, and delete the throw below.\n  void input;\n  throw new Error({});\n}}\n",
        names::string(&format!("`{address}` is not implemented yet: `{path}`"))
    ));
    text
}

/// One doc-comment line per declared field, or the sentence an empty map gets.
fn fields(map: &crate::ir::schema::FieldMap, empty: &str) -> Vec<String> {
    if map.fields.is_empty() {
        return vec![format!("  it {empty}.")];
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
fn relative(from: &str, to: &str) -> String {
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

    /// The stub is a whole TypeScript module: it imports the tool's own
    /// generated schemas, derives the signature from them, and throws.
    #[test]
    fn the_stub_types_itself_against_the_tools_generated_schemas() {
        let ir = ir_of(ONE_MODULE_TOOL);
        let written = scaffolds(&ir);
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].path, "src/tools/sign.ts");
        assert_eq!(written[0].tool, "tool.sign");
        let contents = &written[0].contents;
        assert!(
            contents
                .contains("import type { toolSignInput, toolSignOutput } from \"../schemas.ts\";"),
            "{contents}"
        );
        assert!(
            contents.contains(
                "export default async function toolSign(\n  input: z.infer<typeof toolSignInput>,\n): Promise<z.infer<typeof toolSignOutput>> {"
            ),
            "{contents}"
        );
        assert!(
            contents.contains("`tool.sign` — Sign a payload."),
            "{contents}"
        );
        assert!(contents.contains("`payload` — string"), "{contents}");
        assert!(contents.contains("`rounds` — integer"), "{contents}");
        assert!(contents.contains("`signature` — string"), "{contents}");
        assert!(contents.ends_with("}\n"), "{contents}");
        assert!(
            contents.starts_with("import type"),
            "the stub reads as source: imports first, the contract directly above the function \
             it documents — {contents}"
        );
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
}
