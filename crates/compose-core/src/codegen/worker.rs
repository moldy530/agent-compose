//! `src/worker-node.ts` and `manifest.json`: the two things a **worker** reads
//! out of an artifact (`docs/distributed.md` §3.2, §4, §9.1).
//!
//! [`super::mesh`] emits the hub; this emits the other end of the same wire. It
//! is two files because a worker is two programs meeting at the artifact: the
//! Bun runner that executes one dispatch, and the `agent-compose worker` process
//! that speaks the protocol around it.
//!
//! # `src/worker-node.ts`
//!
//! A **constant** of the compiler release, for [`super::runtime`]'s reasons: the
//! node runner is the same program in every project, and what differs between
//! two compositions is the `placedNodes` registry `src/graph.ts` exports for it.
//! Its stdin/stdout contract — one dispatch in, NDJSON effect lines and one
//! result line out — is documented in the emitted file's own header, where a
//! reader of a generated project finds it.
//!
//! # `manifest.json`
//!
//! What the **Rust** half of a worker has to read, in a format it can read
//! without a JavaScript runtime: the per-placement environment manifest §9.1
//! partitions, the path of the runner above, and which files of the tree the
//! compiler did not write.
//!
//! That last list is PRD resolved q47 read where a reader without a JavaScript
//! runtime is standing. "The manifest is the boundary" is a statement about
//! *ownership*, and the tree stopped being wholly compiler-owned the moment a
//! `module:` binding put authored TypeScript in it (resolved q49). `manifest.json`
//! is the one file in an artifact that answers questions about the artifact in
//! plain JSON, so it is where "these entries are the author's" is written down.
//! It is **not** a second file list to check against: `build --check` compares
//! the build's own two lists and the artifact hash covers the tree, and those
//! remain the only two notions of file identity there are.
//!
//! It is the same partition [`super::deployment`] writes into `src/deployment.ts`
//! and is derived from the same [`super::env::Partition`] — §9.1's "one partition
//! is emitted into the generated project, so the hub reading it and a worker
//! reading it are reading one answer under one artifact hash" is a statement
//! about the derivation, and two files of one derivation keep it. The TypeScript
//! module is what the *hub* checks a join's `env_ok` against; this is what the
//! worker computes that report from, after it has unpacked the tree and before
//! it can join a second time (§3.1, §4 step 5). A worker that had to load a
//! TypeScript module to learn which variables to report would need the runtime
//! it is about to be told it may not have.
//!
//! The header goes in a `"//"` key, which is [`super::project::package_json`]'s
//! own answer to a format with no comments — and it is what marks the file as
//! this compiler's for `build --check` (`build::GENERATED_MARKER`).

use crate::ir::Ir;

use super::{GeneratedFile, env, names};

/// The node runner's source, carried in the compiler and emitted verbatim.
const SOURCE: &str = include_str!("js/worker-node.ts");

/// Where the runner sits in an emitted project — the entry a worker spawns.
pub const RUNNER: &str = "src/worker-node.ts";

/// `src/worker-node.ts`.
#[must_use]
pub fn module(ir: &Ir) -> GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    GeneratedFile {
        path: RUNNER.to_string(),
        contents,
    }
}

