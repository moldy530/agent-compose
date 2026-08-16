//! `src/graph.ts`: the module the compiled graph is assembled in.
//!
//! # What is here, and what is not
//!
//! Node functions, routers with embedded CEL, per-cycle iteration counters,
//! `map`→`Send` dispatch, subgraphs, store ops and model routing are the
//! remaining bullets of PRD §7 M1. None of them is emitted yet, and none is
//! stubbed: a `StateGraph` with invented nodes would type-check, construct, and
//! be wrong, which is worse than absent.
//!
//! What the module does emit is the **seam** — a builder over the state model of
//! `./state.ts`. That is not decoration. LangGraph is the one consumer whose
//! opinion of the state layer matters (PRD §4: LangGraph owns execution), and
//! `new StateGraph(State)` is what asks it: a channel spec it refuses is a type
//! error here and a construction failure at run time, in the same module the
//! later bullets extend. The state layer is therefore checked against the pinned
//! LangGraph release from this PR onward rather than from the PR that finally
//! adds a node.

use crate::ir::Ir;
use crate::ir::definition::DefinitionBody;

/// `src/graph.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);

    let flows: Vec<&String> = ir
        .definitions
        .iter()
        .filter(|(_, definition)| matches!(definition.body, DefinitionBody::Flow(_)))
        .map(|(address, _)| address)
        .collect();
    contents.push_str("//\n// The flows this composition declares, which are what will be\n");
    contents.push_str("// assembled here:\n");
    if flows.is_empty() {
        contents.push_str("//   (none)\n");
    } else {
        for address in flows {
            contents.push_str(&format!("//   {address}\n"));
        }
    }

    contents.push_str(BODY);

    super::GeneratedFile {
        path: "src/graph.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// The compiled graph.
//
// Node functions, routers, bounded-cycle counters, `map` dispatch and subgraphs
// are assembled onto the state model here. What this module holds today is the
// builder they are added to, constructed from `./state.ts` so the state model is
// checked against the pinned LangGraph release rather than merely written for it.
";

const BODY: &str = r#"
import { StateGraph } from "@langchain/langgraph";

import { State } from "./state.ts";

/**
 * A new builder over this composition's state model.
 *
 * Every flow's topology is added to one of these. Constructing it is also what
 * proves the state model is a shape LangGraph accepts — a channel spec it
 * refuses fails here rather than at the first invocation.
 */
export function createBuilder() {
  return new StateGraph(State);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn the_module_names_the_flows_it_will_assemble() {
        let ir = ir_of(
            "version: \"0.1\"\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
agent.a:\n  model: model.m\n  prompt: p\n  output: { verdict: { enum: [ok] } }\n\
flow.one:\n  outputs: {}\n  nodes:\n    a: { agent: agent.a, input: \"'x'\" }\n  edges:\n    - { from: start, to: a }\n    - { from: a, to: end }\n\
flow.two:\n  outputs: {}\n  nodes:\n    a: { agent: agent.a, input: \"'x'\" }\n  edges:\n    - { from: start, to: a }\n    - { from: a, to: end }\n",
        );
        let contents = module(&ir).contents;
        assert!(contents.contains("//   flow.one\n"), "{contents}");
        assert!(contents.contains("//   flow.two\n"), "{contents}");
        assert!(
            contents.contains("export function createBuilder()"),
            "{contents}"
        );
    }

    #[test]
    fn a_composition_with_no_flows_says_so() {
        let contents = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(contents.contains("//   (none)\n"), "{contents}");
    }
}
