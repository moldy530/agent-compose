//! `src/env.ts`: every `${ENV}` reference, and the check that runs at process
//! start (PRD 5.9, grammar 4.3).
//!
//! # Why the compiler never reads the environment
//!
//! Env refs survive **unresolved** into the IR and into generated code: PRD 5.9
//! is explicit that "resolution happens at process start in generated code, never
//! at compile", which is what keeps the artifact committable and keeps a key out
//! of every file this compiler writes. `build` therefore emits the checker and
//! does not run it — a build whose success depended on the building machine's
//! environment could not be reproduced on the deploying one, and the acceptance
//! harness relies on exactly that (it seals the environment around `run` and
//! `serve`, and leaves `validate` and `build` alone).
//!
//! # What the module gives the rest of the project
//!
//! * `environmentReferences` — every name the composition references, sorted,
//!   each with the surfaces that wrote it. That list is also PRD 5.9's
//!   least-privilege input: the set of variables an isolated deployment needs is
//!   computable from it, statically, because every secret is a named ref.
//! * `readEnvironment()` — resolves all of them at once and throws naming
//!   **every** missing variable rather than the first, because a deploy that is
//!   missing three keys should learn that in one run.
//!
//! Presence is `!== undefined`: a variable that is set to the empty string is
//! set. `process.env` cannot tell an empty assignment from an intentional one,
//! and a compiler that guessed would refuse a legitimately empty value with a
//! message about it being absent.

use std::collections::BTreeMap;

use crate::ast::common::Interpolated;
use crate::ast::deploy::PluginValue;
use crate::ir::Ir;
use crate::ir::binding::{Exec, Http, InterpolatedEntry};
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::{NodeKind, ToolImplementation};

use super::names;

/// Every environment reference the composition makes, sorted by variable name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct References {
    /// Variable name to the surfaces that reference it, in discovery order.
    entries: BTreeMap<String, Vec<String>>,
}

impl References {
    /// Collect every reference in the composition.
    ///
    /// The walk is over the IR in its own canonical order — definitions by
    /// address, then the deploy layer — so the sites recorded for a variable are
    /// in a deterministic order without being sorted, and read in the order a
    /// person would find them.
    #[must_use]
    pub fn of(ir: &Ir) -> Self {
        let mut references = Self::default();
        for (address, definition) in &ir.definitions {
            match &definition.body {
                DefinitionBody::Provider(provider) => {
                    let config = &provider.config;
                    for (key, value) in [
                        ("api_key", config.api_key.as_ref()),
                        ("base_url", config.base_url.as_ref()),
                        ("access_key_id", config.access_key_id.as_ref()),
                        ("secret_access_key", config.secret_access_key.as_ref()),
                        ("session_token", config.session_token.as_ref()),
                        ("credentials_json", config.credentials_json.as_ref()),
                    ] {
                        if let Some(reference) = value {
                            references.record(&reference.value.name, &format!("{address}.{key}"));
                        }
                    }
                    for (key, value) in [
                        ("api_version", config.api_version.as_ref()),
                        ("organization", config.organization.as_ref()),
                        ("region", config.region.as_ref()),
                        ("location", config.location.as_ref()),
                        ("project", config.project.as_ref()),
                        ("profile", config.profile.as_ref()),
                    ] {
                        if let Some(text) = value {
                            references.text(&text.value, &format!("{address}.{key}"));
                        }
                    }
                    references.entries_of(&config.headers, &format!("{address}.headers"));
                }
                DefinitionBody::Tool(tool) => match &tool.implementation {
                    ToolImplementation::Exec { exec } => {
                        references.exec(exec, &format!("{address}.exec"));
                    }
                    ToolImplementation::Http { http } => {
                        references.http(http, &format!("{address}.http"));
                    }
                    ToolImplementation::Function { .. } => {}
                },
                DefinitionBody::Flow(flow) => {
                    for node in &flow.nodes {
                        let id = node.id.value.as_str();
                        match &node.kind {
                            NodeKind::Exec { exec } => {
                                references.exec(exec, &format!("{address}.node.{id}.exec"));
                            }
                            NodeKind::Http { http } => {
                                references.http(http, &format!("{address}.node.{id}.http"));
                            }
                            NodeKind::Agent { .. }
                            | NodeKind::Function { .. }
                            | NodeKind::Flow { .. }
                            | NodeKind::Map { .. }
                            | NodeKind::Human { .. }
                            | NodeKind::Store { .. } => {}
                        }
                    }
                }
                DefinitionBody::Agent(_) | DefinitionBody::Store(_) | DefinitionBody::Model(_) => {}
            }
        }

        if let Some(backends) = &ir.deploy.storage_backends {
            for (kind, config) in &backends.defaults {
                references.backend(config, &format!("deploy.storage_backends.defaults.{kind}"));
            }
            for (alias, config) in &backends.aliases {
                references.backend(config, &format!("deploy.storage_backends.aliases.{alias}"));
            }
        }
        if let Some(sources) = &ir.deploy.event_sources {
            for (name, source) in &sources.entries {
                let at = format!("deploy.event_sources.{name}");
                for (key, reference) in &source.connection {
                    references.record(&reference.value.name, &format!("{at}.{key}"));
                }
                for (key, value) in &source.extra {
                    references.plugin(&value.value, &format!("{at}.{key}"));
                }
            }
        }

        references
    }

