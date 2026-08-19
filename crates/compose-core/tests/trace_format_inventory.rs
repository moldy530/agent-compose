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
//! enumerations those fields declare, must be named in `docs/trace.md`. A field
//! added to `src/runtime.ts` without a row fails here.
//!
//! The other direction is deliberately not checked. A specification says more
//! than the type declarations do — presence rules, orders, what a reader may rely
//! on — so "every backticked word in the document is a field" is not a property
//! that holds, and asserting it would push the prose toward a schema dump.
//!
//! # How the declarations are read
//!
//! By line, over the emitted `src/runtime.ts`, which is a formatted file this
//! repository owns: an `export interface` header, a body of doc comments and
//! one-line members, a closing brace. A body line that is neither a comment nor
//! a member the reader understands is kept as **unreadable**, and a declaration
//! the format actually reaches carrying one is a failure — see [`reachable`].
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

/// One exported declaration of the emitted runtime.
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

/// Every `export interface` and `export type` in `source`, by name.
fn declarations(source: &str) -> BTreeMap<String, Declaration> {
    let mut found: BTreeMap<String, Declaration> = BTreeMap::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut at = 0usize;
    while at < lines.len() {
        let line = lines[at];
        if let Some(header) = line.strip_prefix("export interface ") {
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
                "`export interface {name}` has no closing brace in the first column"
            );
            found.insert(name, declaration);
        } else if let Some(header) = line
            .strip_prefix("export type ")
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
                .unwrap_or_else(|| panic!("`export type {header}` has no `=`"));
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
        .unwrap_or_else(|| panic!("`export interface {header}` does not open its body on one line"))
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
            panic!("`{name}` is named by the trace format but is not exported by the runtime");
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
fn names(document: &str, token: &str) -> bool {
    document.contains(&format!("`{token}`"))
}

/// Whether the document names `member` as a value of a closed enumeration.
///
/// Stricter than [`names`], and deliberately: a member is a JSON **string**, so
/// the document writes it with its quotes — `` `"detached"` `` — and requiring
/// them is what keeps `"node"` from being read as satisfied by the twenty places
/// the word `node` is backticked as itself.
fn names_member(document: &str, member: &str) -> bool {
    document.contains(&format!("`\"{member}\"`"))
}

/// Every record type the trace format reaches is specified.
#[test]
fn every_trace_record_type_is_documented() {
    let document = specification();
    let missing: Vec<String> = reachable(&runtime())
        .into_keys()
        .filter(|name| !names(&document, name))
        .collect();
    assert!(
        missing.is_empty(),
        "these trace record types are declared by `src/runtime.ts` and named nowhere \
         in `docs/trace.md`: {missing:?}"
    );
}

/// Every field of every one of them is specified.
#[test]
fn every_trace_record_field_is_documented() {
    let document = specification();
    let mut missing: Vec<String> = Vec::new();
    for (name, declaration) in reachable(&runtime()) {
        for field in &declaration.fields {
            if !names(&document, field) {
                missing.push(format!("{name}.{field}"));
            }
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "these trace fields are recorded by `src/runtime.ts` and named nowhere in \
         `docs/trace.md`, so a reader pinning `trace_version` has not been told about \
         them: {missing:?}"
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
    let document = specification();
    let mut missing: Vec<String> = Vec::new();
    for (name, declaration) in reachable(&runtime()) {
        for literal in &declaration.literals {
            if !names_member(&document, literal) {
                missing.push(format!("{name}: \"{literal}\""));
            }
        }
    }
    missing.sort();
    assert!(
        missing.is_empty(),
        "these enumeration members are emitted by `src/runtime.ts` and named nowhere \
         in `docs/trace.md`: {missing:?}"
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
