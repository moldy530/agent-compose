//! Turning composition names into TypeScript ones, and data into TypeScript
//! literals.
//!
//! # Why this is not a one-line `to_camel_case`
//!
//! Grammar 2.1's identifier is `lower , { lower | digit | "_" }`, which admits
//! `a_1` and `a1`, and `foo_bar` and `foo__bar`. Camel-casing is not injective
//! over that set — `a_1` and `a1` both want to be `a1` — so a naive mapping can
//! emit two exports with one name and produce a project that does not compile,
//! for a composition the validator was right to accept.
//!
//! [`Names`] is therefore a **registry** rather than a function: it assigns every
//! export name once, in a canonical order, and disambiguates a collision by
//! appending `_2`, `_3`, … in that order. The common case reads the way a
//! hand-written project would (`agent.reviewer`'s output is `agentReviewerOutput`)
//! and the pathological case is deterministic instead of broken.
//!
//! # Canonical paths
//!
//! Every name is assigned against a **canonical path** — a dotted string naming
//! the surface the way the grammar does:
//!
//! ```text
//! agent.reviewer.output               an agent's result schema
//! tool.web_search.input               a tool's parameter schema
//! flow.review_loop.outputs            a module's result schema
//! flow.triage.node.approve.output     a `human:` node's result schema
//! store.docs.metadata_schema          a store's metadata schema
//! state.draft                         a state channel's type
//! ```
//!
//! The path is what the rest of the emitter passes around, so no module has to
//! know how a name was spelled, and a doc comment can name the surface in the
//! author's own vocabulary rather than in the emitter's.

use std::collections::BTreeMap;

use crate::ast::common::Literal;
use crate::ast::schema::Number;
use crate::ir::Ir;

/// Every module-level TypeScript name the emitted project declares, keyed by the
/// canonical path of what it names.
///
/// One registry across every kind of name, because they share one module-level
/// namespace in the emitted project: two kinds assigning names independently
/// could agree on one spelling, and nothing would catch it until `tsc`.
#[derive(Clone, Debug)]
pub struct Names {
    assigned: BTreeMap<String, String>,
    taken: BTreeMap<String, ()>,
}

/// The names the emitted modules declare or import themselves.
///
/// A generated schema may not take one of these: `src/index.ts` re-exports every
/// module, so two of them exporting one name would be a duplicate export, and an
/// import shadowed by a local `const` would be a redeclaration. Reserving them
/// up front costs nothing and turns a rare, confusing `tsc` failure into a `_2`
/// suffix nobody has to think about.
const RESERVED: &[&str] = &[
    // `src/schemas.ts`: the import, and every helper of
    // `codegen::schema::HELPERS`. The list is the whole table rather than the
    // helpers that looked likely to collide — `the_reserved_list_holds_every_
    // emitted_helper` keeps the two in step, so a helper added there cannot be
    // forgotten here.
    "z",
    "codePoints",
    "uniqueItems",
    "rfc1123Hostname",
    "rfc3339Date",
    "rfc3339Time",
    "rfc3339DateTime",
    "rfc3986Uri",
    "rfc4122Uuid",
    "rfc5321Email",
    // `src/state.ts`
    "Annotation",
    "MessagesAnnotation",
    "channels",
    "State",
    "GraphState",
    "GraphStateUpdate",
    // `src/graph.ts`
    "END",
    "START",
    "StateGraph",
    "isInterrupted",
    "runtime",
    "flows",
    "CompiledFlow",
    "FlowRun",
    "runFlow",
    "createBuilder",
    // `src/env.ts`
    "process",
    "EnvironmentReference",
    "environmentReferences",
    "MissingEnvironmentError",
    "readEnvironment",
];

impl Names {
    /// Assign a name to every surface of this composition.
    #[must_use]
    pub fn of(ir: &Ir) -> Self {
        let mut names = Self {
            assigned: BTreeMap::new(),
            taken: BTreeMap::new(),
        };
        for reserved in RESERVED {
            names.taken.insert((*reserved).to_string(), ());
            names.taken.insert(capitalize(reserved), ());
        }
        for surface in super::schema::surfaces(ir) {
            names.assign(&surface.path);
        }
        names
    }

