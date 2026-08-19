//! The trace format's inventory: every record type the emitted runtime declares,
//! mapped to the document that specifies it.
//!
//! `docs/trace.md` is normative for a **public** surface — PRD §7 M2's "a
//! documented, stable trace format" — and a normative document that drifts behind
//! the code it describes is worse than no document at all: a reader pins
//! `trace_version` on the strength of it. `tests/static_check_inventory.rs` and
//! `crates/agent-compose/tests/acceptance_inventory.rs` are the same idea over
//! their own surfaces; this one points at prose rather than at diagnostic codes
//! or test names, because what a trace format promises is written in prose.
//!
//! # What is checked, and in which direction
//!
//! From the **code toward the document**: every record type reachable from the
//! trace envelope, every field of one, and every member of the closed
//! enumerations those fields declare, must be named in `docs/trace.md` — and
//! named *where it belongs*, which is the load-bearing half.
//!
//! A field is looked for in a **table row of the section that introduces its
//! record type**, never anywhere in the file. Searching the whole document is
//! the obvious implementation and it does not work: `docs/trace.md` backticks
//! `status`, `map`, `default`, `key` and a dozen other ordinary words for their
//! own reasons, so a `TraceEntry.status` added to the runtime would find §2's
//! envelope row and pass — an undocumented field shipping on a versioned public
//! surface with CI green. Scoping is what closes that, and the section a type is
//! introduced in is found from the document itself: each record type is named
//! once, as a bare backticked type name, in the section that specifies it.
//!
//! The rule this places on the document is the one it already follows: **every
//! field gets a row in the table of its own section.** A field explained only in
//! surrounding prose fails here, and the fix is a row.
//!
//! One class of field is held to a **second** section besides its own: a field
//! that carries free diagnostic text needs a row in §11.1 as well, because that
//! is where an operator reads which fields can hold bytes this process did not
//! compose. See [`every_message_field_is_classified_as_untrusted_text`].
//!
//! The other direction is deliberately not checked. A specification says more
//! than the type declarations do — presence rules, orders, what a reader may rely
//! on — so "every backticked word in the document is a field" is not a property
//! that holds, and asserting it would push the prose toward a schema dump.
//!
//! # How the declarations are read
//!
//! By line, over the emitted `src/runtime.ts`, which is a formatted file this
//! repository owns: an `interface` header, a body of doc comments and one-line
//! members, a closing brace. A body line that is neither a comment nor a member
//! the reader understands is kept as **unreadable**, and a declaration the
//! format actually reaches carrying one is a failure — see [`reachable`].
//!
//! `export` is **not** part of the test. What makes a type part of this format
//! is being reachable from the envelope, and a record type the runtime happens
//! to declare without exporting — `src/runtime.ts` already declares several
//! unexported interfaces for its own use — would otherwise be absent from the
//! map, skipped by [`reachable`]'s walk rather than followed, and have none of
//! its fields held to `docs/trace.md`: an undocumented public surface with CI
//! green, which is the precise drift this file exists to catch. Reading every
//! declaration and letting reachability decide is what closes that.
//! The runtime declares plenty this format does not carry (a method signature on
//! `ResultSchema`, for one), so refusing every shape outright would fail on
//! declarations that have nothing to do with a trace; refusing them where they
//! *are* part of the format is what keeps a member from silently falling out of
//! the inventory.

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

/// The emitted runtime, which is where every record type is declared.
fn runtime() -> String {
    fs::read_to_string(repository().join("crates/compose-core/src/codegen/js/runtime.ts"))
        .expect("the emitted runtime is readable")
}

/// The document under test.
fn specification() -> String {
    fs::read_to_string(repository().join("docs/trace.md")).expect("docs/trace.md is readable")
}

/// Where the walk starts: the envelope, and the entry it carries.
///
/// Everything else is reached from these — `TraceEntry.routing` to a routing
/// decision, that to an edge decision, and so on — so a record type that becomes
/// part of the format by being referenced from one of them is inventoried
/// without anybody remembering to add it here.
const ROOTS: &[&str] = &["TraceDocument", "TraceEntry"];

/// One type declaration of the emitted runtime.
#[derive(Clone)]
struct Declaration {
    /// The member names it declares, nested inline object types included.
    /// Empty for a type alias.
    fields: Vec<String>,
    /// Every string literal in it — the members of a closed enumeration.
    literals: Vec<String>,
    /// Every other declaration it names: member types, and an `extends` clause.
    references: Vec<String>,
    /// Body lines that are neither a comment nor a member shape this file reads.
    unreadable: Vec<String>,
}

