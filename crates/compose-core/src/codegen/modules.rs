//! `src/modules.ts`: the generated half of a `module:` binding — the contract
//! each authored file satisfies, and the one seam generated code reaches it
//! through (grammar 6.1, PRD resolved q47, q48).
//!
//! # Two jobs, one file
//!
//! PRD resolved q48 asks for both halves of "the type checker is the merge
//! tool", and they belong together because neither is useful alone:
//!
//! * **the contract.** One exported function type per module-bound tool,
//!   written from that tool's own `input:` and `output:` through the Zod
//!   [`super::schema`] already emits, beside a type for the `env:` the binding
//!   declared — the argument the runtime hands the implementation instead of
//!   writing the composition's variables into this process's own environment. A
//!   schema that moves is a type error in the authored file naming the field
//!   that moved, and so is a read of a variable the YAML never declared, which
//!   is what makes a marker protocol unnecessary (D132).
//! * **the seam.** One `import` per binding and one typed `const` per binding.
//!   The annotation is the check: `export const toolSignModule: ToolSignModule =
//!   toolSignModuleImplementation;` is an ordinary assignability test, so an
//!   authored default export whose shape is wrong fails here, at a line that
//!   names both the type and the file.
//!
//! # Import direction is a rule
//!
//! Generated code reaches authored code **only here** (D132). `src/graph.ts`
//! imports the `const`s this module exports and never a `./tools/*.ts`; the
//! authored side may import anything the project generates. One seam is what
//! makes that statement checkable — `the_only_generated_import_of_authored_code`
//! is the check — and it is what keeps the emitted import graph a fixed shape
//! whatever a composition binds.
//!
//! # Why the file is always emitted
//!
//! [`super::EMITTED_PATHS`] is a constant: the layout is the same for every
//! composition, and what varies is contents. A file that appeared only for
//! compositions with a module binding would make the compiler's claim on an
//! output directory depend on the spec, and the front end reads that list to
//! refuse a `module:` path naming a file the emitter writes (grammar 6.1). So a
//! composition that binds nothing gets this module with `export {};` in it,
//! which is the spelling `isolatedModules` requires of a file with nothing to
//! export.

use crate::ir::Ir;
use crate::ir::binding::Module;

use super::names::{self, Names};
use super::{GeneratedFile, authored};

/// Where this module sits in an emitted project.
pub const PATH: &str = "src/modules.ts";

/// Declare the names `src/modules.ts` exports, in the project's one namespace.
///
/// Three per binding — the typed `const` the seam exposes, the environment type
/// beside its contract, and the local the authored default is imported as — so a
/// tool called `modules` cannot collide with any of them (see
/// [`Names::declare`](super::names::Names::declare)).
pub fn declare(names: &mut Names, ir: &Ir) {
    for (address, _) in crate::check::modules::bindings(ir) {
        names.declare(&format!("{address}.module"));
        names.declare(&format!("{address}.module.env"));
        names.declare(&format!("{address}.module.implementation"));
    }
}

/// `src/modules.ts`.
#[must_use]
pub fn module(ir: &Ir, names: &Names) -> GeneratedFile {
    let bindings = crate::check::modules::bindings(ir);
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);

    if bindings.is_empty() {
        contents.push_str(
            "\n// This composition binds no `module:` implementation, so there is nothing to\n\
             // import and nothing to expose. The file is still emitted, because the emitted\n\
             // file list is a constant of the compiler rather than of the composition.\n\
             export {};\n",
        );
        return GeneratedFile {
            path: PATH.to_string(),
            contents,
        };
    }

    contents.push_str("\nimport type { z } from \"zod\";\n");
    contents.push_str("\nimport type { RunContext } from \"./runtime.ts\";\n");

    let mut schemas: Vec<String> = Vec::new();
    for (address, _) in &bindings {
        schemas.push(names.value(&format!("{address}.input")).to_string());
        schemas.push(names.value(&format!("{address}.output")).to_string());
    }
    schemas.sort();
    schemas.dedup();
    contents.push_str("import type {\n");
    for schema in &schemas {
        contents.push_str(&format!("  {schema},\n"));
    }
    contents.push_str("} from \"./schemas.ts\";\n");

    // The one place generated code imports authored code. A **default** import,
    // which is what the scaffold writes and what the contract below is checked
    // against; the local is a name of the project's own namespace so that no
    // authored file can shadow anything this module already holds.
    contents.push('\n');
    for (address, binding) in &bindings {
        contents.push_str(&format!(
            "import {} from {};\n",
            names.value(&format!("{address}.module.implementation")),
            names::string(&authored::relative(PATH, &binding.path.value))
        ));
    }

    for (address, binding) in &bindings {
        contents.push_str(&contract(ir, names, address, binding));
    }

    GeneratedFile {
        path: PATH.to_string(),
        contents,
    }
}