    /// The exported `const` name for this canonical path.
    ///
    /// # Panics
    ///
    /// Panics on a path that was never assigned. Every caller reaches a path
    /// through the same enumeration that populated the registry, so an unknown
    /// one is an emitter bug and a fallback spelling would hide it behind a
    /// project that does not compile.
    #[must_use]
    pub fn value(&self, path: &str) -> &str {
        self.assigned
            .get(path)
            .unwrap_or_else(|| panic!("no generated name was assigned for `{path}`"))
    }

    /// The exported `type` name for this canonical path: [`Names::value`] with
    /// its first character upper-cased.
    ///
    /// Derived from the value name rather than assigned separately, so the two
    /// cannot drift and the type namespace inherits the value namespace's
    /// uniqueness for free — every value name starts lowercase, so the mapping
    /// is one-to-one.
    #[must_use]
    pub fn ty(&self, path: &str) -> String {
        capitalize(self.value(path))
    }

    /// Assign a name to a path the schema enumeration does not reach.
    ///
    /// `src/graph.ts` declares module-level names of its own — one per provider,
    /// model, agent and tool binding, one per flow builder, one per node
    /// descriptor, one per shape — and they share the emitted project's single
    /// module namespace with the schemas. Declaring them here rather than
    /// deriving them separately is what makes a collision impossible instead of
    /// unlikely: [`Names::assign`]'s `_2` suffix is the same mechanism, over one
    /// table.
    ///
    /// Idempotent, so a path declared twice keeps its first name.
    pub fn declare(&mut self, path: &str) -> &str {
        if !self.assigned.contains_key(path) {
            self.assign(path);
        }
        self.value(path)
    }

    /// Assign one name, disambiguating against everything assigned so far.
    fn assign(&mut self, path: &str) {
        let candidate = camel(path);
        let mut name = candidate.clone();
        let mut ordinal = 2u32;
        while self.taken.contains_key(&name) {
            name = format!("{candidate}_{ordinal}");
            ordinal += 1;
        }
        self.taken.insert(name.clone(), ());
        let previous = self.assigned.insert(path.to_string(), name);
        assert!(previous.is_none(), "`{path}` was assigned a name twice");
    }
}

/// Camel-case a canonical path: `flow.review_loop.outputs` → `flowReviewLoopOutputs`.
///
/// Words are the maximal runs of `[a-z0-9]` between `.` and `_` separators, so
/// empty runs — a trailing underscore, a doubled one — contribute nothing. That
/// is exactly the collapse [`Names`] disambiguates afterwards.
#[must_use]
pub fn camel(path: &str) -> String {
    let mut name = String::with_capacity(path.len());
    for word in path.split(['.', '_']).filter(|word| !word.is_empty()) {
        if name.is_empty() {
            name.push_str(word);
        } else {
            name.push_str(&capitalize(word));
        }
    }
    if name.is_empty() {
        // Unreachable for a path built from identifiers, which begin with a
        // lowercase letter — but a name is a hard requirement of the output, so
        // this answers with one rather than with an empty export.
        name.push_str("schema");
    }
    name
}

/// Upper-case the first character, leaving the rest alone.
#[must_use]
pub fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// A TypeScript string literal, double-quoted and escaped.
///
/// JSON string syntax is a subset of TypeScript's, so the JSON writer is the
/// escaper: it is already in the tree, it is the one this compiler emits the IR
/// with, and it handles the control characters and quotes a description may
/// carry.
///
/// # Panics
///
/// Panics if the JSON writer refuses a `str`, which it cannot: every Rust string
/// is valid UTF-8 and every UTF-8 string is a representable JSON string.
#[must_use]
pub fn string(text: &str) -> String {
    serde_json::to_string(text).expect("a string is representable as JSON")
}

/// A TypeScript number literal.
///
/// The IR's floats are checked for infinity and NaN where they are read
/// (see [`crate::ir`]), so every value here has a finite decimal form, and
/// Rust's shortest round-tripping formatting is a valid TypeScript numeric
/// literal for all of them — `1.0_f64` formats as `1`, which is the same
/// TypeScript value.
#[must_use]
pub fn number(value: &Number) -> String {
    match value {
        Number::Int(int) => int.to_string(),
        Number::Float(float) => float.to_string(),
    }
}