/// Every `interface` and `type` declaration in `source`, by name — whether or
/// not the runtime exports it (see this file's header).
fn declarations(source: &str) -> BTreeMap<String, Declaration> {
    let mut found: BTreeMap<String, Declaration> = BTreeMap::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut at = 0usize;
    while at < lines.len() {
        let line = lines[at];
        if let Some(header) = line
            .strip_prefix("export interface ")
            .or_else(|| line.strip_prefix("interface "))
        {
            let (name, extends, body_at) = interface_header(header, at);
            let mut declaration = Declaration {
                fields: Vec::new(),
                literals: Vec::new(),
                references: identifiers(&extends),
                unreadable: Vec::new(),
            };
            at = body_at;
            // The body runs to the closing brace in the first column, which is
            // where a formatted declaration ends.
            while at < lines.len() && lines[at] != "}" {
                match read(lines[at]) {
                    Line::Comment => {}
                    Line::Member(member) => {
                        declaration.fields.extend(member.names);
                        declaration.literals.extend(strings(&member.ty));
                        declaration.references.extend(identifiers(&member.ty));
                    }
                    Line::Unreadable(text) => declaration.unreadable.push(text),
                }
                at += 1;
            }
            assert!(
                at < lines.len(),
                "`interface {name}` has no closing brace in the first column"
            );
            found.insert(name, declaration);
        } else if let Some(header) = line
            .strip_prefix("export type ")
            .or_else(|| line.strip_prefix("type "))
            // `export type { … };` re-exports names another module declared; it
            // introduces nothing, and the two are told apart by the brace.
            .filter(|header| !header.trim_start().starts_with('{'))
        {
            // An alias may run over several lines; it ends at the `;`.
            let mut text = header.to_string();
            while !text.trim_end().ends_with(';') && at + 1 < lines.len() {
                at += 1;
                text.push('\n');
                text.push_str(lines[at]);
            }
            let (name, body) = text
                .split_once('=')
                .unwrap_or_else(|| panic!("`type {header}` has no `=`"));
            let name = name.trim().to_string();
            found.insert(
                name,
                Declaration {
                    fields: Vec::new(),
                    literals: strings(body),
                    references: identifiers(body),
                    unreadable: Vec::new(),
                },
            );
        }
        at += 1;
    }
    found
}

/// An interface header: its name, its `extends` clause, and the line its body
/// starts on.
fn interface_header(header: &str, at: usize) -> (String, String, usize) {
    let header = header.trim_end();
    let open = header
        .strip_suffix('{')
        .unwrap_or_else(|| panic!("`interface {header}` does not open its body on one line"))
        .trim();
    let (name, extends) = match open.split_once(" extends ") {
        Some((name, extends)) => (name, extends),
        None => (open, ""),
    };
    // A generic parameter list is not part of the name.
    let name = name.split('<').next().expect("a name").trim();
    (name.to_string(), extends.to_string(), at + 1)
}

/// One member of an interface body: the names it declares, and its type text.
struct Member {
    names: Vec<String>,
    ty: String,
}

/// What one line of an interface body is.
enum Line {
    /// A doc comment, a line comment, or blank.
    Comment,
    /// A member declaration.
    Member(Member),
    /// Something else — kept, and refused where it is part of the format.
    Unreadable(String),
}

/// Read one body line.
fn read(line: &str) -> Line {
    let trimmed = line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("//")
        || trimmed.starts_with("/*")
        || trimmed.starts_with('*')
    {
        return Line::Comment;
    }
    let Some((name, ty)) = declared(trimmed) else {
        return Line::Unreadable(trimmed.to_string());
    };
    let mut names = vec![name];
    // A member whose type is an inline object declares members of its own, and
    // they are as much a part of the format as any other — `budget: { key, used,
    // max }` is three fields a reader indexes by name.
    if let Some(open) = ty.find('{') {
        let close = ty.rfind('}').unwrap_or(ty.len());
        let inline = &ty[open + 1..close];
        for part in inline.split(';') {
            if let Some((nested, _)) = declared(part.trim()) {
                names.push(nested);
            }
        }
    }
    Line::Member(Member { names, ty })
}

/// `[readonly] <name>[?]: <type>` — the one member shape this format uses.
fn declared(text: &str) -> Option<(String, String)> {
    let text = text.trim().trim_start_matches("readonly ").trim_start();
    let colon = text.find(':')?;
    let (name, rest) = text.split_at(colon);
    let name = name.trim().trim_end_matches('?').trim();
    if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some((
        name.to_string(),
        rest.trim_start_matches(':').trim().to_string(),
    ))
}

/// Every double-quoted literal in `text`.
fn strings(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('"') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find('"') else { break };
        found.push(rest[..close].to_string());
        rest = &rest[close + 1..];
    }
    found
}

