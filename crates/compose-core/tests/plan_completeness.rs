//! What a plan may be silent about, stated as a property rather than as a
//! golden.
//!
//! Every other test of this command asserts a *particular* report: this pair of
//! specs produces these lines. That is the right shape for what a plan says, and
//! it is structurally blind to what a plan **fails** to say — a rule that
//! swallows a whole class of edit produces no line to assert against, so no
//! golden anywhere fails and CI is green on a diff that reports two compositions
//! agree when they do not. `docs/plan.md` §12 promises a machine surface; a
//! reader branching on it needs the silence to be exhaustively accounted for,
//! not merely spot-checked.
//!
//! So this file states the account itself:
//!
//! > The three structural sections are empty **if and only if** the two
//! > artifacts agree, once everything `docs/plan.md` §11 excuses is taken out of
//! > them.
//!
//! Both directions are load bearing, and each fails a different way:
//!
//! * **left to right** is completeness — a difference the plan says nothing
//!   about, which is the plan being wrong about the composition. An author's
//!   literal array of objects compared as though its entries were a keyed map is
//!   exactly this: reverse `default: [{name: one}, {name: two}]` and every
//!   caller is handed a different value, with nothing in the report;
//!   [`compared`] does not sort that array, so the property fails on it;
//! * **right to left** is quiet — a line about an edit nobody made. Reordering
//!   an `env:` map, a node's `input:` bindings or a flow's `nodes:` changes the
//!   *layout* of the generated project and nothing it decides, and §11 is where
//!   each of those is accounted for. [`compared`] canonicalizes exactly those,
//!   so a plan that started reporting one would fail the property here.
//!
//! # `compared` is written twice on purpose
//!
//! [`compared`] restates §11 independently of `crates/compose-core/src/plan/`.
//! It is the same rules — take the source regions out, sort the sets, sort the
//! arrays the generated code looks up by name, drop the keys no section owns —
//! written from the document rather than shared with the code under test, which
//! is what keeps the property from agreeing with a bug by construction.
//!
//! # Where the validation section is
//!
//! Outside the biconditional, deliberately. `validation` is the difference
//! between two check-phase reports **plus** what resolving each side said, and a
//! resolution warning is not part of the artifact — so a pair whose artifacts
//! agree can legitimately differ there. What is asserted about it instead is
//! that it never *saves* a case: the three structural sections are what the
//! property is stated over, so an edit hidden from them fails here whatever the
//! validator happened to notice.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Composition, Ir, Plan, plan, resolve};
use serde_json::{Map, Value};

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// A scratch directory of this test's own, cleaned out before use.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("plan-completeness-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

// ---------------------------------------------------------------------------
// The document's §11, restated over the artifact.
// ---------------------------------------------------------------------------

/// Top-level keys of the artifact no section of a plan owns (`docs/plan.md`
/// §11, §2.2).
///
/// `sources` is about file layout, `entrypoint` and `ir_version` are the same on
/// both sides of nearly every comparison and §2.2 carries the useful ones
/// instead, `spec_version` is carried rather than compared, and `target` is
/// §2.2's as well.
const UNOWNED: &[&str] = &[
    "entrypoint",
    "ir_version",
    "sources",
    "spec_version",
    "target",
];

/// Keys of the deploy layer no section owns: the file a layer was read from, the
/// target §2.2 carries, and the one section `local` cannot declare.
const UNOWNED_DEPLOY: &[&str] = &["source", "storage_backends", "target"];

/// The keys whose arrays are memberships, whose order is never compared
/// (`docs/plan.md` §3, §11).
const SETS: &[&str] = &["expect_exit", "expect_status", "optional", "route_on"];

