//! What the generated runtime **strips** is what the wire **refuses**
//! (PRD §9 resolved q55, ruling a).
//!
//! The ruling puts a lowering table on each (wire, mechanism): the constraint
//! keywords that decoder cannot compile, taken off the schema before the request
//! and folded into descriptions. A table like that is a claim about somebody
//! else's service, so it is written down twice — once in the emitted runtime,
//! which acts on it, and once in the mock provider, which stands in for the
//! service and refuses what the service refuses — and this is the test that
//! keeps the two the same.
//!
//! It is here rather than in either crate because this is the only test target
//! that can see both: `agent-compose` depends on `compose-core` (whose
//! `codegen::runtime::SOURCE` is the emitted TypeScript) and dev-depends on
//! `mock-provider` (whose `lowering::enforced` is the enforced subset).
//!
//! What it buys is that an edit to one side alone fails loudly. Widening a table
//! because a vendor started accepting a keyword is one line in each file and a
//! green suite; widening only the runtime's is a projection the oracle no longer
//! checks, and widening only the mock's is a suite that refuses requests the
//! runtime still sends. Neither is discoverable anywhere else — the acceptance
//! suite would pass the first and fail the second with a message about a
//! schema rather than about a table.
//!
//! The **acceptance** half of the same proof lives in
//! `compiled_graph_acceptance.rs`: a real compiled graph sends a real
//! array-bearing schema at a mock that enforces these lists, on all three wires.

use std::collections::BTreeSet;

use compose_core::codegen::runtime::SOURCE;
use mock_provider::{OutputMechanism, Surface, lowering};

/// Which surfaces one emitted `Wire` reaches.
///
/// The runtime names three wires and the mock names four surfaces, because
/// Azure is a route rather than a body: `chat_completions` is one table serving
/// both OpenAI-shaped surfaces, and a row that ever split them would have to
/// split here first.
fn surfaces_of(wire: &str) -> Vec<Surface> {
    match wire {
        "messages" => vec![Surface::Anthropic],
        "chat_completions" => vec![Surface::OpenAi, Surface::AzureOpenAi],
        "responses" => vec![Surface::Responses],
        other => panic!("`{other}` is not a wire the emitted runtime names"),
    }
}

fn mechanism_of(spelling: &str) -> OutputMechanism {
    match spelling {
        "native" => OutputMechanism::Native,
        "forced_tool" => OutputMechanism::ForcedTool,
        other => panic!("`{other}` is not one of the two mechanisms (PRD resolved q53)"),
    }
}

/// Every row of the emitted `LOWERED_AWAY`, read out of the TypeScript.
///
/// Read rather than shared, because the point of the test is that two
/// independent statements agree: a constant one crate imported from the other
/// could not disagree, and so could not catch anything.
fn emitted_tables() -> Vec<(String, String, Vec<String>)> {
    let mut rows = Vec::new();
    for line in table_body("LOWERED_AWAY").lines() {
        let line = line.trim();
        let Some((wire, mechanisms)) = line.split_once(": {") else {
            continue;
        };
        let mechanisms = mechanisms
            .trim_end_matches(',')
            .trim_end()
            .trim_end_matches('}')
            .trim();
        for entry in mechanisms.split(',') {
            let Some((mechanism, named)) = entry.split_once(':') else {
                continue;
            };
            rows.push((
                wire.trim().to_string(),
                mechanism.trim().to_string(),
                keywords_of(named.trim()),
            ));
        }
    }
    assert_eq!(
        rows.len(),
        6,
        "the emitted `LOWERED_AWAY` no longer carries one entry per (wire, mechanism): {rows:?}"
    );
    rows
}

/// The keywords a table entry names — a `[]` written inline, or a named array.
fn keywords_of(named: &str) -> Vec<String> {
    if named == "[]" {
        return Vec::new();
    }
    quoted(&declaration_body(&format!(
        "const {named}: readonly string[] = ["
    )))
}

/// The text between the braces of one `const NAME: … = {` declaration.
fn table_body(name: &str) -> String {
    let opened = format!(
        "const {name}: Readonly<Record<Wire, Readonly<Record<OutputMechanism, readonly string[]>>>> = {{"
    );
    declaration_body(&opened)
}