/// Every bare identifier in `text`, which is what a reference to another
/// declaration looks like.
fn identifiers(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut held = String::new();
    let mut quoted = false;
    for character in text.chars() {
        if character == '"' {
            quoted = !quoted;
            continue;
        }
        if !quoted && (character.is_alphanumeric() || character == '_') {
            held.push(character);
            continue;
        }
        if !held.is_empty() {
            found.push(std::mem::take(&mut held));
        }
    }
    if !held.is_empty() {
        found.push(held);
    }
    found
}

/// The record types the trace format is made of: the roots, and everything they
/// reach.
///
/// # Panics
///
/// On a reachable declaration this file could not read in full — one carrying an
/// [unreadable][`Declaration::unreadable`] body line, or an interface it read no
/// member out of at all. Either would leave a field out of the inventory while
/// every assertion below still passed, which is the one failure mode this file
/// has to be loud about.
fn reachable(source: &str) -> BTreeMap<String, Declaration> {
    let all = declarations(source);
    let mut held: BTreeMap<String, Declaration> = BTreeMap::new();
    let mut queue: Vec<String> = ROOTS.iter().map(|name| (*name).to_string()).collect();
    while let Some(name) = queue.pop() {
        if held.contains_key(&name) {
            continue;
        }
        let Some(declaration) = all.get(&name) else {
            panic!("`{name}` is named by the trace format but is not declared by the runtime");
        };
        assert!(
            declaration.unreadable.is_empty(),
            "`{name}` is part of the trace format and has a member this inventory \
             cannot read: {:?}",
            declaration.unreadable
        );
        assert!(
            !declaration.fields.is_empty() || !declaration.literals.is_empty(),
            "`{name}` is part of the trace format and was read as declaring nothing \
             at all, which means this file could not read it"
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

/// Whether the document names `token` as a name rather than in passing.
///
/// Backticks are the test: `docs/trace.md` writes every field, type and
/// enumeration member as code, so a match is a mention of the thing rather than
/// of an English word that happens to be spelled the same (`error`, `key`,
/// `value`, `step`).
///
/// Used unscoped only where the whole document is the right scope — the version
/// constant, which belongs to no record type. Everything a record type owns goes
/// through [`row_names`] instead.
fn names(document: &str, token: &str) -> bool {
    document.contains(&format!("`{token}`"))
}

/// One section of the document: a heading, and the lines up to the next heading.
struct Section {
    /// The heading line, verbatim, for a failure message a reader can act on.
    heading: String,
    /// Every line under it, up to the next heading of any level.
    body: Vec<String>,
}

/// The document, split at its headings.
///
/// Subsections are sections of their own rather than part of their parent: §4
/// specifies `RoutingDecision` and §4.1 specifies `EdgeDecision`, and a field of
/// one is not documented by a row in the other's table.
///
/// A fenced block is never read for headings: `#` opens a comment in half the
/// languages a specification quotes, and a heading found inside one would split
/// a section in the middle.
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
/// A row rather than the section's prose, because the prose of a section is
/// where a field is *discussed* and the table is where it is *specified* — and a
/// discussion is exactly what leaves a reader unable to tell presence from
/// meaning. The pipe is the test: `docs/trace.md` writes every record type's
/// fields as one table.
fn row_names(section: &Section, token: &str) -> bool {
    section
        .body
        .iter()
        .any(|line| line.trim_start().starts_with('|') && line.contains(&format!("`{token}`")))
}

/// The same, for a member of a closed enumeration.
///
/// Stricter than [`row_names`], and deliberately: a member is a JSON **string**,
/// so the document writes it with its quotes — `` `"detached"` `` — and
/// requiring them is what keeps `"node"` from being read as satisfied by the
/// places the word `node` is backticked as itself.
fn row_names_member(section: &Section, member: &str) -> bool {
    row_names(section, &format!("\"{member}\""))
}

/// Every record type of `declarations` the document does not introduce exactly
/// once, as `<name>: <what is wrong>`.
///
/// Exactly once rather than at least once: the section a type is introduced in
/// is what scopes every other assertion here, so a type named as a bare
/// backticked name in two places leaves the scope ambiguous, and a document that
/// has drifted into specifying one record in two places is worth failing over.
fn unintroduced(document: &str, declarations: &BTreeMap<String, Declaration>) -> Vec<String> {
    let sections = sections(document);
    let mut missing: Vec<String> = Vec::new();
    for name in declarations.keys() {
        match introduces(&sections, name).as_slice() {
            [_] => {}
            [] => missing.push(format!("`{name}`: no section names it")),
            found => missing.push(format!(
                "`{name}`: {} sections name it ({}), so which one specifies it is \
                 ambiguous",
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
            // Reported by `every_trace_record_type_is_documented`; with no
            // section to scope to, none of its fields can be documented either.
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

/// The same for the members of the closed enumerations those fields declare.
fn undocumented_members(
    document: &str,
    declarations: &BTreeMap<String, Declaration>,
) -> Vec<String> {
    let sections = sections(document);
    let mut missing: Vec<String> = Vec::new();
    for (name, declaration) in declarations {
        let held = introduces(&sections, name);
        let [home] = held.as_slice() else {
            missing.extend(declaration.literals.iter().map(|literal| {
                format!("{name}: \"{literal}\" (no single section introduces `{name}`)")
            }));
            continue;
        };
        for literal in &declaration.literals {
            if !row_names_member(home, literal) {
                missing.push(format!(
                    "{name}: \"{literal}\" (looked for in {})",
                    home.heading
                ));
            }
        }
    }
    missing.sort();
    missing
}

/// Every record type the trace format reaches is specified, in one place.
#[test]
fn every_trace_record_type_is_documented() {
    let missing = unintroduced(&specification(), &reachable(&runtime()));
    assert!(
        missing.is_empty(),
        "`docs/trace.md` introduces a record type by naming it as a bare backticked \
         type name in the section that specifies it, and these declarations of \
         `src/runtime.ts` have no such section: {missing:?}"
    );
}

/// Every field of every one of them is specified, in that type's own section.
#[test]
fn every_trace_record_field_is_documented() {
    let missing = undocumented_fields(&specification(), &reachable(&runtime()));
    assert!(
        missing.is_empty(),
        "these trace fields are recorded by `src/runtime.ts` and have no row in the \
         `docs/trace.md` section that specifies their record type, so a reader \
         pinning `trace_version` has not been told about them: {missing:?}"
    );
}

/// …and every member of the closed enumerations those fields declare.
///
/// The vocabularies are the half of a record's meaning a field name does not
/// carry: `outcome` says nothing without `"detached"` beside it, and §10 makes
/// adding a member a version bump. A member added in the runtime and not written
/// down would be a bump nobody knew to make.
#[test]
fn every_enumeration_member_is_documented() {
    let missing = undocumented_members(&specification(), &reachable(&runtime()));
    assert!(
        missing.is_empty(),
        "these enumeration members are emitted by `src/runtime.ts` and have no row in \
         the `docs/trace.md` section that specifies their record type: {missing:?}"
    );
}

/// A field is looked for where its record type is specified, not anywhere in the
/// file.
///
/// This is the assertion the three above are only as strong as. A whole-document
/// search passes on a field whose name happens to be backticked somewhere else —
/// and `docs/trace.md` backticks `status`, `map` and `key` for reasons of its
/// own — so the scoping is pinned here rather than trusted.
#[test]
fn a_field_named_only_outside_its_types_section_is_undocumented() {
    let declared = declarations(
        r#"
export interface TraceDocument {
  readonly status: "completed";
  readonly entries: readonly TraceEntry[];
}

export interface TraceEntry {
  readonly outcome: "completed";
}
"#,
    );
    let document = "\
## 2. The envelope

`TraceDocument`, in the emitted `src/runtime.ts`.

| field | meaning |
|---|---|
| `status` | whether the run produced an answer, `\"completed\"` or not |
| `entries` | every entry the run recorded |

## 3. Entries

`TraceEntry`.

| field | meaning |
|---|---|
| `outcome` | `\"completed\"`, and the rest |
";

    assert!(
        undocumented_fields(document, &declared).is_empty(),
        "each field has a row in its own type's section"
    );

    // The same document, with `TraceEntry`'s row removed. `status` and
    // `entries` are still backticked in §2 — a whole-file search would find
    // them — and `outcome` is not backticked at all.
    let thinned = document.replace("| `outcome` | `\"completed\"`, and the rest |\n", "");
    let missing = undocumented_fields(&thinned, &declared);
    assert_eq!(
        missing,
        ["TraceEntry.outcome (looked for in ## 3. Entries)"],
        "a field with no row in its own section is missing, whatever the rest of the \
         file backticks"
    );
    assert_eq!(
        undocumented_members(&thinned, &declared),
        ["TraceEntry: \"completed\" (looked for in ## 3. Entries)"],
        "…and so is its vocabulary, though `\"completed\"` is a row in §2"
    );
}

/// A record type named in two sections is ambiguous rather than doubly
/// documented.
#[test]
fn a_record_type_two_sections_introduce_has_no_home() {
    let declared = declarations(
        r#"
export interface TraceEntry {
  readonly outcome: "completed";
}
"#,
    );
    let missing = unintroduced(
        "\
## 3. Entries

`TraceEntry`.

## 9. Failed runs

`TraceEntry`, once a run has stopped.
",
        &declared,
    );
    assert_eq!(missing.len(), 1, "{missing:?}");
    assert!(
        missing[0].contains("2 sections name it"),
        "the failure says what is ambiguous: {missing:?}"
    );
}

/// The walk really reaches the whole format rather than stopping at the roots.
///
/// Without this, a `reachable` that silently followed nothing would make the
/// three assertions above pass over two declarations while the format has ten.
#[test]
fn the_walk_reaches_every_record_the_envelope_carries() {
    let reached: BTreeSet<String> = reachable(&runtime()).into_keys().collect();
    let expected: BTreeSet<String> = [
        "DispatchRecord",
        "EdgeDecision",
        "Failover",
        "ModelCall",
        "Refusal",
        "RouteCondition",
        "RoutingDecision",
        "StoreRecord",
        "TraceDocument",
        "TraceEntry",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    assert_eq!(
        reached, expected,
        "the set of record types the trace format is made of has changed; if that is \
         intended, `docs/trace.md` describes it and this row moves with it"
    );
}

/// The version the document declares is the version the runtime emits.
#[test]
fn the_documented_version_is_the_one_the_runtime_emits() {
    let source = runtime();
    let declared = source
        .lines()
        .find_map(|line| line.trim().strip_prefix("export const TRACE_VERSION = "))
        .map(|rest| rest.trim_end_matches(';').trim().to_string())
        .expect("`src/runtime.ts` declares `TRACE_VERSION`");

    let document = specification();
    let documented = document
        .lines()
        .find_map(|line| line.strip_prefix("**Trace version:** "))
        .map(|rest| rest.trim().trim_matches('`').to_string())
        .expect("`docs/trace.md` opens with the trace version it specifies");

    assert_eq!(
        documented, declared,
        "`docs/trace.md` specifies trace version {documented} and the runtime emits \
         {declared}; a bump moves both (see its §10)"
    );
    assert!(
        names(&document, "TRACE_VERSION"),
        "…and the document names the constant a compiled project spells it with"
    );
}

/// Every surface that delivers a trace delivers the version beside it.
///
/// A source-level check rather than a live one, because it is about the *rule*
/// — `docs/trace.md` §1's "wherever a `trace` appears, the `trace_version` that
/// describes it appears beside it" — holding at every site rather than at the
/// ones a run happens to reach.
/// `crates/agent-compose/tests/trace_format_stability.rs` is where the same
/// claim is made about real runs.
#[test]
fn every_delivery_surface_emits_the_version() {
    for (module, sites, what) in [
        (
            "cli.ts",
            3,
            "the completed and failed `--format json` records, and the trace file's envelope",
        ),
        (
            "serve.ts",
            1,
            "the status route's report, which the completion webhook posts too",
        ),
    ] {
        let source = fs::read_to_string(
            repository().join(format!("crates/compose-core/src/codegen/js/{module}")),
        )
        .expect("the emitted module is readable");
        let emitted = source.matches("trace_version: TRACE_VERSION").count();
        assert_eq!(
            emitted, sites,
            "`src/{module}` writes `trace_version` at {emitted} site(s); `docs/trace.md` \
             §1 promises {sites} — {what}"
        );
    }
}

/// `StoreRecord.op`'s vocabulary is documented too, though its union lives in
/// another module.
///
/// The field is typed `string` in `src/runtime.ts` because the union it draws on
/// — `src/stores.ts`'s `StoreOp` — is declared *above* the runtime in the module
/// graph (`stores.ts` imports `StoreRecord`, not the other way round), so naming
/// it there would invert that dependency. The consequence is that
/// [`every_enumeration_member_is_documented`] cannot see the vocabulary at all,
/// and `docs/trace.md` §10.1 counts `op` among the closed enumerations a reader
/// may rely on: an op added to the catalog is a version bump, and a bump nobody
/// knew to make is exactly what this file exists to prevent. So the union is
/// read from where it is.
#[test]
fn every_store_op_is_documented() {
    let stores =
        fs::read_to_string(repository().join("crates/compose-core/src/codegen/js/stores.ts"))
            .expect("the emitted store module is readable");
    let union = stores
        .lines()
        .find_map(|line| line.strip_prefix("export type StoreOp = "))
        .expect("`src/stores.ts` declares `StoreOp`");
    let ops = strings(union);
    assert!(
        ops.len() >= 7,
        "grammar 11.4's catalog is seven ops; `StoreOp` read as {ops:?}"
    );

    let document = specification();
    let sections = sections(&document);
    let held = introduces(&sections, "StoreRecord");
    let [home] = held.as_slice() else {
        panic!("`docs/trace.md` introduces `StoreRecord` in exactly one section");
    };
    let missing: Vec<&String> = ops.iter().filter(|op| !row_names(home, op)).collect();
    assert!(
        missing.is_empty(),
        "these store ops are in `src/stores.ts`'s `StoreOp` and in no row of {} — \
         `docs/trace.md` §10.1 makes the vocabulary something a reader may rely on, \
         so an op it does not name is an undocumented member of a closed \
         enumeration: {missing:?}",
        home.heading
    );
}

/// The field names this format writes free diagnostic text under.
///
/// Two spellings and no more: `error` on the three records that carry a failure,
/// and `detail` on the one that carries what a provider answered. Every other
/// field of the format is a name, a number, a boolean or a member of a closed
/// enumeration — so a field with one of these names is a message, and a message
/// is the one thing in a trace that can hold bytes from outside this process.
const MESSAGE_FIELDS: &[&str] = &["error", "detail"];

/// Every message field is classified in the section that warns about untrusted
/// text.
///
/// `docs/trace.md` §11.1 tells an operator which fields can hold bytes the run
/// did not compose — a rejected response body, a child's stderr, what a provider
/// answered — and a table that is *nearly* the set is worse than none: a reader
/// takes the omitted field for runtime-composed text and renders it unescaped.
/// `DispatchRecord.error` is the field that showed this is worth checking rather
/// than reviewing: it carries a failed item's activity text verbatim, and under
/// `on_item_error: skip` it is the **only** field that text reaches, because the
/// run survives and no entry's `error` is written for it.
///
/// Name-based rather than type-based, for the reason [`MESSAGE_FIELDS`] gives: a
/// message is recognizable by what it is called here, and a *third* spelling
/// would slip past this — so the constant is the thing to extend when the format
/// grows one, and its doc comment says so.
#[test]
fn every_message_field_is_classified_as_untrusted_text() {
    let reached = reachable(&runtime());
    let mut messages: Vec<String> = Vec::new();
    for (name, declaration) in &reached {
        for field in &declaration.fields {
            if MESSAGE_FIELDS.contains(&field.as_str()) {
                messages.push(format!("{name}.{field}"));
            }
        }
    }
    assert!(
        messages.contains(&"TraceEntry.error".to_string()),
        "the reader found the format's message fields at all — it read {messages:?}, \
         and the entry's own `error` is the one every reader of this format meets"
    );

    let document = specification();
    let sections = sections(&document);
    let home = sections
        .iter()
        .find(|section| section.heading.starts_with("### 11.1 "))
        .expect("`docs/trace.md` has a §11.1, which is where untrusted text is classified");
    let missing: Vec<&String> = messages
        .iter()
        .filter(|message| !row_names(home, message))
        .collect();
    assert!(
        missing.is_empty(),
        "these fields carry free text and have no row in {} — an operator reading \
         that table takes it for the whole set, so a message field missing from it \
         is text from outside this process that nothing warned about: {missing:?}",
        home.heading
    );
}

/// An activity that failed names the `${ENV}` reference its author wrote, never
/// the value it resolved to.
///
/// `docs/trace.md` §11.1 is a promise about a **public** surface: no resolved
/// environment value appears in the format. Two messages are where one could —
/// grammar 4.3 class 2 makes an `http:` binding's `url` and an `exec:` binding's
/// `command` interpolable, and both are quoted when the activity is refused — so
/// both quote [`asWritten`] rather than the resolved string beside them.
/// `crates/agent-compose/tests/trace_format_stability.rs` runs the `exec:` half
/// against a real graph; this is the half that holds at both sites whether a
/// fixture reaches them or not.
#[test]
fn an_activity_failure_quotes_the_reference_rather_than_the_resolved_value() {
    let source = runtime();
    // The needles are template-literal source, where the message's own backticks
    // are escaped — which is also what keeps them from matching anything else.
    for (site, written, resolved) in [
        (
            "runHttp",
            r"\`${asWritten(binding.url)}\` answered ",
            r"\`${url}\` answered ",
        ),
        (
            "runExec",
            r"\`${asWritten(binding.command)}\` exited ",
            r"\`${command}\` exited ",
        ),
    ] {
        assert!(
            source.contains(written),
            "`{site}` reports a refused activity as `{written}…`, which is what keeps \
             a resolved `${{ENV}}` value out of `TraceEntry.error` (`docs/trace.md` §11.1)"
        );
        assert!(
            !source.contains(resolved),
            "`{site}` quotes the **resolved** value in `{resolved}…`; that value reaches \
             `TraceEntry.error` and the trace file, which `docs/trace.md` §11.1 says it \
             does not"
        );
    }
}

/// …and so does a failure the runtime never composed itself.
///
/// The three sites of `docs/trace.md` §11.2. Each is a place the **platform**
/// writes the message — a spawn the OS refused, a string neither `URL` nor
/// `fetch` could parse — and every one of those messages embeds the resolved
/// value: `spawn /opt/tokens/rg ENOENT` on Node, `"secret/reports" cannot be
/// parsed as a URL.` on Bun, `Failed to parse URL from https://secret…` on Node
/// again. All three reach `TraceEntry.error` (the last through
/// `Refusal.detail`), so §11.1's promise holds only while each is caught and
/// restated.
///
/// Two needles per site, and the pair is the point: the **restatement** must be
/// there, and the **unguarded** call it replaced must not. Asserting only the
/// first would pass on a runtime that had both — a catch nothing reaches, beside
/// the platform call that still throws past it.
///
/// Both needles are looked for in the **function's own body** rather than in the
/// file, for the reason [`row_names`] is scoped: `await fetch(url, {` is the
/// unguarded spelling in [`send`] and the guarded one in `runHttp`, where `url`
/// is already a parsed `URL`, so a whole-file search would report a hole that is
/// not there — or, worse, stop reporting one that is once the other site moves.
#[test]
fn a_failure_the_platform_worded_is_restated_rather_than_quoted() {
    let source = runtime();
    for (site, header, restated, unguarded, what) in [
        (
            "runExec",
            "export async function runExec(",
            r"\`${asWritten(binding.command)}\` could not be run",
            "const result = await new Promise<",
            "a command the platform refused to spawn quotes the resolved path",
        ),
        (
            "runHttp",
            "export async function runHttp(",
            r"\`${asWritten(binding.url)}\` is not a URL once its ",
            "const url = new URL(interpolate(binding.url));",
            "`new URL` quotes the resolved string it could not parse",
        ),
        (
            "send",
            "async function send(",
            r"\`${model.provider.address}\`'s resolved \`base_url:\` is not a URL",
            "await fetch(url, {",
            "`fetch` quotes the endpoint it could not parse, which is built from a \
             `base_url:` grammar 4.3 class 1 makes a whole-value `${ENV}` reference",
        ),
    ] {
        let body = function_body(&source, header);
        assert!(
            body.contains(restated),
            "`{site}` no longer restates its platform failure as `{restated}…`; without \
             it {what}, and that value reaches `TraceEntry.error` and the trace file \
             (`docs/trace.md` §11.2)"
        );
        assert!(
            !body.contains(unguarded),
            "`{site}` calls `{unguarded}…` with nothing catching it, so {what} \
             (`docs/trace.md` §11.2)"
        );
    }
}

/// One function of the emitted runtime, from its header to the closing brace in
/// the first column — which is where a formatted declaration ends, the same
/// boundary [`declarations`] reads an interface body to.
fn function_body(source: &str, header: &str) -> String {
    let mut lines = source.lines().skip_while(|line| !line.starts_with(header));
    let opened = lines
        .next()
        .unwrap_or_else(|| panic!("`src/runtime.ts` declares `{header}…`"));
    let mut held = String::from(opened);
    for line in lines {
        held.push('\n');
        held.push_str(line);
        if line == "}" {
            return held;
        }
    }
    panic!("`{header}…` has no closing brace in the first column")
}

/// The reader behind every assertion above reads what this repository's own
/// declarations look like — and refuses what it cannot read.
///
/// The refusal is the load-bearing half. A body line skipped rather than
/// rejected is a field the inventory quietly stops covering, which is exactly
/// the drift this file exists to catch, so a member shape nobody taught it is a
/// panic rather than a `continue`.
#[test]
fn the_declaration_reader_reads_members_comments_and_inline_objects() {
    let read = declarations(
        r#"
export interface Sample {
  /**
   * A doc comment, with a `field: value` shape inside it that is not a member.
   */
  readonly named: string;
  // A line comment mentioning outcome: "nope".
  readonly optional?: "one" | "two";
  readonly nested?: { readonly key: string; readonly used: number };
  readonly other: OtherRecord;
}

export type Vocabulary = "left" | "right";
"#,
    );

    let sample = read.get("Sample").expect("the interface was read");
    assert_eq!(
        sample.fields,
        ["named", "optional", "nested", "key", "used", "other"]
    );
    assert_eq!(sample.literals, ["one", "two"]);
    assert!(
        sample.unreadable.is_empty(),
        "every line of this body is one of the two shapes: {:?}",
        sample.unreadable
    );
    assert!(
        sample.references.contains(&"OtherRecord".to_string()),
        "a member's own type is a reference to follow: {:?}",
        sample.references
    );
    assert!(
        !sample.literals.contains(&"nope".to_string()),
        "a literal inside a comment is not part of the format: {:?}",
        sample.literals
    );

    let vocabulary = read.get("Vocabulary").expect("the alias was read");
    assert_eq!(vocabulary.literals, ["left", "right"]);
    assert!(vocabulary.fields.is_empty());
}

/// A record type the envelope reaches is inventoried whether or not the runtime
/// exports it.
///
/// `export` says who outside the module may name a type; it says nothing about
/// whether a trace carries it. A reader that keyed on the keyword would drop an
/// unexported record out of the map, and [`reachable`] follows only what the map
/// holds — so the walk would neither refuse it nor follow it, and every field it
/// declares would ship on a versioned public surface with no row in
/// `docs/trace.md` and CI green. `src/runtime.ts` declares unexported interfaces
/// already (`ResultIssue` among them), so the shape below is one refactor away
/// rather than hypothetical: retyping a member as a locally-declared record is
/// an ordinary thing to do, and it must not take the record out of the
/// inventory.
#[test]
fn a_record_the_envelope_reaches_is_inventoried_though_the_runtime_does_not_export_it() {
    let source = r#"
export interface TraceDocument {
  readonly status: "completed";
  readonly entries: readonly TraceEntry[];
}

export interface TraceEntry {
  readonly outcome: "completed";
  readonly budget?: Budget;
}

interface Budget {
  readonly key: string;
  readonly used: number;
}
"#;

    let reached: BTreeSet<String> = reachable(source).into_keys().collect();
    assert!(
        reached.contains("Budget"),
        "an unexported record the envelope reaches is part of the format: {reached:?}"
    );

    // …and is held to the document like any other, which is the half that would
    // have been silently skipped: no section introduces `Budget`, so both of its
    // fields are reported undocumented.
    let document = "\
## 2. The envelope

`TraceDocument`.

| field | meaning |
|---|---|
| `status` | whether the run produced an answer, `\"completed\"` or not |
| `entries` | every entry the run recorded |

## 3. Entries

`TraceEntry`.

| field | meaning |
|---|---|
| `outcome` | `\"completed\"`, and the rest |
| `budget` | what the edge's budget had spent |
";
    assert_eq!(
        undocumented_fields(document, &reachable(source)),
        [
            "Budget.key (no single section of the document introduces `Budget`)",
            "Budget.used (no single section of the document introduces `Budget`)",
        ],
        "the fields of an unexported record are checked against `docs/trace.md` too"
    );
}

/// …and an `extends` clause is followed, which is the one reference that is not
/// a member's type.
#[test]
fn the_declaration_reader_follows_an_extends_clause() {
    let read = declarations(
        r#"
export interface Narrowed extends Wider {
  readonly condition: string;
}
"#,
    );
    assert!(
        read["Narrowed"].references.contains(&"Wider".to_string()),
        "`Failover extends Refusal` is how a refusal's fields reach the format"
    );
}

/// A member shape the reader does not understand fails — but only where the
/// format reaches it.
///
/// Both halves matter. The runtime declares far more than a trace carries, and a
/// reader that refused every shape it had not been taught would fail on a method
/// signature three sections away from anything traced; a reader that skipped
/// them everywhere would quietly stop covering a field of a record that *is*
/// traced. So an unreadable line is kept where it was found and refused at the
/// boundary of the format.
#[test]
#[should_panic(
    expected = "is part of the trace format and has a member this inventory cannot read"
)]
fn an_unreadable_member_of_a_traced_record_is_a_failure() {
    let _ = reachable(
        r#"
export interface TraceDocument {
  readonly entries: readonly TraceEntry[];
}

export interface TraceEntry {
  readonly split:
    | "one"
    | "two";
}
"#,
    );
}

/// …and one outside it is not, because the runtime is full of them.
#[test]
fn an_unreadable_member_outside_the_format_is_carried_rather_than_refused() {
    let read = declarations(
        r#"
export interface ResultSchema<T> {
  safeParse(value: unknown): {
    readonly success: boolean;
  };
}
"#,
    );
    let schema = read.get("ResultSchema").expect("the interface was read");
    assert_eq!(
        schema.unreadable,
        ["safeParse(value: unknown): {", "};"],
        "the shape is kept rather than refused; `reachable` is what refuses it"
    );
}