/// The keys whose arrays the generated code looks entries up in **by name**, so
/// that a reordering is a change to the layout of the emitted project and to
/// nothing the composition decides (`docs/plan.md` §5, §11).
///
/// Each is paired with the key its entries name themselves by. `fields` and
/// `variants` are deliberately **not** here: both reach the JSON Schema the
/// model is handed as written, so their order is reported and this must not
/// canonicalize it away.
const BY_NAME: &[(&str, &[&str])] = &[
    ("entries", &["name", "field"]),
    ("env", &["name"]),
    ("headers", &["name"]),
    ("nodes", &["id"]),
    ("routes", &["tag"]),
];

/// The artifact with everything `docs/plan.md` §11 excuses taken out of it.
///
/// Two artifacts whose `compared` forms agree differ in nothing a plan reports;
/// two whose forms differ must produce at least one structural record. That is
/// the whole property, and everything in this function is one clause of §11.
fn compared(ir: &Ir) -> Value {
    let mut value = serde_json::to_value(ir).expect("the artifact is representable as JSON");
    if let Value::Object(map) = &mut value {
        for key in UNOWNED {
            map.remove(*key);
        }
        if let Some(Value::Object(deploy)) = map.get_mut("deploy") {
            for key in UNOWNED_DEPLOY {
                deploy.remove(*key);
            }
        }
    }
    canonical("", value)
}

/// The same value with its source regions removed and its excused orders made
/// canonical. `key` is the object key this value was found under, which is what
/// classifies an array here.
fn canonical(key: &str, value: Value) -> Value {
    match value {
        Value::Object(map) => {
            // A `Spanned<T>` reaches the artifact as `{value, span}`, and is
            // unwrapped so that a path names `model` rather than `model.value`.
            if map.len() == 2
                && let Some(inner) = map.get("value")
                && map.get("span").and_then(Value::as_str).is_some_and(is_span)
            {
                return canonical(key, inner.clone());
            }
            let mut kept = Map::new();
            for (key, value) in map {
                if key == "span" && value.as_str().is_some_and(is_span) {
                    continue;
                }
                let value = canonical(&key, value);
                kept.insert(key, value);
            }
            Value::Object(kept)
        }
        Value::Array(items) => {
            let mut items: Vec<Value> =
                items.into_iter().map(|item| canonical(key, item)).collect();
            if SETS.contains(&key) && flat(&items) {
                items.sort_by_key(Value::to_string);
            } else if let Some((_, names)) = BY_NAME.iter().find(|(held, _)| *held == key) {
                items.sort_by_key(|item| (named(item, names), item.to_string()));
            } else if key == "edges" {
                items = grouped(items);
            }
            Value::Array(items)
        }
        scalar => scalar,
    }
}

/// Whether every element is a scalar of one kind — all strings, or all numbers.
///
/// Every membership of [`SETS`] the grammar can produce is exactly that:
/// `optional:` and `route_on:` hold names, `expect_exit:` and `expect_status:`
/// hold integers. The document states the rule over the **key** an array sits
/// under (§3, §11), and the one place the key alone is not enough is an author's
/// own array written under one of the four names inside a `settings:` or a
/// `default:`, which is data rather than one of the grammar's memberships.
///
/// The compiler draws that line at a uniform flat array
/// (`crates/compose-core/src/plan/diff.rs`), and this restatement draws it in the
/// same place on purpose. The two are meant to be independent, not to disagree:
/// a mixed `optional: [1, "a"]` is unreachable through the grammar, so on every
/// composition the two readings decide alike — and a weaker line here (anything
/// that is not an object, say) would report that author's array as canonicalized
/// while the code compares it by position, failing the property over code
/// behaving exactly as documented.
fn flat(items: &[Value]) -> bool {
    items.iter().all(Value::is_string) || items.iter().all(Value::is_number)
}

/// The first of these keys this entry carries as a string, for sorting.
fn named(item: &Value, names: &[&str]) -> String {
    names
        .iter()
        .find_map(|name| item.get(*name).and_then(Value::as_str))
        .unwrap_or_default()
        .to_string()
}