    /// Whether the composition references nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The variable names, sorted.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Where a variable is referenced.
    #[must_use]
    pub fn sites(&self, name: &str) -> &[String] {
        self.entries.get(name).map_or(&[], Vec::as_slice)
    }

    fn record(&mut self, name: &str, site: &str) {
        let sites = self.entries.entry(name.to_string()).or_default();
        if !sites.iter().any(|existing| existing == site) {
            sites.push(site.to_string());
        }
    }

    /// An interpolable string contributes every name it really carries — which
    /// is the list the IR records, not a scan of the text: an escaped `$${NAME}`
    /// is text and names nothing (grammar 4.3).
    fn text(&mut self, text: &Interpolated, site: &str) {
        for name in &text.references {
            self.record(name, site);
        }
    }

    fn entries_of(&mut self, entries: &[InterpolatedEntry], site: &str) {
        for entry in entries {
            self.text(&entry.value.value, &format!("{site}.{}", entry.name.value));
        }
    }

    fn exec(&mut self, exec: &Exec, site: &str) {
        self.text(&exec.command.value, &format!("{site}.command"));
        for (index, argument) in exec.args.iter().enumerate() {
            self.text(&argument.value, &format!("{site}.args[{index}]"));
        }
        if let Some(cwd) = &exec.cwd {
            self.text(&cwd.value, &format!("{site}.cwd"));
        }
        self.entries_of(&exec.env, &format!("{site}.env"));
    }

    fn http(&mut self, http: &Http, site: &str) {
        self.text(&http.url.value, &format!("{site}.url"));
        self.entries_of(&http.headers, &format!("{site}.headers"));
    }

    fn backend(&mut self, config: &crate::ir::deploy::BackendConfig, site: &str) {
        for (key, reference) in &config.connection {
            self.record(&reference.value.name, &format!("{site}.{key}"));
        }
        for (key, value) in &config.extra {
            self.plugin(&value.value, &format!("{site}.{key}"));
        }
    }

    fn plugin(&mut self, value: &PluginValue, site: &str) {
        match value {
            PluginValue::Text(text) => self.text(text, site),
            PluginValue::Sequence(items) => {
                for (index, item) in items.iter().enumerate() {
                    self.plugin(&item.value, &format!("{site}[{index}]"));
                }
            }
            PluginValue::Mapping(entries) => {
                for entry in entries {
                    self.plugin(&entry.value.value, &format!("{site}.{}", entry.key.value));
                }
            }
            PluginValue::Null
            | PluginValue::Bool(_)
            | PluginValue::Int(_)
            | PluginValue::Float(_) => {}
        }
    }
}