/// One binding's type and its typed `const`.
fn contract(ir: &Ir, names: &Names, address: &str, binding: &Module) -> String {
    let tool = match ir.definition(address).map(|definition| &definition.body) {
        Some(crate::ir::definition::DefinitionBody::Tool(tool)) => tool,
        _ => unreachable!("`bindings` yields tool definitions"),
    };
    let input = names.value(&format!("{address}.input"));
    let output = names.value(&format!("{address}.output"));
    let value = names.value(&format!("{address}.module"));
    let ty = names.ty(&format!("{address}.module"));
    let environment = names.ty(&format!("{address}.module.env"));
    let implementation = names.value(&format!("{address}.module.implementation"));
    let path = binding.path.value.as_str();

    let mut text = String::from("\n");
    text.push_str(&environment_type(binding, address, &environment));

    let mut doc: Vec<String> = vec![
        format!("`{address}` — {}", tool.description.value),
        String::new(),
        format!(
            "The contract `{path}` satisfies, derived from the tool's own `input:` and \
             `output:` (grammar 6.1). Change either schema and the authored file stops \
             type-checking, naming the field that moved — which is the merge tool PRD resolved \
             q48 chooses over marker comments."
        ),
        String::new(),
    ];
    doc.push("Takes:".to_string());
    doc.extend(authored::fields(&tool.input, "it takes no arguments"));
    doc.push("Answers:".to_string());
    doc.extend(authored::fields(&tool.output, "it answers nothing"));
    doc.push(format!(
        "Reads: `{environment}` — the third argument, never `process.env`."
    ));

    text.push('\n');
    text.push_str(&names::doc("", &doc));
    text.push_str(&format!(
        "export type {ty} = (\n  input: z.infer<typeof {input}>,\n  context: RunContext,\n  env: \
         {environment},\n) => z.infer<typeof {output}> | Promise<z.infer<typeof {output}>>;\n"
    ));
    text.push('\n');
    text.push_str(&names::doc(
        "",
        &[format!(
            "`{address}`, as `{path}` implements it. The annotation is the check: an authored \
             default export the contract does not admit fails here, naming both."
        )],
    ));
    text.push_str(&format!("export const {value}: {ty} = {implementation};\n"));
    text
}

/// One binding's declared environment, as a type.
///
/// The names the `env:` map wrote out and no others, because the values reach
/// the implementation as an **argument** (`runtime.ModuleEnv`) rather than
/// through `process.env`: a read of a variable the composition did not declare
/// is a type error here instead of an `undefined` at run time, and the
/// declaration is the same one that puts the variable on the manifest of every
/// process that can execute the tool (`docs/distributed.md` §9.1). A binding
/// that declares nothing gets a type that admits nothing, which says the same
/// thing about the empty case.
fn environment_type(binding: &Module, address: &str, name: &str) -> String {
    let mut doc: Vec<String> = vec![
        format!("`{address}`'s declared environment (grammar 6.1, 4.3 class 2)."),
        String::new(),
    ];
    if binding.env.is_empty() {
        doc.push(
            "This binding declares no `env:`, so its implementation reads none: the type admits \
             no property, and a variable it needs is one the composition declares."
                .to_string(),
        );
        return format!(
            "{}export type {name} = Record<never, string>;\n",
            names::doc("", &doc)
        );
    }
    doc.push(
        "Handed to the implementation as its third argument, resolved at the moment of the call. \
         `process.env` is not where these live: the values belong to this call rather than to the \
         process, exactly as an `exec:` binding's belong to its child."
            .to_string(),
    );
    doc.push(String::new());
    doc.push("Reads:".to_string());
    for entry in &binding.env {
        doc.push(format!(
            "  `{}` — `{}`",
            entry.name.value,
            entry.value.value.as_str()
        ));
    }
    let mut text = names::doc("", &doc);
    text.push_str(&format!("export type {name} = {{\n"));
    for entry in &binding.env {
        text.push_str(&format!("  readonly {}: string;\n", entry.name.value));
    }
    text.push_str("};\n");
    text
}