/// A flow's edges with the **groups** put in one order and the order *within*
/// each group left alone.
///
/// Grammar 7.3 evaluates a node's outgoing edges in declaration order, so a swap
/// between two edges of one node decides which fires and is reported
/// (`docs/plan.md` §5) — this must not hide it. An edge moved past an edge of a
/// *different* node changes nobody's precedence and is not reported, which is
/// what putting the groups in `from` order excuses.
fn grouped(items: Vec<Value>) -> Vec<Value> {
    let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for item in items {
        let from = item
            .get("from")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        groups.entry(from).or_default().push(item);
    }
    groups.into_values().flatten().collect()
}

/// Whether this string is a source region as `crate::ir::leaf` writes one:
/// `<file>:<line>:<col>..<line>:<col>`, with a non-empty file.
///
/// Written out here rather than borrowed from the compiler, so that the property
/// does not inherit the code's own idea of what a span looks like. A file name
/// may hold a `:`, so the coordinates are taken from the right.
fn is_span(text: &str) -> bool {
    let Some((start, end)) = text.rsplit_once("..") else {
        return false;
    };
    let Some((start, start_column)) = start.rsplit_once(':') else {
        return false;
    };
    let Some((file, start_line)) = start.rsplit_once(':') else {
        return false;
    };
    let Some((end_line, end_column)) = end.split_once(':') else {
        return false;
    };
    !file.is_empty()
        && [start_line, start_column, end_line, end_column]
            .iter()
            .all(|held| held.parse::<u32>().is_ok())
}

// ---------------------------------------------------------------------------
// The property.
// ---------------------------------------------------------------------------

/// How many records the three structural sections hold.
fn structural(plan: &Plan) -> usize {
    plan.components.len() + plan.topology.len() + plan.interfaces.len()
}

/// The artifact of a project that must resolve cleanly enough to be planned.
///
/// Check-phase diagnostics are content rather than a refusal (`docs/plan.md`
/// §1), so a composition the validator rejects still has an artifact; what has
/// to succeed here is resolution.
fn artifact(entrypoint: &Path) -> Option<Ir> {
    resolve(entrypoint).ir
}

/// The property, over one pair: the three structural sections are empty exactly
/// when the two artifacts agree on everything §11 does not excuse.
#[track_caller]
fn holds(what: &str, before: &Ir, after: &Ir) -> usize {
    let plan = plan(
        Composition {
            entrypoint: "before/main.yml",
            ir: before,
            resolution: &[],
        },
        Composition {
            entrypoint: "after/main.yml",
            ir: after,
            resolution: &[],
        },
    );
    let reported = structural(&plan);
    let agree = compared(before) == compared(after);
    assert_eq!(
        reported == 0,
        agree,
        "{what}: {reported} structural record(s) against artifacts that {}. {}",
        if agree { "agree" } else { "differ" },
        if agree {
            "A plan reported an edit nobody made: `docs/plan.md` §11 says this difference is not \
             compared, so no record may name it."
        } else {
            "A plan said two compositions agree when they do not: the artifacts differ in \
             something `docs/plan.md` §11 does not excuse, and no section named it."
        }
    );
    reported
}

// ---------------------------------------------------------------------------
// Corpus 1: every pair the CLI's own golden corpus is built from.
// ---------------------------------------------------------------------------

/// The plan command's fixture corpus, which is `crates/agent-compose/tests`'.
///
/// Shared rather than duplicated: those pairs are chosen one per sentence of
/// `docs/plan.md`, so they are exactly the pairs whose reports are worth holding
/// to the property that produced them.
fn corpus() -> PathBuf {
    repository().join("crates/agent-compose/tests/projects/plan")
}