/// `src/env.ts`.
#[must_use]
pub fn module(ir: &Ir, references: &References) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents.push_str("\nimport process from \"node:process\";\n");
    contents.push_str(DECLARATIONS);

    if references.is_empty() {
        contents.push_str(
            "export const environmentReferences: readonly EnvironmentReference[] = [];\n",
        );
    } else {
        contents
            .push_str("export const environmentReferences: readonly EnvironmentReference[] = [\n");
        for name in references.names() {
            contents.push_str(&format!(
                "  {{\n    name: {},\n    sites: [\n",
                names::string(name)
            ));
            for site in references.sites(name) {
                contents.push_str(&format!("      {},\n", names::string(site)));
            }
            contents.push_str("    ],\n  },\n");
        }
        contents.push_str("];\n");
    }

    contents.push_str(CHECKER);

    super::GeneratedFile {
        path: "src/env.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// Every `${ENV}` reference the composition makes, and the presence check that
// runs before the graph does (PRD 5.9, grammar 4.3). No value is baked in here:
// the spec never contains a credential, and neither does this file.
";

const DECLARATIONS: &str = r#"
/** One `${ENV}` reference, and the surfaces that wrote it. */
export interface EnvironmentReference {
  /** The variable name, between the braces. */
  readonly name: string;
  /** Where it is referenced, as spec addresses. */
  readonly sites: readonly string[];
}

/**
 * Every reference this composition makes, sorted by name.
 *
 * This is also the least-privilege list of PRD 5.9: a deployment of this graph
 * needs these variables and no others.
 */
"#;

const CHECKER: &str = r#"
/** Raised when the process starts without a variable the composition needs. */
export class MissingEnvironmentError extends Error {
  /** Every variable that was missing, sorted by name. */
  readonly missing: readonly EnvironmentReference[];

  constructor(missing: readonly EnvironmentReference[]) {
    const detail = missing
      .map((reference) => `${reference.name} (referenced by ${reference.sites.join(", ")})`)
      .join("; ");
    super(`missing environment ${missing.length === 1 ? "variable" : "variables"}: ${detail}`);
    this.name = "MissingEnvironmentError";
    this.missing = missing;
  }
}

/**
 * Resolve every reference at process start, or throw naming all of them.
 *
 * Presence is `!== undefined`: a variable set to the empty string is set. All
 * missing variables are reported at once, because a deployment that is short
 * three keys should learn that in one run rather than in three.
 */
export function readEnvironment(
  source: Record<string, string | undefined> = process.env,
): ReadonlyMap<string, string> {
  const resolved = new Map<string, string>();
  const missing: EnvironmentReference[] = [];
  for (const reference of environmentReferences) {
    const value = source[reference.name];
    if (value === undefined) {
      missing.push(reference);
    } else {
      resolved.set(reference.name, value);
    }
  }
  if (missing.length > 0) {
    throw new MissingEnvironmentError(missing);
  }
  return resolved;
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    const PROJECT: &str = "version: \"0.1\"\n\
provider.p:\n  kind: openai_compatible\n  api_key: ${LLM_KEY}\n  base_url: ${LLM_URL}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
tool.search:\n  description: Search.\n  input: { q: { type: string } }\n  output: { hits: { type: integer } }\n  http:\n    method: GET\n    url: \"https://${SEARCH_HOST}/v1\"\n    headers:\n      authorization: \"Bearer ${LLM_KEY}\"\n";

    #[test]
    fn every_reference_is_collected_with_where_it_was_written() {
        let references = References::of(&ir_of(PROJECT));
        assert_eq!(
            references.names().collect::<Vec<_>>(),
            ["LLM_KEY", "LLM_URL", "SEARCH_HOST"]
        );
        assert_eq!(
            references.sites("LLM_KEY"),
            [
                "provider.p.api_key",
                "tool.search.http.headers.authorization"
            ]
        );
        assert_eq!(references.sites("SEARCH_HOST"), ["tool.search.http.url"]);
    }

    /// Grammar 4.3: `$${NAME}` is an escape, and the IR's own reference list —
    /// not a scan of the text — is what says which tokens are real.
    #[test]
    fn an_escaped_token_references_nothing() {
        let references = References::of(&ir_of(
            "version: \"0.1\"\n\
tool.t:\n  description: A tool.\n  input: {}\n  output: {}\n  exec:\n    command: echo\n    args: [\"$${NOT_A_REF}\"]\n",
        ));
        assert!(references.is_empty(), "{references:?}");
    }

    #[test]
    fn a_composition_referencing_nothing_still_emits_the_checker() {
        let ir = ir_of("version: \"0.1\"\n");
        let emitted = module(&ir, &References::of(&ir)).contents;
        assert!(emitted.contains("environmentReferences: readonly EnvironmentReference[] = [];"));
        assert!(emitted.contains("export function readEnvironment("));
    }

    #[test]
    fn the_emitted_list_carries_names_and_sites() {
        let ir = ir_of(PROJECT);
        let emitted = module(&ir, &References::of(&ir)).contents;
        assert!(emitted.contains("    name: \"LLM_KEY\",\n"), "{emitted}");
        assert!(
            emitted.contains("      \"tool.search.http.headers.authorization\",\n"),
            "{emitted}"
        );
        assert!(
            !emitted.contains("${LLM_KEY}"),
            "no reference is resolved or reproduced verbatim: {emitted}"
        );
    }
}