const MODULE_DOC: &str = "\
//
// The generated half of every `module:` binding (grammar 6.1, PRD resolved q48).
//
// One exported type per module-bound tool — its contract, written from the
// tool's own `input:` and `output:` — the environment its binding declared
// beside it, and one typed `const` holding the authored implementation.
// `src/graph.ts` dispatches through those consts.
//
// This is the **only** generated module that imports authored code. The rule
// runs one way: authored files may import anything this project generates, and
// generated code reaches them here or nowhere.
";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    const TWO_MODULE_TOOLS: &str = r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts

tool.verify:
  description: Verify a signature.
  input:
    payload: { type: string }
    signature: { type: string }
  output:
    ok: { type: boolean }
  module:
    path: ./src/tools/verify.ts
    env:
      SIGNING_KEY: "${SIGNING_KEY}"
"#;

    /// The same two tools, with one binding **outside** the conventional zone.
    ///
    /// `AUTHORED_ZONE` is a convention rather than a rule, so the composition a
    /// project-wide claim about authored code is tested over has to contain the
    /// case a directory-name heuristic would miss.
    const ONE_BINDING_OUTSIDE_THE_ZONE: &str = r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts

tool.verify:
  description: Verify a signature.
  input:
    payload: { type: string }
    signature: { type: string }
  output:
    ok: { type: boolean }
  module:
    path: ./lib/verify.ts
    env:
      SIGNING_KEY: "${SIGNING_KEY}"