/// Every pair of the corpus, by name.
fn corpus_pairs() -> Vec<String> {
    let mut found: Vec<String> = fs::read_dir(corpus())
        .expect("the plan corpus is readable")
        .map(|entry| entry.expect("a directory entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn every_pair_of_the_corpus_reports_exactly_the_differences_it_has() {
    let pairs = corpus_pairs();
    assert!(pairs.len() >= 10, "the corpus is populated: {pairs:?}");

    let mut skipped = Vec::new();
    let mut checked = 0usize;
    for name in &pairs {
        let before = artifact(&corpus().join(name).join("before/main.yml"));
        let after = artifact(&corpus().join(name).join("after/main.yml"));
        match (before, after) {
            (Some(before), Some(after)) => {
                holds(name, &before, &after);
                // …and the property is symmetric, because the diff is.
                holds(&format!("{name} (reversed)"), &after, &before);
                checked += 1;
            }
            _ => skipped.push(name.clone()),
        }
    }

    // A pair is skipped only because it exists to test a spec with no artifact,
    // and those are named for it. Anything else silently dropping out of this
    // corpus is the failure this assertion is here for.
    assert!(
        skipped.iter().all(|name| name.starts_with("unresolvable-")),
        "only the deliberately unresolvable pairs have no artifact, and these were skipped: \
         {skipped:?}"
    );
    assert_eq!(
        checked + skipped.len(),
        pairs.len(),
        "every pair is either checked or accounted for"
    );
}

// ---------------------------------------------------------------------------
// Corpus 2: one composition, edited one construct at a time.
// ---------------------------------------------------------------------------

/// One case: the two sides as edits to the composition they both start from.
struct Case {
    /// What the case is about, and what a failure names.
    what: &'static str,
    /// Applied to the before side.
    before: &'static [Edit],
    /// Applied to the after side.
    after: &'static [Edit],
    /// Whether the two are expected to differ in anything a plan reports.
    ///
    /// The property is a biconditional and would hold with both sides equal, so
    /// a mutation that silently failed to apply would pass it. This is what says
    /// which way each case is supposed to come out.
    differs: bool,
}

/// One textual edit to one file of the composition: exactly one occurrence,
/// replaced.
type Edit = (&'static str, &'static str, &'static str);

/// The composition every case starts from: `examples/triage-fanout`, which is
/// the widest one in the tree — a discriminator-routed `map` with a catch-all, a
/// `human` node, an inline `exec:` and an inline `http:`, kv and vector stores,
/// reduced channels, four kinds of trigger, and a deploy layer with placements.
const BASE: &str = "examples/triage-fanout";

/// A channel whose default is an author's literal array of objects, and the
/// same channel with the two elements the other way round.
///
/// Reversing it hands every caller a different value — the emitted schema reads
/// `.default([{"name":"beta",…},{"name":"alpha",…}])` — so it is a change. What
/// makes it worth a case of its own is that its elements are shaped exactly like
/// a field map's: a rule that classified an array by its elements rather than by
/// the key above it reports nothing here.
const SEEDS: &str = r#"  seeds:
    type: array
    max_items: 4
    items:
      type: object
      properties:
        name: { type: string }
        weight: { type: integer }
    default:
      - { name: alpha, weight: 1 }
      - { name: beta, weight: 2 }

  human_decision:"#;
const SEEDS_SWAPPED: &str = r#"  seeds:
    type: array
    max_items: 4
    items:
      type: object
      properties:
        name: { type: string }
        weight: { type: integer }
    default:
      - { name: beta, weight: 2 }
      - { name: alpha, weight: 1 }

  human_decision:"#;

/// A node's two input bindings, and the same two swapped. Dispatched on by name.
const BINDINGS: &str = r#"        pattern: "input.pattern"
        max_matches: "25""#;
const BINDINGS_SWAPPED: &str = r#"        max_matches: "25"
        pattern: "input.pattern""#;

/// An `exec:`'s environment, planted with a second entry so that there is an
/// order to reorder, and the same two swapped. Looked up by name by the child
/// process, so the order is nothing the composition decides.
const ENV: &str = r#"        env:
          CI: "true"
          TZ: "UTC""#;
const ENV_SWAPPED: &str = r#"        env:
          TZ: "UTC"
          CI: "true""#;

/// The same for an `http:` block's headers, which the request builder also looks
/// up by name. The `body:` below is what tells `announce`'s block from
/// `escalate`'s.
const ONE_HEADER: &str = r#"        headers:
          authorization: "Bearer ${QUEUE_TOKEN}"
        body:
          report: "state.report_normalized""#;
const HEADERS: &str = r#"        headers:
          authorization: "Bearer ${QUEUE_TOKEN}"
          x-triage-source: "compose"
        body:
          report: "state.report_normalized""#;
const HEADERS_SWAPPED: &str = r#"        headers:
          x-triage-source: "compose"
          authorization: "Bearer ${QUEUE_TOKEN}"
        body:
          report: "state.report_normalized""#;

/// The `dispatch` map's two named routes, and the same two swapped.
const ROUTES: &str = r#"          auto_fixable:
            node: agent.fixer
            max_concurrency: 4
            input:
              file: "finding.file"
              patch_hint: "finding.patch_hint"
            writes:
              patch: patches
          needs_human:
            node: tool.review_queue
            input:
              summary: "finding.summary"
              severity: "finding.severity""#;
const ROUTES_SWAPPED: &str = r#"          needs_human:
            node: tool.review_queue
            input:
              summary: "finding.summary"
              severity: "finding.severity"
          auto_fixable:
            node: agent.fixer
            max_concurrency: 4
            input:
              file: "finding.file"
              patch_hint: "finding.patch_hint"
            writes:
              patch: patches"#;

/// Two nodes of `flow.triage`, and the same two swapped. The graph is the
/// edges', not this mapping's (`docs/plan.md` §5).
const NODES: &str = r#"    scan:
      function: tool.repo_grep
      input:
        pattern: "input.pattern"
        max_matches: "25"

    classify:
      agent: agent.triage
      input:
        report: "state.report_normalized"
        matches: "state.matches""#;
const NODES_SWAPPED: &str = r#"    classify:
      agent: agent.triage
      input:
        report: "state.report_normalized"
        matches: "state.matches"

    scan:
      function: tool.repo_grep
      input:
        pattern: "input.pattern"
        max_matches: "25""#;

/// Two variants of `agent.triage`'s union, and the same two swapped. This order
/// is the order of the `oneOf` the model is handed.
const VARIANTS: &str = r#"          auto_fixable:
            file: { type: string }
            patch_hint: { type: string }
          needs_human:
            summary: { type: string }
            severity: { enum: [low, high, critical] }"#;
const VARIANTS_SWAPPED: &str = r#"          needs_human:
            summary: { type: string }
            severity: { enum: [low, high, critical] }
          auto_fixable:
            file: { type: string }
            patch_hint: { type: string }"#;

/// Two fields of `flow.triage`'s input surface, and the same two swapped. This
/// order is the order of a JSON Schema's `properties` and `required`.
const INPUTS: &str = r#"    report:
      description: The raw bug report.
      type: string
      min_length: 1
    pattern:
      description: Repository search pattern used to ground the triage agent.
      type: string"#;
const INPUTS_SWAPPED: &str = r#"    pattern:
      description: Repository search pattern used to ground the triage agent.
      type: string
    report:
      description: The raw bug report.
      type: string
      min_length: 1"#;

/// Two outgoing edges of one node, and the same two swapped. Grammar 7.3 takes
/// the first whose guard passes, so this decides which fires.
const SIBLINGS: &str = r#"    - { from: approve, to: end, when: "approve.output.decision == 'approve'" }
    - { from: approve, to: escalate, when: "approve.output.decision == 'reject'" }"#;
const SIBLINGS_SWAPPED: &str = r#"    - { from: approve, to: escalate, when: "approve.output.decision == 'reject'" }
    - { from: approve, to: end, when: "approve.output.decision == 'approve'" }"#;

/// Two edges of **different** nodes, and the same two swapped. Neither node's
/// precedence moves.
const STRANGERS: &str = r#"    - { from: dispatch, to: verify }
    - { from: announce, to: verify }"#;
const STRANGERS_SWAPPED: &str = r#"    - { from: announce, to: verify }
    - { from: dispatch, to: verify }"#;

/// One edit per construct whose comparison rule is not the obvious one — and,
/// beside each, the edit that looks like it and must come out the other way.
const CASES: &[Case] = &[
    // --- The two open surfaces, where the author chooses the keys. -----------
    Case {
        what: "an author's literal array of objects reordered",
        before: &[("main.yml", "  human_decision:", SEEDS)],
        after: &[("main.yml", "  human_decision:", SEEDS_SWAPPED)],
        differs: true,
    },
    Case {
        what: "a literal scalar default changed",
        before: &[],
        after: &[("main.yml", "    default: approve", "    default: reject")],
        differs: true,
    },
    // --- The four memberships, whose order is never compared. ----------------
    Case {
        what: "a set of accepted exit statuses reordered",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        expect_exit: [0, 1]",
            "        expect_exit: [1, 0]",
        )],
        differs: false,
    },
    Case {
        what: "a set of accepted exit statuses grown",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "        expect_exit: [0, 1]",
            "        expect_exit: [0, 1, 2]",
        )],
        differs: true,
    },
    // --- The arrays the generated code looks up by name. ---------------------
    Case {
        what: "two node input bindings swapped",
        before: &[],
        after: &[("flows/triage.yml", BINDINGS, BINDINGS_SWAPPED)],
        differs: false,
    },
    Case {
        what: "two map routes swapped",
        before: &[],
        after: &[("flows/triage.yml", ROUTES, ROUTES_SWAPPED)],
        differs: false,
    },
    Case {
        what: "two nodes swapped in the declaring mapping",
        before: &[],
        after: &[("flows/triage.yml", NODES, NODES_SWAPPED)],
        differs: false,
    },
    Case {
        what: "two environment entries swapped",
        before: &[(
            "flows/triage.yml",
            "        env:\n          CI: \"true\"",
            ENV,
        )],
        after: &[(
            "flows/triage.yml",
            "        env:\n          CI: \"true\"",
            ENV_SWAPPED,
        )],
        differs: false,
    },
    Case {
        what: "two request headers swapped",
        before: &[("flows/triage.yml", ONE_HEADER, HEADERS)],
        after: &[("flows/triage.yml", ONE_HEADER, HEADERS_SWAPPED)],
        differs: false,
    },
    Case {
        what: "an environment value changed",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "          CI: \"true\"",
            "          CI: \"false\"",
        )],
        differs: true,
    },
    // --- The two orders that reach the model. --------------------------------
    Case {
        what: "two union variants swapped",
        before: &[],
        after: &[("agents/triage.yml", VARIANTS, VARIANTS_SWAPPED)],
        differs: true,
    },
    Case {
        what: "two fields of a flow's input surface swapped",
        before: &[],
        after: &[("flows/triage.yml", INPUTS, INPUTS_SWAPPED)],
        differs: true,
    },
    // --- Edges, whose precedence is per source node. --------------------------
    Case {
        what: "two edges of one node swapped",
        before: &[],
        after: &[("flows/triage.yml", SIBLINGS, SIBLINGS_SWAPPED)],
        differs: true,
    },
    Case {
        what: "two edges of different nodes swapped",
        before: &[],
        after: &[("flows/triage.yml", STRANGERS, STRANGERS_SWAPPED)],
        differs: false,
    },
    Case {
        what: "an edge retargeted",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "    - { from: verify, to: summarize }",
            "    - { from: verify, to: remember }",
        )],
        differs: true,
    },
    // --- A default written out, which the artifact holds and a plan reports. --
    Case {
        what: "a default written out as the value the grammar already supplies",
        before: &[],
        after: &[(
            "agents/triage.yml",
            "  stores: [store.docs]",
            "  stores: [store.docs]\n  max_tool_iterations: 8",
        )],
        differs: true,
    },
    // --- The three leaves the artifact keeps as source text (§11). ------------
    Case {
        what: "a duration respelled",
        before: &[],
        after: &[("flows/triage.yml", "timeout: 24h", "timeout: 1440m")],
        differs: true,
    },
    Case {
        what: "a guard respelled",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "when: \"approve.output.decision == 'reject'\"",
            "when: \"approve.output.decision=='reject'\"",
        )],
        differs: true,
    },
    // --- Files, which are authoring UX and nothing the composition does. ------
    Case {
        what: "an import list reordered",
        before: &[],
        after: &[(
            "main.yml",
            "  - providers.yml\n  - models.yml",
            "  - models.yml\n  - providers.yml",
        )],
        differs: false,
    },
    Case {
        what: "a comment and a blank line inserted",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "  edges:",
            "  # Every edge, in the order grammar 7.3 evaluates them.\n\n  edges:",
        )],
        differs: false,
    },
    // --- The rest of the surfaces, one edit each. -----------------------------
    Case {
        what: "a policy default retimed",
        before: &[],
        after: &[("main.yml", "  timeout: 90s", "  timeout: 45s")],
        differs: true,
    },
    Case {
        what: "a fan-out bound lowered",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "max_concurrency: 8",
            "max_concurrency: 6",
        )],
        differs: true,
    },
    Case {
        what: "an argv entry added",
        before: &[],
        after: &[(
            "flows/triage.yml",
            "args: [\"--fast\"]",
            "args: [\"--fast\", \"--quiet\"]",
        )],
        differs: true,
    },
    Case {
        what: "a trigger route moved",
        before: &[],
        after: &[("triggers.yml", "path: /reports", "path: /v1/reports")],
        differs: true,
    },
    Case {
        what: "a placement runtime changed",
        before: &[],
        after: &[(
            "deploy/local.yml",
            "  flow.triage:\n    runtime: colocated",
            "  flow.triage:\n    runtime: isolated",
        )],
        differs: true,
    },
    Case {
        what: "a channel bound narrowed",
        before: &[],
        after: &[("main.yml", "    max_items: 200", "    max_items: 100")],
        differs: true,
    },
];