/// `manifest.json`.
#[must_use]
pub fn manifest(ir: &Ir, partition: &env::Partition) -> GeneratedFile {
    let mut contents = String::from("{\n  \"//\": [\n");
    let header = super::header_lines(ir);
    for (index, line) in header.iter().enumerate() {
        let comma = if index + 1 == header.len() { "" } else { "," };
        contents.push_str(&format!("    {}{comma}\n", names::string(line)));
    }
    contents.push_str("  ],\n");
    contents.push_str(&format!("  \"node_runner\": {},\n", names::string(RUNNER)));
    // The authored half of the tree, by the path it takes inside the artifact —
    // sorted and de-duplicated, which is what one entry per *file* rather than
    // per binding means. Emitted as `[]` rather than omitted for the reason the
    // empty `placements` array is: "this artifact carries no authored code" and
    // "this manifest does not say" must not be the same answer.
    let mut authored: Vec<&str> = crate::check::modules::bindings(ir)
        .into_iter()
        .map(|(_, module)| module.path.value.as_str())
        .collect();
    authored.sort_unstable();
    authored.dedup();
    contents.push_str("  \"authored\": [");
    if authored.is_empty() {
        contents.push_str("],\n");
    } else {
        contents.push('\n');
        for (index, path) in authored.iter().enumerate() {
            let comma = if index + 1 == authored.len() { "" } else { "," };
            contents.push_str(&format!("    {}{comma}\n", names::string(path)));
        }
        contents.push_str("  ],\n");
    }
    contents.push_str("  \"placements\": [\n");
    let placements: Vec<&env::Process> = partition
        .processes()
        .filter(|process| !matches!(process, env::Process::Hub))
        .collect();
    for (index, process) in placements.iter().enumerate() {
        let variables = env::References::for_process(ir, partition, process);
        contents.push_str("    {\n");
        contents.push_str(&format!(
            "      \"name\": {},\n",
            names::string(process.name())
        ));
        contents.push_str("      \"environment\": [");
        let listed: Vec<String> = variables.names().map(names::string).collect();
        if listed.is_empty() {
            contents.push_str("]\n");
        } else {
            contents.push('\n');
            for (position, written) in listed.iter().enumerate() {
                let comma = if position + 1 == listed.len() {
                    ""
                } else {
                    ","
                };
                contents.push_str(&format!("        {written}{comma}\n"));
            }
            contents.push_str("      ]\n");
        }
        let comma = if index + 1 == placements.len() {
            ""
        } else {
            ","
        };
        contents.push_str(&format!("    }}{comma}\n"));
    }
    contents.push_str("  ]\n}\n");
    GeneratedFile {
        path: "manifest.json".to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn the_runner_is_emitted_verbatim_under_the_header() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.starts_with("// This file was generated by agent-compose"));
        assert!(emitted.ends_with(SOURCE), "the runner is emitted verbatim");
    }

    /// A composition with no placements still carries a manifest, and it is
    /// empty rather than absent.
    ///
    /// The worker reads this file to learn what to report at join (§3.1), and a
    /// file that appeared only for a mesh would make "the artifact does not
    /// describe its placements" and "this artifact has none" the same answer.
    #[test]
    fn a_composition_with_no_placements_carries_an_empty_manifest() {
        let ir = ir_of("version: \"0.1\"\n");
        let partition = env::Partition::of(&ir);
        let written = manifest(&ir, &partition).contents;
        assert!(written.contains("generated by agent-compose"), "{written}");
        assert!(written.contains("\"placements\": [\n  ]\n"), "{written}");
        assert!(
            written.contains(&format!("\"node_runner\": \"{RUNNER}\"")),
            "{written}"
        );
        assert!(written.contains("\"authored\": [],\n"), "{written}");
    }

    /// The boundary, written where a reader with no JavaScript runtime finds it
    /// (PRD resolved q47, q49): one entry per authored file the artifact
    /// carries, in the path space the tree uses.
    #[test]
    fn the_manifest_names_the_authored_files_the_artifact_carries() {
        let ir = ir_of(
            r#"version: "0.1"

tool.verify:
  description: Verify a signature.
  input: {}
  output: {}
  module: ./src/tools/verify.ts

tool.sign:
  description: Sign a payload.
  input: {}
  output: {}
  module: ./src/tools/sign.ts
"#,
        );
        let partition = env::Partition::of(&ir);
        let written = manifest(&ir, &partition).contents;
        assert!(
            written.contains(
                "  \"authored\": [\n    \"src/tools/sign.ts\",\n    \"src/tools/verify.ts\"\n  ],\n"
            ),
            "{written}"
        );

        // …and it is the **tree's** answer rather than a second one. The array
        // is derived from the bindings and the carried set is derived from the
        // same place, so today they cannot disagree; asserting it is what keeps
        // that true, because nothing reads this array back — a manifest field a
        // reader trusts and no code consumes is exactly the kind that goes
        // stale without failing anything.
        let project = crate::codegen::emit(
            &ir,
            &crate::codegen::authored::Authored::of([
                ("src/tools/sign.ts".to_string(), "// yours\n".to_string()),
                ("src/tools/verify.ts".to_string(), "// yours\n".to_string()),
            ]),
        );
        let listed: Vec<&str> = written
            .lines()
            .skip_while(|line| !line.starts_with("  \"authored\": ["))
            .skip(1)
            .take_while(|line| !line.starts_with("  ],"))
            .map(|line| line.trim().trim_end_matches(',').trim_matches('"'))
            .collect();
        assert_eq!(
            listed,
            project
                .carried()
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            "the manifest and the tree disagree about which files are the author's"
        );
    }
}