/// The text a top-level declaration opens and closes over.
///
/// The emitted runtime is formatted, so a top-level `const` opens on its own
/// line and closes on a line that is exactly `};` — the same reading
/// `codegen::runtime`'s own pins take of a function body.
fn declaration_body(opened: &str) -> String {
    let mut lines = SOURCE.lines().skip_while(|line| !line.starts_with(opened));
    lines
        .next()
        .unwrap_or_else(|| panic!("the emitted runtime declares `{opened}…`"));
    let mut held = String::new();
    for line in lines {
        if line == "};" || line == "];" {
            return held;
        }
        held.push_str(line);
        held.push('\n');
    }
    panic!("`{opened}…` has no closing line in the first column")
}

/// Every double-quoted string in a block, with `//` comments taken out first.
fn quoted(block: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in block.lines() {
        let line = line.split_once("//").map_or(line, |(code, _)| code);
        let mut rest = line;
        while let Some(open) = rest.find('"') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('"') else { break };
            found.push(after[..close].to_string());
            rest = &after[close + 1..];
        }
    }
    found
}

/// The emitted lowering tables and the mock's enforced subsets are the same
/// lists (PRD §9 resolved q55, ruling a).
#[test]
fn what_the_runtime_strips_is_what_the_oracle_refuses() {
    let mut checked = 0usize;
    for (wire, mechanism, stripped) in emitted_tables() {
        let unique: BTreeSet<&str> = stripped.iter().map(String::as_str).collect();
        assert_eq!(
            unique.len(),
            stripped.len(),
            "the emitted `{wire}`/`{mechanism}` table names a keyword twice: {stripped:?}"
        );
        for surface in surfaces_of(&wire) {
            let refused: BTreeSet<&str> = lowering::enforced(surface, mechanism_of(&mechanism))
                .iter()
                .copied()
                .collect();
            assert_eq!(
                unique,
                refused,
                "the emitted runtime's `{wire}`/`{mechanism}` lowering table and what the mock \
                 provider refuses on {surface:?}/{mechanism} have drifted apart. Stripped but not \
                 refused: {:?}. Refused but not stripped: {:?}. Both are one line per keyword — \
                 `LOWERED_AWAY` in `crates/compose-core/src/codegen/js/runtime.ts` and `ENFORCED` \
                 in `crates/mock-provider/src/lowering.rs` — and a vendor that changes its subset \
                 changes both (PRD resolved q55, resolved q30's treadmill terms)",
                unique.difference(&refused).collect::<Vec<_>>(),
                refused.difference(&unique).collect::<Vec<_>>(),
            );
            checked += 1;
        }
    }
    assert_eq!(
        checked, 8,
        "one of the mock's (surface, mechanism) rows was never compared"
    );
}

/// The keyword grammar D10 makes unavoidable is on the rung resolved q53
/// prefers, everywhere it is compiled by something that cannot take it.
///
/// The regression this whole campaign is about, stated as a fact rather than as
/// a table: `max_items` is REQUIRED on every array inside a result schema
/// (grammar D10, §3.5), so a table that stopped stripping `maxItems` from a
/// native rung would put every real composition back on the 400 the ruling was
/// resolved from.
#[test]
fn every_native_rung_strips_the_keyword_d10_makes_mandatory() {
    for (wire, mechanism, stripped) in emitted_tables() {
        if mechanism != "native" {
            continue;
        }
        assert!(
            stripped.iter().any(|keyword| keyword == "maxItems"),
            "the emitted `{wire}`/native table does not strip `maxItems`, which grammar D10 puts \
             on every result-schema array: {stripped:?}"
        );
    }
}

/// …and the one row that is deliberately empty stays a claim.
///
/// The Messages wire's forced-tool rung sends the schema as an ordinary tool's
/// `input_schema`, which the API takes whole — so its table strips nothing, and
/// the mock accepts a `maxItems` there. An empty row that quietly grew would be
/// this runtime lowering a schema the wire never asked it to.
#[test]
fn the_messages_forced_tool_rung_lowers_nothing() {
    let rows = emitted_tables();
    let (_, _, stripped) = rows
        .iter()
        .find(|(wire, mechanism, _)| wire == "messages" && mechanism == "forced_tool")
        .expect("the emitted table carries a Messages forced-tool row");
    assert!(
        stripped.is_empty(),
        "the Messages forced-tool rung now strips {stripped:?}, but a schema rides it as a tool's \
         `input_schema` and the API takes it whole (PRD resolved q55)"
    );
    assert!(
        lowering::enforced(Surface::Anthropic, OutputMechanism::ForcedTool).is_empty(),
        "…and the oracle would refuse one anyway"
    );
}