/// Copy the base composition into `into`, then apply the edits.
fn planted(into: &Path, edits: &[Edit]) {
    copy(&repository().join(BASE), into);
    for (file, from, to) in edits {
        let path = into.join(file);
        let text = fs::read_to_string(&path).expect("a file of the base composition is readable");
        assert_eq!(
            text.matches(from).count(),
            1,
            "{file}: the text to edit appears exactly once:\n{from}"
        );
        fs::write(&path, text.replace(from, to)).expect("can write the edited file");
    }
}

/// Copy a directory tree.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("can create a directory");
    for entry in fs::read_dir(from).expect("the source directory is readable") {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("can copy a file");
        }
    }
}

#[test]
fn one_composition_edited_one_construct_at_a_time_reports_every_edit_and_no_other() {
    for (at, case) in CASES.iter().enumerate() {
        let before = scratch(&format!("{at}-before"));
        let after = scratch(&format!("{at}-after"));
        planted(&before, case.before);
        planted(&after, case.after);

        let old = artifact(&before.join("main.yml"))
            .unwrap_or_else(|| panic!("{}: the before side resolves", case.what));
        let new = artifact(&after.join("main.yml"))
            .unwrap_or_else(|| panic!("{}: the after side resolves", case.what));

        let reported = holds(case.what, &old, &new);
        assert_eq!(
            reported > 0,
            case.differs,
            "{}: {reported} structural record(s), and the case says the two {}",
            case.what,
            if case.differs { "differ" } else { "agree" }
        );
        holds(&format!("{} (reversed)", case.what), &new, &old);

        let _ = fs::remove_dir_all(&before);
        let _ = fs::remove_dir_all(&after);
    }
}
