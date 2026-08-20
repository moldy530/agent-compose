//! The plan format's inventory: every record type `agent-compose plan
//! --format json` writes, mapped to the document that specifies it.
//!
//! `docs/plan.md` is normative for a **public** surface — PRD §7 M2's
//! "`agent-compose plan` (topology + validation diff between two specs)", whose
//! whole point is being read by something other than a person — and a normative
//! document that drifts behind the code it describes is worse than no document
//! at all: a reader pins `plan_version` on the strength of it.
//! `tests/trace_format_inventory.rs` is the same idea over `docs/trace.md`, and
//! this file is its sibling with one difference: the declarations it reads are
//! **Rust**, in `crates/compose-core/src/plan/`, because a plan is written by
//! the compiler rather than by an emitted runtime.
//!
//! # What is checked, and in which direction
//!
//! From the **code toward the document**: every record type reachable from the
//! two documents this command writes, every field of one, and every member of
//! the closed enumerations those fields declare, must be named in
//! `docs/plan.md` — and named *where it belongs*, which is the load-bearing
//! half.
//!
//! A field is looked for in a **table row of the section that introduces its
//! record type**, never anywhere in the file. Searching the whole document is
//! the obvious implementation and it does not work: `docs/plan.md` backticks
//! `before`, `after`, `change`, `span` and a dozen other of its own words for
//! reasons of its own, so a field added to one record would find another
//! record's row and pass — an undocumented field shipping on a versioned public
//! surface with CI green. The section a type is specified in is found from the
//! document itself: each record type is named once, as a bare backticked type
//! name, in that section.
//!
//! The rule this places on the document is the one it already follows: **every
//! field gets a row in the table of its own section.** A field explained only in
//! surrounding prose fails here, and the fix is a row.
//!
//! The other direction is deliberately not checked, for
//! `trace_format_inventory.rs`'s reason: a specification says more than the type
//! declarations do, and asserting that every backticked word is a field would
//! push the prose toward a schema dump.
//!
//! # Where the boundary of the format is
//!
//! The walk follows only names `src/plan/` itself declares. `Span`,
//! `DiagnosticCode`, `Severity` and `Diagnostic` are the compiler's own types,
//! documented by `crates/compose-core/src/diag.rs` and by `docs/grammar.md`
//! Appendix B, and `docs/plan.md` §7 says so out loud: a diagnostic code added
//! to the compiler is not a change to the plan format. `serde_json::Value` is
//! the same — a field change carries whatever the IR holds, and what the IR
//! holds is `crates/compose-core/src/ir`'s business.
//!
//! What that boundary would hide is a plan record typed as something *in this
//! module* that does not serialize, so [`reachable`] refuses one: a reached
//! declaration that does not derive `Serialize` is a failure rather than a
//! skip.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// Every Rust file the plan module is written in, concatenated.
///
/// The whole module rather than `document.rs` alone: what makes a type part of
/// this format is being reachable from a document the command writes, not which
/// file it happens to sit in, and a record type moved to a file of its own must
/// not fall out of the inventory by moving.
fn module() -> String {
    let directory = repository().join("crates/compose-core/src/plan");
    let mut paths: Vec<PathBuf> = fs::read_dir(&directory)
        .expect("the plan module is readable")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|held| held == "rs"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 2,
        "the plan module is more than one file; this read {paths:?}"
    );
    paths
        .iter()
        .map(|path| fs::read_to_string(path).expect("a module file is readable"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The document under test.
fn specification() -> String {
    fs::read_to_string(repository().join("docs/plan.md")).expect("docs/plan.md is readable")
}

/// Where the walk starts: the two documents the command writes.
///
/// Everything else is reached from these — the plan to its four sections, a
/// component change to the vocabulary it is tagged with, and so on — so a record
/// type that becomes part of the format by being referenced from one of them is
/// inventoried without anybody remembering to add it here.
const ROOTS: &[&str] = &["Plan", "Refusal"];

/// One type declaration of the plan module.
#[derive(Clone)]
struct Declaration {
    /// Whether it reaches JSON at all.
    serialized: bool,
    /// The field names it declares. Empty for an enumeration.
    fields: Vec<String>,
    /// The members it declares, in their serialized spelling. Empty for a
    /// struct.
    members: Vec<String>,
    /// Every other declaration its fields name.
    references: Vec<String>,
    /// Body lines that are neither a comment, an attribute, nor a member shape
    /// this file reads.
    unreadable: Vec<String>,
}

/// Every `pub struct` and `pub enum` in `source`, by name.
fn declarations(source: &str) -> BTreeMap<String, Declaration> {
    let mut found: BTreeMap<String, Declaration> = BTreeMap::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut attributes: Vec<&str> = Vec::new();
    let mut at = 0usize;
    while at < lines.len() {
        let line = lines[at];
        let trimmed = line.trim();
        if trimmed.starts_with("#[") {
            attributes.push(trimmed);
            at += 1;
            continue;
        }
        let structure = line.strip_prefix("pub struct ").map(|rest| (rest, true));
        let enumeration = line.strip_prefix("pub enum ").map(|rest| (rest, false));
        let Some((header, is_struct)) = structure.or(enumeration) else {
            if !trimmed.is_empty() && !trimmed.starts_with("///") && !trimmed.starts_with("//") {
                attributes.clear();
            }
            at += 1;
            continue;
        };
        let name = header
            .trim_end()
            .strip_suffix(" {")
            .unwrap_or_else(|| panic!("`{header}` does not open its body on one line"))
            .split(['<', '('])
            .next()
            .expect("a name")
            .trim()
            .to_string();
        let serialized = attributes.iter().any(|held| held.contains("Serialize"));
        let snake = attributes
            .iter()
            .any(|held| held.contains("rename_all = \"snake_case\""));
        assert!(
            is_struct || !serialized || snake,
            "`enum {name}` is written into the plan document and does not declare \
             `#[serde(rename_all = \"snake_case\")]`; every vocabulary of this format is \
             spelled that way, and `docs/plan.md` writes the members out"
        );
        attributes.clear();

        let mut declaration = Declaration {
            serialized,
            fields: Vec::new(),
            members: Vec::new(),
            references: Vec::new(),
            unreadable: Vec::new(),
        };
        at += 1;
        while at < lines.len() && lines[at] != "}" {
            let trimmed = lines[at].trim();
            if trimmed.is_empty()
                || trimmed.starts_with("//")
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
                || trimmed.starts_with("#[")
            {
                at += 1;
                continue;
            }
            if is_struct {
                match trimmed.strip_prefix("pub ").and_then(|rest| {
                    rest.split_once(": ")
                        .map(|(name, ty)| (name.trim().to_string(), ty.to_string()))
                }) {
                    Some((name, ty)) => {
                        declaration.references.extend(identifiers(&ty));
                        declaration.fields.push(name);
                    }
                    None => declaration.unreadable.push(trimmed.to_string()),
                }
            } else {
                match trimmed.strip_suffix(',').filter(|held| {
                    !held.is_empty() && held.chars().all(|c| c.is_alphanumeric() || c == '_')
                }) {
                    Some(variant) => declaration.members.push(snake_case(variant)),
                    None => declaration.unreadable.push(trimmed.to_string()),
                }
            }
            at += 1;
        }
        assert!(
            at < lines.len(),
            "`{name}` has no closing brace in the first column"
        );
        found.insert(name, declaration);
        at += 1;
    }
    found
}

/// A variant's name in the spelling `#[serde(rename_all = "snake_case")]` gives
/// it.
fn snake_case(name: &str) -> String {
    let mut held = String::new();
    for (at, character) in name.chars().enumerate() {
        if character.is_uppercase() {
            if at > 0 {
                held.push('_');
            }
            held.extend(character.to_lowercase());
        } else {
            held.push(character);
        }
    }
    held
}

/// Every bare identifier in `text`, which is what a reference to another
/// declaration looks like inside a field's type.
fn identifiers(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut held = String::new();
    for character in text.chars() {
        if character.is_alphanumeric() || character == '_' {
            held.push(character);
        } else if !held.is_empty() {
            found.push(std::mem::take(&mut held));
        }
    }
    if !held.is_empty() {
        found.push(held);
    }
    found
}

/// The record types the plan format is made of: the roots, and everything they
/// reach.
///
/// # Panics
///
/// On a reachable declaration this file could not read in full, on one that
/// does not serialize, and on one it read nothing at all out of. Each would
/// leave a field out of the inventory while every assertion below still passed,
/// which is the one failure mode this file has to be loud about.
fn reachable(source: &str) -> BTreeMap<String, Declaration> {
    let all = declarations(source);
    let mut held: BTreeMap<String, Declaration> = BTreeMap::new();
    let mut queue: Vec<String> = ROOTS.iter().map(|name| (*name).to_string()).collect();
    while let Some(name) = queue.pop() {
        if held.contains_key(&name) {
            continue;
        }
        let Some(declaration) = all.get(&name) else {
            panic!("`{name}` is named by the plan format but the plan module does not declare it");
        };
        assert!(
            declaration.serialized,
            "`{name}` is part of the plan format and does not derive `Serialize`, so it \
             reaches no document and this inventory would carry a type nothing writes"
        );
        assert!(
            declaration.unreadable.is_empty(),
            "`{name}` is part of the plan format and has a member this inventory cannot \
             read: {:?}",
            declaration.unreadable
        );
        assert!(
            !declaration.fields.is_empty() || !declaration.members.is_empty(),
            "`{name}` is part of the plan format and was read as declaring nothing at all, \
             which means this file could not read it"
        );
        for reference in &declaration.references {
            if all.contains_key(reference) && !held.contains_key(reference) {
                queue.push(reference.clone());
            }
        }
        held.insert(name, declaration.clone());
    }
    held
}

/// One section of the document: a heading, and the lines up to the next
/// heading.
struct Section {
    heading: String,
    body: Vec<String>,
}

/// The document, split at its headings. Subsections are sections of their own.
fn sections(document: &str) -> Vec<Section> {
    let mut found: Vec<Section> = Vec::new();
    let mut fenced = false;
    for line in document.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        if !fenced && line.starts_with('#') {
            found.push(Section {
                heading: line.to_string(),
                body: Vec::new(),
            });
        } else if let Some(section) = found.last_mut() {
            section.body.push(line.to_string());
        }
    }
    found
}

/// Where a record type is specified: the section whose body names it as a bare
/// backticked type name.
fn introduces<'a>(sections: &'a [Section], name: &str) -> Vec<&'a Section> {
    sections
        .iter()
        .filter(|section| {
            section
                .body
                .iter()
                .any(|line| line.contains(&format!("`{name}`")))
        })
        .collect()
}