/// A literal from a `default:` or a `settings:` block, as TypeScript source.
///
/// Mapping keys are quoted whatever they look like: a key here is arbitrary text
/// (grammar 3.6's literals are data), and quoting unconditionally is one rule
/// instead of an identifier test that would have to agree with TypeScript's
/// reserved words.
#[must_use]
pub fn literal(value: &Literal) -> String {
    match value {
        Literal::Null => "null".to_string(),
        Literal::Bool(boolean) => boolean.to_string(),
        Literal::Int(int) => int.to_string(),
        Literal::Float(float) => float.to_string(),
        Literal::String(text) => string(text),
        Literal::Sequence(items) => {
            let rendered: Vec<String> = items.iter().map(|item| literal(&item.value)).collect();
            format!("[{}]", rendered.join(", "))
        }
        Literal::Mapping(entries) => {
            if entries.is_empty() {
                return "{}".to_string();
            }
            let rendered: Vec<String> = entries
                .iter()
                .map(|entry| {
                    format!(
                        "{}: {}",
                        string(&entry.key.value),
                        literal(&entry.value.value)
                    )
                })
                .collect();
            format!("{{ {} }}", rendered.join(", "))
        }
    }
}

/// A `/** … */` doc comment at this indentation.
///
/// A `*/` inside the text is written `*\/`: the backslash is ordinary text
/// inside a comment, so the comment survives, and a description that mentions a
/// closing marker does not truncate the file.
#[must_use]
pub fn doc(indent: &str, lines: &[String]) -> String {
    let safe: Vec<String> = lines
        .iter()
        .flat_map(|line| line.split('\n').map(|part| part.replace("*/", "*\\/")))
        .collect();
    if let [single] = safe.as_slice()
        && single.len() + indent.len() <= 76
    {
        return format!("{indent}/** {single} */\n");
    }
    let mut text = format!("{indent}/**\n");
    for line in safe {
        if line.is_empty() {
            text.push_str(&format!("{indent} *\n"));
        } else {
            text.push_str(&format!("{indent} * {line}\n"));
        }
    }
    text.push_str(&format!("{indent} */\n"));
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;
    use crate::diag::Spanned;

    #[test]
    fn a_path_camel_cases_segment_by_segment() {
        assert_eq!(camel("agent.reviewer.output"), "agentReviewerOutput");
        assert_eq!(camel("tool.web_search.input"), "toolWebSearchInput");
        assert_eq!(camel("state.draft"), "stateDraft");
        assert_eq!(
            camel("flow.triage.node.approve.output"),
            "flowTriageNodeApproveOutput"
        );
        assert_eq!(
            camel("store.docs.metadata_schema"),
            "storeDocsMetadataSchema"
        );
    }

    /// Grammar 2.1 admits `a_1` and `a1`, and both camel-case to `a1`. The
    /// registry is what keeps that from emitting one name twice.
    #[test]
    fn colliding_identifiers_are_disambiguated_in_canonical_order() {
        let mut names = Names {
            assigned: BTreeMap::new(),
            taken: BTreeMap::new(),
        };
        names.assign("state.a1");
        names.assign("state.a_1");
        names.assign("state.a__1");
        assert_eq!(names.value("state.a1"), "stateA1");
        assert_eq!(names.value("state.a_1"), "stateA1_2");
        assert_eq!(names.value("state.a__1"), "stateA1_3");
        assert_eq!(names.ty("state.a_1"), "StateA1_2");
    }

    /// A schema may not take a name one of the emitted modules already declares:
    /// `src/index.ts` re-exports every module, so two of them exporting one name
    /// is a duplicate export and a project that does not compile.
    #[test]
    fn a_schema_never_takes_a_name_a_module_already_declares() {
        // `state.channels` camel-cases to `stateChannels`, which is free; the
        // reserved ones are the bare module-level names, and the composition
        // below reaches one of them through a store called `unique_items`.
        let names = Names::of(&ir_of(
            "version: \"0.1\"\nstate:\n  channels: { type: string }\n",
        ));
        assert_eq!(names.value("state.channels"), "stateChannels");

        let mut names = Names {
            assigned: BTreeMap::new(),
            taken: BTreeMap::new(),
        };
        for reserved in RESERVED {
            names.taken.insert((*reserved).to_string(), ());
            names.taken.insert(capitalize(reserved), ());
        }
        names.assign("unique.items");
        assert_eq!(
            names.value("unique.items"),
            "uniqueItems_2",
            "`uniqueItems` is the emitted `unique_items` helper"
        );
        names.assign("read.environment");
        assert_eq!(names.value("read.environment"), "readEnvironment_2");
    }

    /// The same composition assigns the same names however its channels are
    /// ordered on the page, because the IR sorts them before the emitter sees
    /// them.
    #[test]
    fn assignment_does_not_depend_on_declaration_order() {
        let one = Names::of(&ir_of(
            "version: \"0.1\"\nstate:\n  a1: { type: string }\n  a_1: { type: string }\n",
        ));
        let other = Names::of(&ir_of(
            "version: \"0.1\"\nstate:\n  a_1: { type: string }\n  a1: { type: string }\n",
        ));
        assert_eq!(one.value("state.a1"), other.value("state.a1"));
        assert_eq!(one.value("state.a_1"), other.value("state.a_1"));
        assert_eq!(one.value("state.a1"), "stateA1");
    }

    #[test]
    fn strings_are_escaped_the_way_json_escapes_them() {
        assert_eq!(string("plain"), "\"plain\"");
        assert_eq!(string("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(string("two\nlines"), "\"two\\nlines\"");
    }

    #[test]
    fn literals_render_as_typescript_data() {
        assert_eq!(literal(&Literal::Null), "null");
        assert_eq!(literal(&Literal::Bool(true)), "true");
        assert_eq!(literal(&Literal::Int(-3)), "-3");
        assert_eq!(literal(&Literal::String("x".into())), "\"x\"");
        assert_eq!(literal(&Literal::Mapping(Vec::new())), "{}");

        let span = crate::diag::Span::new(
            "main.yml".into(),
            0..0,
            crate::diag::Position::new(1, 1),
            crate::diag::Position::new(1, 1),
        );
        let sequence = Literal::Sequence(vec![
            Spanned::new(Literal::Int(1), span.clone()),
            Spanned::new(Literal::String("two".into()), span.clone()),
        ]);
        assert_eq!(literal(&sequence), "[1, \"two\"]");

        let mapping = Literal::Mapping(vec![crate::ast::common::LiteralEntry {
            key: Spanned::new("fixed".to_string(), span.clone()),
            value: Spanned::new(Literal::Int(0), span),
        }]);
        assert_eq!(literal(&mapping), "{ \"fixed\": 0 }");
    }

    #[test]
    fn a_doc_comment_cannot_be_closed_by_its_own_text() {
        let rendered = doc("  ", &["ends a comment with */ inline".to_string()]);
        assert!(rendered.contains("*\\/"));
        assert_eq!(rendered.matches("*/").count(), 1, "{rendered}");
    }

    #[test]
    fn a_long_doc_comment_is_written_across_lines() {
        let long = "x".repeat(120);
        let rendered = doc("", &[long]);
        assert!(rendered.starts_with("/**\n * "));
        assert!(rendered.ends_with(" */\n"));
    }

    /// Every helper `src/schemas.ts` can declare is reserved.
    ///
    /// A generated schema taking a helper's name would shadow it, and a module
    /// that calls a `const` holding a Zod schema fails at run time rather than
    /// at `tsc`. No canonical path camel-cases to one of these today — every
    /// path opens with a namespace segment — but the reserved list is what
    /// makes that a decision rather than a coincidence, and a list that held
    /// only some of the helpers would be neither.
    #[test]
    fn the_reserved_list_holds_every_emitted_helper() {
        for helper in crate::codegen::schema::helper_names() {
            assert!(
                RESERVED.contains(&helper),
                "`{helper}` is declared in `src/schemas.ts` and is not reserved, \
                 so a schema could be assigned its name"
            );
        }
    }
}