"#;

    fn emitted(source: &str) -> String {
        let ir = ir_of(source);
        module(&ir, &crate::codegen::registry(&ir)).contents
    }

    /// The contract and the seam, over a composition binding two files.
    #[test]
    fn each_binding_gets_a_type_and_a_typed_const_holding_its_import() {
        let text = emitted(TWO_MODULE_TOOLS);
        assert!(
            text.contains("import toolSignModuleImplementation from \"./tools/sign.ts\";"),
            "{text}"
        );
        assert!(
            text.contains("import toolVerifyModuleImplementation from \"./tools/verify.ts\";"),
            "{text}"
        );
        assert!(
            text.contains(
                "export type ToolSignModule = (\n  input: z.infer<typeof toolSignInput>,\n  \
                 context: RunContext,\n  env: ToolSignModuleEnv,\n) => z.infer<typeof \
                 toolSignOutput> | Promise<z.infer<typeof toolSignOutput>>;"
            ),
            "{text}"
        );
        assert!(
            text.contains(
                "export const toolSignModule: ToolSignModule = toolSignModuleImplementation;"
            ),
            "the annotation is what checks the authored file: {text}"
        );
        assert!(
            text.contains("`payload` — string"),
            "the contract names the fields: {text}"
        );
    }

    /// The declared environment is a **type**, per binding, and it is the third
    /// argument rather than this process's own environment (PRD resolved q49).
    ///
    /// Two claims, because the empty case is the one that could be written as
    /// "anything goes" without anybody noticing: a binding that declared
    /// `SIGNING_KEY` admits that name and a binding that declared nothing admits
    /// none, so a `process.env`-shaped read of an undeclared variable is a type
    /// error in the authored file rather than an `undefined` at run time.
    #[test]
    fn a_bindings_declared_environment_is_a_type_of_its_own() {
        let text = emitted(TWO_MODULE_TOOLS);
        assert!(
            text.contains(
                "export type ToolVerifyModuleEnv = {\n  readonly SIGNING_KEY: string;\n};"
            ),
            "{text}"
        );
        assert!(
            text.contains("`SIGNING_KEY` — `${SIGNING_KEY}`"),
            "the type's doc names the reference as written: {text}"
        );
        assert!(
            text.contains("export type ToolSignModuleEnv = Record<never, string>;"),
            "a binding that declares nothing admits nothing: {text}"
        );
        assert!(
            text.contains("  env: ToolVerifyModuleEnv,\n"),
            "the contract takes it as an argument: {text}"
        );
    }

    /// The seam is the only import of authored code, and it never reaches one
    /// with anything but a default import.
    #[test]
    fn the_authored_import_is_a_default_import_of_the_declared_path() {
        let text = emitted(TWO_MODULE_TOOLS);
        let authored: Vec<&str> = text
            .lines()
            .filter(|line| line.contains("./tools/"))
            .collect();
        assert_eq!(
            authored,
            [
                "import toolSignModuleImplementation from \"./tools/sign.ts\";",
                "import toolVerifyModuleImplementation from \"./tools/verify.ts\";",
            ],
            "{text}"
        );
    }

    /// A composition with no binding still emits the module, because the file
    /// list is a constant of the compiler (PRD resolved q47).
    #[test]
    fn a_composition_that_binds_nothing_still_emits_a_module() {
        let text = emitted("version: \"0.1\"\n");
        assert!(text.contains("generated by agent-compose"), "{text}");
        assert!(
            text.contains("export {};"),
            "`isolatedModules` needs a file to be a module: {text}"
        );
        assert!(
            !text.lines().any(|line| line.starts_with("import ")),
            "there is nothing to import: {text}"
        );
    }

    /// The import direction, held over the whole emitted project: this module
    /// reaches authored code and nothing else does (D132).
    ///
    /// Stated over every generated file rather than over `src/graph.ts`, because
    /// the rule is about the project: a future emitter that reached a
    /// `./tools/*.ts` from `src/worker-node.ts` or from the barrel would break
    /// the one property that makes "generated code and authored code in one
    /// tree" a fixed shape rather than a growing surface.
    ///
    /// What counts as "reaches authored code" is read off the project's own
    /// **carried** paths rather than off a directory name. `src/tools/` is a
    /// convention (see [`super::AUTHORED_ZONE`]) and a binding may name
    /// `./lib/sign.ts`, so a predicate spelled `contains("tools/")` would be
    /// blind to exactly the import this rule exists to forbid.
    ///
    /// And it is the **quoted specifier anywhere in the file**, not a line that
    /// starts with `import `. Every way one module can name another ends in that
    /// string: a single-line import, the multi-line form this emitter already
    /// writes for more than one name (`graph.rs` writes
    /// `import {\n  toolStampModule,\n} from "./modules.ts";`, whose specifier
    /// sits on a line beginning with `}`), an `export … from`, a side-effect
    /// import, a dynamic `import(…)`. A line-shaped predicate would pass a
    /// project that broke the rule in the emitter's own house style, which is
    /// the one blind spot a test of a *direction* cannot afford.
    #[test]
    fn the_only_generated_import_of_authored_code_is_the_seam() {
        let ir = ir_of(ONE_BINDING_OUTSIDE_THE_ZONE);
        let project = super::super::emit(
            &ir,
            &crate::codegen::authored::Authored::of([
                ("src/tools/sign.ts".to_string(), "// yours\n".to_string()),
                ("lib/verify.ts".to_string(), "// yours\n".to_string()),
            ]),
        );
        // Every carried file, as the specifier a generated module would have to
        // write to reach it: `../lib/verify.ts` and `./tools/sign.ts` from
        // `src/`, which is where every generated module sits.
        let specifiers: Vec<String> = project
            .carried()
            .iter()
            .map(|file| authored::relative(PATH, &file.path))
            .collect();
        assert_eq!(specifiers.len(), 2, "both bindings are carried");
        for file in project.files() {
            let reaches = specifiers
                .iter()
                .any(|specifier| file.contents.contains(&format!("\"{specifier}\"")));
            assert_eq!(
                reaches,
                file.path == PATH,
                "`{}` and the seam disagree about who may import authored code",
                file.path
            );
        }
        // …and `src/graph.ts` reaches them the way it is supposed to: by name,
        // off the seam.
        let graph = project
            .file("src/graph.ts")
            .expect("a graph module")
            .contents
            .as_str();
        assert!(
            graph.contains("  toolSignModule,\n  toolVerifyModule,\n} from \"./modules.ts\";"),
            "{graph}"
        );
    }

    /// A path outside `src/` is a specifier that climbs, and the seam writes it
    /// rather than assuming the conventional zone.
    #[test]
    fn a_binding_outside_the_conventional_zone_is_imported_where_it_is() {
        let text = emitted(
            r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input: {}
  output: {}
  module: ./lib/sign.ts
"#,
        );
        assert!(
            text.contains("import toolSignModuleImplementation from \"../lib/sign.ts\";"),
            "{text}"
        );
        assert!(text.contains("it takes no arguments"), "{text}");
        assert!(text.contains("it answers nothing"), "{text}");
    }
}