/// Whether a **table row** of this section names `token`.
///
/// A row rather than the section's prose, because the prose is where a field is
/// *discussed* and the table is where it is *specified*. The pipe is the test:
/// `docs/plan.md` writes every record type's fields as one table.
fn row_names(section: &Section, token: &str) -> bool {
    section
        .body
        .iter()
        .any(|line| line.trim_start().starts_with('|') && line.contains(&format!("`{token}`")))
}

/// The same, for a member of a closed vocabulary, which is a JSON **string** —
/// so the document writes it with its quotes and this requires them. Without
/// that, `"flow"` would be read as satisfied by every place the word `flow` is
/// backticked as itself, and `docs/plan.md` backticks it as a field name in the
/// section next door.
fn row_names_member(section: &Section, member: &str) -> bool {
    row_names(section, &format!("\"{member}\""))
}

/// Every record type of `declarations` the document does not introduce exactly
/// once.
fn unintroduced(document: &str, declarations: &BTreeMap<String, Declaration>) -> Vec<String> {
    let sections = sections(document);
    let mut missing: Vec<String> = Vec::new();
    for name in declarations.keys() {
        match introduces(&sections, name).as_slice() {
            [_] => {}
            [] => missing.push(format!("`{name}`: no section names it")),
            found => missing.push(format!(
                "`{name}`: {} sections name it ({}), so which one specifies it is ambiguous",
                found.len(),
                found
                    .iter()
                    .map(|section| section.heading.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
    missing
}

/// Every `<type>.<field>` the document does not give a row of that type's own
/// section.
fn undocumented_fields(
    document: &str,
    declarations: &BTreeMap<String, Declaration>,
) -> Vec<String> {
    let sections = sections(document);
    let mut missing: Vec<String> = Vec::new();
    for (name, declaration) in declarations {
        let held = introduces(&sections, name);
        let [home] = held.as_slice() else {
            missing.extend(declaration.fields.iter().map(|field| {
                format!("{name}.{field} (no single section of the document introduces `{name}`)")
            }));
            continue;
        };
        for field in &declaration.fields {
            if !row_names(home, field) {
                missing.push(format!("{name}.{field} (looked for in {})", home.heading));
            }
        }
    }
    missing.sort();
    missing
}

/// The same for the members of the closed vocabularies.
fn undocumented_members(
    document: &str,
    declarations: &BTreeMap<String, Declaration>,
) -> Vec<String> {
    let sections = sections(document);
    let mut missing: Vec<String> = Vec::new();
    for (name, declaration) in declarations {
        let held = introduces(&sections, name);
        let [home] = held.as_slice() else {
            missing.extend(declaration.members.iter().map(|member| {
                format!("{name}: \"{member}\" (no single section introduces `{name}`)")
            }));
            continue;
        };
        for member in &declaration.members {
            if !row_names_member(home, member) {
                missing.push(format!(
                    "{name}: \"{member}\" (looked for in {})",
                    home.heading
                ));
            }
        }
    }
    missing.sort();
    missing
}

/// Every record type the plan format reaches is specified, in one place.
#[test]
fn every_plan_record_type_is_documented() {
    let missing = unintroduced(&specification(), &reachable(&module()));
    assert!(
        missing.is_empty(),
        "`docs/plan.md` introduces a record type by naming it as a bare backticked type \
         name in the section that specifies it, and these declarations of \
         `src/plan/` have no such section: {missing:?}"
    );
}

/// Every field of every one of them is specified, in that type's own section.
#[test]
fn every_plan_record_field_is_documented() {
    let missing = undocumented_fields(&specification(), &reachable(&module()));
    assert!(
        missing.is_empty(),
        "these plan fields are written by `src/plan/` and have no row in the \
         `docs/plan.md` section that specifies their record type, so a reader pinning \
         `plan_version` has not been told about them: {missing:?}"
    );
}

/// …and every member of the closed vocabularies those fields declare.
///
/// The vocabularies are the half of a record's meaning a field name does not
/// carry: `change` says nothing without `"added"` beside it, and §12.3 makes
/// adding a member a version bump. A member added in the compiler and not
/// written down would be a bump nobody knew to make.
#[test]
fn every_vocabulary_member_is_documented() {
    let missing = undocumented_members(&specification(), &reachable(&module()));
    assert!(
        missing.is_empty(),
        "these vocabulary members are written by `src/plan/` and have no row in the \
         `docs/plan.md` section that specifies their record type: {missing:?}"
    );
}

/// The version the document declares is the version the compiler emits.
#[test]
fn the_documented_version_is_the_one_the_compiler_emits() {
    let declared = module()
        .lines()
        .find_map(|line| line.trim().strip_prefix("pub const PLAN_VERSION: u32 = "))
        .map(|rest| rest.trim_end_matches(';').trim().to_string())
        .expect("the plan module declares `PLAN_VERSION`");

    let document = specification();
    let documented = document
        .lines()
        .find_map(|line| line.strip_prefix("**Plan version:** "))
        .map(|rest| rest.trim().trim_matches('`').to_string())
        .expect("`docs/plan.md` opens with the plan version it specifies");

    assert_eq!(
        documented, declared,
        "`docs/plan.md` specifies plan version {documented} and the compiler emits \
         {declared}; a bump moves both (see its §12)"
    );
}

/// The walk really reaches the whole format rather than stopping at the roots.
///
/// Without this, a `reachable` that silently followed nothing would make the
/// three assertions above pass over two declarations while the format has
/// thirteen.
#[test]
fn the_walk_reaches_every_record_the_two_documents_carry() {
    let reached: BTreeSet<String> = reachable(&module()).into_keys().collect();
    let expected: BTreeSet<String> = [
        "ChangeKind",
        "ComponentChange",
        "ComponentKind",
        "FieldChange",
        "Finding",
        "InterfaceChange",
        "InterfaceKind",
        "Plan",
        "Refusal",
        "Refused",
        "Spec",
        "SpecSide",
        "TopologyChange",
        "TopologyKind",
        "Validation",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        reached, expected,
        "the set of record types the plan format is made of has changed; if that is \
         intended, `docs/plan.md` describes it and this row moves with it"
    );
}

/// The document's keys are the declarations' own names.
///
/// This is what makes the inventory above readable at all: it reads a field's
/// name off the Rust declaration and looks for exactly that string in
/// `docs/plan.md`. A `#[serde(rename = …)]` would put a key in the document that
/// no declaration spells, a `#[serde(flatten)]` would move one record's keys
/// into another's object, and a `#[serde(skip)]` would leave a documented field
/// out of the JSON — each of them a drift this file could not see. So the plan
/// module does none of the three, and this is where that is a rule rather than a
/// habit.
///
/// `skip_serializing_if` is the one attribute the format does use, and it is
/// deliberately not on the list: it makes a key *absent*, which
/// `docs/plan.md` §3 gives a meaning to.
#[test]
fn the_documents_keys_are_the_declarations_own_names() {
    let source = module();
    for attribute in ["serde(rename = ", "serde(flatten", "serde(skip)"] {
        assert!(
            !source.contains(attribute),
            "`src/plan/` uses `#[{attribute}…)]`, which puts a key in the plan document \
             that its declaration does not spell — and this inventory reads the \
             declarations"
        );
    }
}

/// A field is looked for where its record type is specified, not anywhere in
/// the file.
///
/// This is the assertion the three above are only as strong as. A
/// whole-document search passes on a field whose name happens to be backticked
/// somewhere else — and `docs/plan.md` backticks `before`, `after` and `span`
/// for reasons of its own — so the scoping is pinned here rather than trusted.
#[test]
fn a_field_named_only_outside_its_types_section_is_undocumented() {
    let declared = declarations(
        r#"
#[derive(Serialize)]
pub struct Plan {
    pub plan_version: u32,
    pub components: Vec<ComponentChange>,
}

#[derive(Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    EventSource,
}
"#,
    );
    assert_eq!(declared["ChangeKind"].members, ["added", "event_source"]);

    let document = "\
## 2.1 The plan

`Plan`, the document a comparison produces.

| field | meaning |
|---|---|
| `plan_version` | the shape of this document |
| `components` | the components section, whose entries carry `\"added\"` |

## 3. What a change record says

`ChangeKind`.

| member | meaning |
|---|---|
| `\"added\"` | it arrived |
| `\"event_source\"` | not really a change kind |
";

    assert!(
        undocumented_fields(document, &declared).is_empty(),
        "each field has a row in its own type's section"
    );
    assert!(undocumented_members(document, &declared).is_empty());

    // The same document with the vocabulary's own rows removed. `"added"` is
    // still backticked in §2.1, which a whole-file search would find.
    let thinned = document
        .replace("| `\"added\"` | it arrived |\n", "")
        .replace("| `\"event_source\"` | not really a change kind |\n", "");
    assert_eq!(
        undocumented_members(&thinned, &declared),
        [
            "ChangeKind: \"added\" (looked for in ## 3. What a change record says)",
            "ChangeKind: \"event_source\" (looked for in ## 3. What a change record says)",
        ],
        "a member with no row in its own section is missing, whatever the rest of the \
         file backticks"
    );
}

/// A record type named in two sections is ambiguous rather than doubly
/// documented.
#[test]
fn a_record_type_two_sections_introduce_has_no_home() {
    let declared = declarations(
        r#"
#[derive(Serialize)]
pub struct Spec {
    pub target: String,
}
"#,
    );
    let missing = unintroduced(
        "\
## 2.2 Each side

`Spec`.

## 12. Stability

`Spec` again, which leaves no one section specifying it.
",
        &declared,
    );
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(
        missing[0].contains("2 sections name it"),
        "the failure says what is ambiguous: {missing:?}"
    );
}

/// A member shape the reader does not understand fails — where the format
/// reaches it.
#[test]
#[should_panic(expected = "is part of the plan format and has a member this inventory cannot read")]
fn an_unreadable_member_of_a_planned_record_is_a_failure() {
    let _ = reachable(
        r#"
#[derive(Serialize)]
pub struct Refusal {
    pub plan_version: u32,
}

#[derive(Serialize)]
pub struct Plan {
    pub validation: Validation,
}

#[derive(Serialize)]
pub struct Validation {
    introduced: Vec<String>,
}
"#,
    );
}

/// …and a record the format reaches that does not serialize is one too.
#[test]
#[should_panic(expected = "is part of the plan format and does not derive `Serialize`")]
fn a_record_the_format_reaches_that_does_not_serialize_is_a_failure() {
    let _ = reachable(
        r#"
#[derive(Serialize)]
pub struct Refusal {
    pub plan_version: u32,
}

#[derive(Serialize)]
pub struct Plan {
    pub before: Spec,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub target: String,
}
"#,
    );
}
