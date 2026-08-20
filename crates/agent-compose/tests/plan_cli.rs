//! `agent-compose plan`, end to end through the real binary.
//!
//! `docs/plan.md` is normative for a **public** machine surface and §12 is its
//! stability contract, so what pins it is whole documents rather than probes:
//! every report below is asserted **byte for byte**, in both formats, and a
//! change to either is a reviewed diff rather than a downstream reader's
//! discovery. `crates/compose-core/tests/plan_format_inventory.rs` is the other
//! half of that — it holds the *shape* to the document; this file holds what the
//! document *says* to what the command actually produces.
//!
//! The corpus is one composition and a project per edit of it, each edit chosen
//! for a sentence `docs/plan.md` makes:
//!
//! | project | what it pins |
//! |---|---|
//! | `reformatted` | §1's first property: imports, ordering, and comments are not changes |
//! | `renamed-model` | a rename is one removal, one addition, and the references that followed it |
//! | `rerouted-flow` | §5: a node added, an edge retargeted, a guard and a cycle budget moved, a channel edited |
//! | `resequenced-edges` | §5's edge identity and its `order`: an edge inserted, two swapped, one deleted, all between siblings sharing an address |
//! | `retimed-policy` | §5's policy half, at all three sites that carry one |
//! | `widened-surface` | §6: what a caller feels, and nothing else |
//! | `redeployed` | §4's deploy-layer components, which the `local` target admits |
//! | `reordered` | §3's order rule and §11's line under it, in both directions at once — including the two arrays of one model that fall on opposite sides of it, and an author's literal array, which is neither |
//! | `respelled` | §11's residual: the three leaves the artifact keeps as source text |
//! | `restated-defaults` | §11's other residual: a default written out is a key the artifact holds |
//! | `reworded-prompt` | §13: a cut may not hide the change it was run to show |
//! | `added-flow` | §3's rule that a component which arrived brings nothing with it |
//! | `broken-routing` | §7 in both directions, and §1's second property: an error introduced is a plan, exit `0` |
//! | `unresolvable-after` | §2.3: a spec with no artifact leaves nothing to compare, exit `1` |
//! | `unresolvable-both` | §2.3 again, when neither side resolves |
//!
//! Every run sets `NO_COLOR` and reads the streams through a pipe. The plan
//! report itself is unstyled either way — it is a list, not a diagnostic
//! (`crates/agent-compose/src/plan.rs`) — but the **refusal** goes through the
//! snippet renderer, which is where the variable would be.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

/// The corpus, which is also the directory every path in a report is relative
/// to.
fn projects() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/plan")
}

fn plan(arguments: &[&str]) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(projects())
        .env("NO_COLOR", "1")
        .arg("plan")
        .args(arguments)
        .output()
        .expect("the command runs")
}

/// The human report for one project pair, with its exit code.
fn report(project: &str) -> (String, i32) {
    let output = plan(&[&format!("{project}/before"), &format!("{project}/after")]);
    assert_eq!(
        stdout(&output),
        "",
        "{project}: human writes nothing to stdout"
    );
    (stderr(&output).to_string(), code(&output))
}

/// …and the machine one.
fn document(project: &str) -> (String, i32) {
    let output = plan(&[
        &format!("{project}/before"),
        &format!("{project}/after"),
        "--format",
        "json",
    ]);
    assert_eq!(
        stderr(&output),
        "",
        "{project}: JSON writes nothing to stderr"
    );
    (stdout(&output).to_string(), code(&output))
}

#[track_caller]
fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("stdout is UTF-8")
}

#[track_caller]
fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).expect("stderr is UTF-8")
}

#[track_caller]
fn code(output: &Output) -> i32 {
    output.status.code().expect("the command was not signalled")
}

/// The project `agent-compose build` emits for one entrypoint, as its files by
/// relative path.
///
/// Here so that "these two specs decide the same thing" can be *checked* rather
/// than asserted in a comment: a plan is a diff of compositions, and the claim
/// that a reported change is one the composition does not feel is only worth
/// what the emitted project says about it (`docs/plan.md` §11).
#[track_caller]
fn built(entrypoint: &str, purpose: &str) -> BTreeMap<String, Vec<u8>> {
    let out = std::env::temp_dir().join(format!(
        "agent-compose-plan-cli-{purpose}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&out);
    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(projects())
        .env("NO_COLOR", "1")
        .args(["build", entrypoint, "--out"])
        .arg(&out)
        .output()
        .expect("the command runs");
    assert_eq!(
        code(&output),
        0,
        "`{entrypoint}` builds: {}",
        stderr(&output)
    );

    let mut found = BTreeMap::new();
    let mut queue = vec![out.clone()];
    while let Some(directory) = queue.pop() {
        for entry in fs::read_dir(&directory)
            .expect("the emitted directory is readable")
            .flatten()
        {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
                continue;
            }
            let name = path
                .strip_prefix(&out)
                .expect("the walk started at the root")
                .to_string_lossy()
                .into_owned();
            found.insert(name, fs::read(&path).expect("an emitted file is readable"));
        }
    }
    let _ = fs::remove_dir_all(&out);
    assert!(!found.is_empty(), "`{entrypoint}` emitted a project");
    found
}

/// Two specs that resolve the same way are the same composition, however
/// differently they are written.
///
/// The `after` of this pair is the `before` split across three imported files,
/// with its sections in another order, its node keys swapped, one binding
/// written as a block rather than inline, and comments throughout. Every one of
/// those moves a span, and a plan reports nothing: `docs/plan.md` §1 and §10.
#[test]
fn a_composition_written_differently_is_not_a_composition_changed() {
    let (report, code) = report("reformatted");
    assert_eq!(
        report,
        "`reformatted/after/main.yml` and `reformatted/before/main.yml` describe the same \
         composition (target `local`)\n"
    );
    assert_eq!(code, 0);

    // …and the machine document is the same four empty collections rather than
    // the absence of one (§2.1).
    let (document, code) = document("reformatted");
    assert_eq!(
        document,
        r#"{
  "plan_version": 1,
  "before": {
    "entrypoint": "reformatted/before/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "after": {
    "entrypoint": "reformatted/after/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "components": [],
  "topology": [],
  "interfaces": [],
  "validation": {
    "introduced": [],
    "resolved": []
  }
}
"#
    );
    assert_eq!(code, 0);
}

/// A renamed definition is one removed and one added — and the two references
/// that had to follow it.
///
/// There is no rename detection and deliberately so: `model.fast` and
/// `model.default` are different addresses, everything that named the first now
/// names the second, and the four lines say exactly that. A plan that guessed
/// "renamed" would be guessing.
#[test]
fn a_renamed_definition_is_one_removed_one_added_and_its_references() {
    let (report, code) = report("renamed-model");
    assert_eq!(
        report,
        "\
components
  ~ agent.reviewer  renamed-model/after/main.yml:41:1
      model: \"model.fast\" -> \"model.default\"
  ~ agent.writer  renamed-model/after/main.yml:35:1
      model: \"model.fast\" -> \"model.default\"
  + model.default  renamed-model/after/main.yml:17:1
  - model.fast  renamed-model/before/main.yml:17:1

`renamed-model/after/main.yml` differs from `renamed-model/before/main.yml` (target `local`): \
4 component changes
"
    );
    assert_eq!(code, 0);
}

/// The rename in the machine format: one fixed record shape per change, the
/// removal carrying the **before** spec's location and everything else the
/// after's (§10).
#[test]
fn a_component_record_carries_its_change_its_kind_its_address_and_its_span() {
    let (document, code) = document("renamed-model");
    assert_eq!(
        document,
        r#"{
  "plan_version": 1,
  "before": {
    "entrypoint": "renamed-model/before/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "after": {
    "entrypoint": "renamed-model/after/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "components": [
    {
      "change": "changed",
      "component": "agent",
      "address": "agent.reviewer",
      "fields": [
        {
          "path": "model",
          "before": "model.fast",
          "after": "model.default"
        }
      ],
      "span": "main.yml:41:1..47:41"
    },
    {
      "change": "changed",
      "component": "agent",
      "address": "agent.writer",
      "fields": [
        {
          "path": "model",
          "before": "model.fast",
          "after": "model.default"
        }
      ],
      "span": "main.yml:35:1..39:28"
    },
    {
      "change": "added",
      "component": "model",
      "address": "model.default",
      "fields": [],
      "span": "main.yml:17:1..19:22"
    },
    {
      "change": "removed",
      "component": "model",
      "address": "model.fast",
      "fields": [],
      "span": "main.yml:17:1..19:22"
    }
  ],
  "topology": [],
  "interfaces": [],
  "validation": {
    "introduced": [],
    "resolved": []
  }
}
"#
    );
    assert_eq!(code, 0);
}

/// A topology edit: a node added, an edge **retargeted**, a guard and a cycle
/// budget moved, and a channel edited.
///
/// The retargeting is the line that a diff matching edges by `(from, to)` alone
/// could not produce: `review -> end` did not disappear and `review -> polish`
/// did not arrive, one edge changed where it goes (`docs/plan.md` §5).
#[test]
fn a_rerouted_flow_reports_the_edge_that_moved_rather_than_two_that_did_not() {
    let (report, code) = report("rerouted-flow");
    assert_eq!(
        report,
        "\
topology
  + flow.review_loop.polish  rerouted-flow/after/main.yml:58:13
  + flow.review_loop.polish->end  rerouted-flow/after/main.yml:64:7
  ~ flow.review_loop.review->polish  rerouted-flow/after/main.yml:63:7
      to: \"end\" -> \"polish\"
  ~ flow.review_loop.review->write  rerouted-flow/after/main.yml:62:7
      max_iterations: 3 -> 5
      when: \"review.output.verdict == 'revise'\" -> \"review.output.verdict == 'revise' && size(state…
  ~ state.draft  rerouted-flow/after/main.yml:8:3
      type.description: \"The current draft, rewritten on every pass.\" -> \"The current draft, rewritten on every pass and …

note: a value longer than one line is cut, with a `…` where the cut is; \
`--format json` carries every value whole
`rerouted-flow/after/main.yml` differs from `rerouted-flow/before/main.yml` (target `local`): \
5 topology changes
"
    );
    assert_eq!(code, 0);
}

/// A policy edit, reported at the site that changed — all three of them.
///
/// Grammar 9.3 resolves `retry`/`timeout`/`on_error` over four levels, and a
/// plan does not resolve them: what it reports is the level that moved, so
/// `defaults` and the two nodes are three lines rather than one derived answer
/// (`docs/plan.md` §5).
#[test]
fn a_policy_edit_is_reported_at_the_level_that_declared_it() {
    let (report, code) = report("retimed-policy");
    assert_eq!(
        report,
        "\
topology
  ~ defaults  retimed-policy/after/main.yml:4:3
      retry: (absent) -> {\"backoff\":\"1s\",\"max\":2}
      timeout: \"120s\" -> \"300s\"
  ~ flow.review_loop.review  retimed-policy/after/main.yml:61:7
      timeout: (absent) -> \"30s\"
  ~ flow.review_loop.write  retimed-policy/after/main.yml:59:12
      on_error: (absent) -> {\"strategy\":\"skip\"}

`retimed-policy/after/main.yml` differs from `retimed-policy/before/main.yml` (target `local`): \
3 topology changes
"
    );
    assert_eq!(code, 0);
}

/// What a caller feels, and nothing else.
///
/// A flow input gained a constraint and a second field arrived; the trigger
/// moved route, changed response mode, and grew a session key. None of it
/// touches the graph, so `topology` has nothing to say — and the schema change
/// is reported **at the field that changed** rather than as a rewritten field
/// map, which is what matching a named array by name buys (`docs/plan.md` §3).
#[test]
fn a_widened_surface_is_reported_where_a_caller_meets_it() {
    let (report, code) = report("widened-surface");
    assert_eq!(
        report,
        "\
interfaces
  ~ flow.review_loop  widened-surface/after/main.yml:49:1
      inputs.fields[goal].type.min_length: (absent) -> 1
      inputs.fields[tone]: (absent) -> {\"name\":\"tone\",\"type\":{\"default\":\"plain\",\"form\":…
  ~ trigger.on_request  widened-surface/after/main.yml:67:5
      path: \"/review\" -> \"/v1/review\"
      respond: \"sync\" -> \"async\"
      session_key: (absent) -> \"payload.body.session\"

note: a value longer than one line is cut, with a `…` where the cut is; \
`--format json` carries every value whole
`widened-surface/after/main.yml` differs from `widened-surface/before/main.yml` (target `local`): \
2 interface changes
"
    );
    assert_eq!(code, 0);
}

/// The deploy layer the built-in target admits is compared like any other
/// component.
///
/// `placements:` is reserved grammar and a no-op until M3 (PRD 5.10), which is
/// exactly why it is worth diffing: a plan is how a reviewer sees a change to a
/// section nothing executes yet. `storage_backends:` is the one section of that
/// layer this version cannot reach, and `docs/plan.md` §11 says why.
#[test]
fn a_changed_deploy_layer_is_reported_as_a_component() {
    let (report, code) = report("redeployed");
    assert_eq!(
        report,
        "\
components
  + placement.agent.reviewer  redeployed/after/deploy/local.yml:5:3
  ~ placement.agent.writer  redeployed/after/deploy/local.yml:7:3
      description: \"In-process, where there is one process.\" -> \"Its own process, even locally.\"
      runtime: \"colocated\" -> \"isolated\"

`redeployed/after/main.yml` differs from `redeployed/before/main.yml` (target `local`): \
2 component changes
"
    );
    assert_eq!(code, 0);
}

/// A declaration order is reported when the composition behaves differently for
/// it, and not otherwise.
///
/// The `after` of this pair swaps **ten** orders and edits nothing else. Four of
/// them decide something and are the ten lines below:
///
/// * a field map's order is a JSON Schema's `properties` and `required`, a
///   union's is its `oneOf`, and a model route's is the order its members are
///   tried in — three orders the grammar gives a meaning to (§3);
/// * the `seeds` channel's `default:` is not a declaration order at all. It is
///   an author's literal array, whose order **is** the value: reversed, it is a
///   different default handed to every caller, and it reports as four ordinary
///   field changes rather than as an `order` record. The key an array sits under
///   is what decides which rule it meets, and `default:` is not one of the
///   grammar's named arrays — a plan that classified it by the shape of its
///   elements would call this pair unchanged.
///
/// The other six are the assertion this test is really making, because what pins
/// them is the lines that are *not* here: an `optional:` set, an `expect_exit:`
/// set, a `route_on:` set, an `exec:`'s `env:`, a node's `input:` bindings and a
/// routed map's `routes:` are all selected by name or tested for membership, so
/// `docs/plan.md` §11 leaves them out and a regression would show up as extra
/// lines in this golden.
///
/// `model.resilient` is where both halves of the rule meet: `route:` and
/// `route_on:` are two arrays of one definition, and one of them is reported.
#[test]
fn a_declaration_order_is_reported_where_the_composition_reads_it() {
    let (report, code) = report("reordered");
    assert_eq!(
        report,
        "\
components
  ~ agent.reviewer  reordered/after/main.yml:69:1
      output.fields[reason].order: 1 -> 0
      output.fields[verdict].order: 0 -> 1
  ~ agent.triage  reordered/after/main.yml:79:1
      output.fields[findings].type.items.variants[auto_fixable].order: 0 -> 1
      output.fields[findings].type.items.variants[needs_human].order: 1 -> 0
  ~ model.resilient  reordered/after/main.yml:51:1
      route[0]: \"model.fast\" -> \"model.slow\"
      route[1]: \"model.slow\" -> \"model.fast\"

topology
  ~ state.seeds  reordered/after/main.yml:26:3
      type.default[0].name: \"alpha\" -> \"beta\"
      type.default[0].weight: 1 -> 2
      type.default[1].name: \"beta\" -> \"alpha\"
      type.default[1].weight: 2 -> 1

`reordered/after/main.yml` differs from `reordered/before/main.yml` (target `local`): \
3 component changes, 1 topology change
"
    );
    assert_eq!(code, 0);
}

/// An edge's **position** is compared, and it is compared over the edges both
/// specs declare.
///
/// The three edits of this pair are each an edge list rewritten, and each one
/// sits between edges running to the same node — so no edge here is identified
/// by its endpoints alone (`docs/plan.md` §5, §8):
///
/// * `triage` lost the first of four outgoing edges, and the `else:` one that
///   survived shares its address. The `-` names the edge that *left*, at the
///   line it was written on, and the survivor is not reported at all;
/// * `fix` had its two guarded edges swapped, which is the one edit here that
///   moves an edge within its own source node's outgoing order — two `order`
///   records, one per edge that moved. Grammar 7.3 rule 6 is multicast, so this
///   does not change which of the two fires; what it changes is the order the
///   node's routing decision is recorded in (`docs/plan.md` §5);
/// * `ask` gained an edge ahead of the two it had, to the node they both already
///   run to. One `+`, and nothing about the two it pushed down: positions are
///   counted over the edges both sides declare, which is §3's rule for a named
///   sequence and §5's for an edge.
///
/// What pins the pairing is as much what is *absent* here: fold a position into
/// the identity an edge is matched on and every one of these three lists reads
/// as a rewrite — the untouched siblings report guard edits, and the `-` lands
/// on the edge that stayed.
#[test]
fn an_edge_that_only_shifted_is_the_edge_that_was_already_there() {
    let (report, code) = report("resequenced-edges");
    assert_eq!(
        report,
        "\
topology
  + flow.sweep.ask->end  resequenced-edges/after/main.yml:76:7
  ~ flow.sweep.fix->end  resequenced-edges/after/main.yml:73:7
      order: 1 -> 0
  ~ flow.sweep.fix->report  resequenced-edges/after/main.yml:74:7
      order: 0 -> 1
  - flow.sweep.triage->report  resequenced-edges/before/main.yml:73:7

`resequenced-edges/after/main.yml` differs from `resequenced-edges/before/main.yml` \
(target `local`): 4 topology changes
"
    );
    assert_eq!(code, 0);

    let (document, code) = document("resequenced-edges");
    assert_eq!(
        document,
        r#"{
  "plan_version": 1,
  "before": {
    "entrypoint": "resequenced-edges/before/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "after": {
    "entrypoint": "resequenced-edges/after/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "components": [],
  "topology": [
    {
      "change": "added",
      "site": "edge",
      "address": "flow.sweep.ask->end",
      "flow": "flow.sweep",
      "fields": [],
      "span": "main.yml:76:7..76:62"
    },
    {
      "change": "changed",
      "site": "edge",
      "address": "flow.sweep.fix->end",
      "flow": "flow.sweep",
      "fields": [
        {
          "path": "order",
          "before": 1,
          "after": 0
        }
      ],
      "span": "main.yml:73:7..73:53"
    },
    {
      "change": "changed",
      "site": "edge",
      "address": "flow.sweep.fix->report",
      "flow": "flow.sweep",
      "fields": [
        {
          "path": "order",
          "before": 0,
          "after": 1
        }
      ],
      "span": "main.yml:74:7..74:68"
    },
    {
      "change": "removed",
      "site": "edge",
      "address": "flow.sweep.triage->report",
      "flow": "flow.sweep",
      "fields": [],
      "span": "main.yml:73:7..73:73"
    }
  ],
  "interfaces": [],
  "validation": {
    "introduced": [],
    "resolved": []
  }
}
"#
    );
    assert_eq!(code, 0);
}

/// A leaf the artifact keeps as source text is compared as that text, and this
/// is the whole of what that costs a reader.
///
/// Three leaves reach the IR as the author's own spelling: a duration, a CEL
/// expression, and whether a number was written as an integer or as a float. The
/// `after` of this pair respells all three and decides nothing differently —
/// `agent-compose build` emits a byte-identical project for two of them, and for
/// the third the guard's text *is* the string in `src/graph.ts`. All three
/// report, which is what `docs/plan.md` §11 states for a reader branching on the
/// command: real in the artifact, and nothing the composition does differently.
///
/// This golden is the deliberate half of that. The alternative — deciding a
/// duration from a string — needs the key above it to know when to try, and a
/// key-gated rule over `settings:` and `default:` would hide an edit to author
/// data rather than suppress a non-edit.
#[test]
fn a_leaf_the_artifact_keeps_as_source_text_is_compared_as_that_text() {
    let (report, code) = report("respelled");
    assert_eq!(
        report,
        "\
components
  ~ agent.writer  respelled/after/main.yml:20:1
      input.fields[passes].type.minimum: 1 -> 1.0

topology
  ~ defaults  respelled/after/main.yml:10:3
      timeout: \"120s\" -> \"2m\"
  ~ flow.review_loop.review->write  respelled/after/main.yml:48:7
      when: \"review.output.verdict == 'revise'\" -> \"review.output.verdict=='revise'\"

`respelled/after/main.yml` differs from `respelled/before/main.yml` (target `local`): \
1 component change, 2 topology changes
"
    );
    assert_eq!(code, 0);
}

/// A default written out is a key the artifact holds, and reports as one.
///
/// The `after` of this pair writes four of the grammar's own defaults out and
/// edits nothing else: `expect_exit: [0]` (grammar 6.1),
/// `max_tool_iterations: 8` (Decision D51), `unique_items: false` (grammar 3.5),
/// and an `http` trigger's `method: POST` (grammar 13.3, Decision D45). The IR
/// materializes no default (`crates/compose-core/src/ir`), so each of the four
/// is a key the after spec's artifact holds and the before spec's does not — one
/// record in each of the three structural sections, and two in the first.
///
/// The second half of the test is why the pair is here rather than in a comment:
/// the two sides emit the **same project, byte for byte**, so every one of those
/// four lines is a change to the artifact and to nothing the composition does.
/// `docs/plan.md` §11 states it for a reader branching on the command, beside
/// the three respelled leaves, and normalizing it away would need the table of
/// keys and their defaults §3's closing paragraph refuses.
#[test]
fn a_default_written_out_is_a_key_the_artifact_holds() {
    let (report, code) = report("restated-defaults");
    assert_eq!(
        report,
        "\
components
  ~ agent.writer  restated-defaults/after/main.yml:45:1
      max_tool_iterations: (absent) -> 8
  ~ tool.spell_check  restated-defaults/after/main.yml:34:1
      exec.expect_exit: (absent) -> [0]

topology
  ~ state.notes  restated-defaults/after/main.yml:26:3
      type.unique_items: (absent) -> false

interfaces
  ~ trigger.on_request  restated-defaults/after/main.yml:69:5
      method: (absent) -> \"POST\"

`restated-defaults/after/main.yml` differs from `restated-defaults/before/main.yml` \
(target `local`): 2 component changes, 1 topology change, 1 interface change
"
    );
    assert_eq!(code, 0);

    // …and the machine document writes no `before` key at all on any of the
    // four, which is §3's spelling for a field that is not declared — not the
    // same thing as one declared `null`.
    let (document, code) = document("restated-defaults");
    assert_eq!(
        document,
        r#"{
  "plan_version": 1,
  "before": {
    "entrypoint": "restated-defaults/before/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "after": {
    "entrypoint": "restated-defaults/after/main.yml",
    "target": "local",
    "spec_version": "0.1"
  },
  "components": [
    {
      "change": "changed",
      "component": "agent",
      "address": "agent.writer",
      "fields": [
        {
          "path": "max_tool_iterations",
          "after": 8
        }
      ],
      "span": "main.yml:45:1..50:28"
    },
    {
      "change": "changed",
      "component": "tool",
      "address": "tool.spell_check",
      "fields": [
        {
          "path": "exec.expect_exit",
          "after": [
            0
          ]
        }
      ],
      "span": "main.yml:34:1..43:21"
    }
  ],
  "topology": [
    {
      "change": "changed",
      "site": "channel",
      "address": "state.notes",
      "fields": [
        {
          "path": "type.unique_items",
          "after": false
        }
      ],
      "span": "main.yml:26:3..32:16"
    }
  ],
  "interfaces": [
    {
      "change": "changed",
      "surface": "trigger",
      "address": "trigger.on_request",
      "fields": [
        {
          "path": "method",
          "after": "POST"
        }
      ],
      "span": "main.yml:69:5..75:32"
    }
  ],
  "validation": {
    "introduced": [],
    "resolved": []
  }
}
"#
    );
    assert_eq!(code, 0);

    // …and the four changes are changes to the artifact and to nothing the
    // composition does: one project, emitted twice.
    let before = built("restated-defaults/before/main.yml", "restated-before");
    let after = built("restated-defaults/after/main.yml", "restated-after");
    assert_eq!(
        before.keys().collect::<Vec<_>>(),
        after.keys().collect::<Vec<_>>(),
        "the two sides emit the same files"
    );
    for (name, held) in &before {
        assert_eq!(
            String::from_utf8_lossy(held),
            String::from_utf8_lossy(&after[name]),
            "`{name}` is emitted identically for the two sides"
        );
    }
}

/// A cut may not hide the change the command was run to show.
///
/// The two prompts of this pair agree for sixty-odd characters and differ after
/// them, which is past the end of a report line: cut from the head, both sides
/// would print as the same text. The window moves onto the difference instead,
/// the same window on both sides, and the note above the verdict says where the
/// whole value is (`docs/plan.md` §13).
#[test]
fn a_value_cut_past_the_difference_is_printed_from_the_difference() {
    let (report, code) = report("reworded-prompt");
    assert_eq!(
        report,
        "\
components
  ~ agent.reviewer  reworded-prompt/after/main.yml:13:1
      prompt: …hen say alpha.\" -> …hen say beta.\"

note: a value longer than one line is cut, with a `…` where the cut is; \
`--format json` carries every value whole
`reworded-prompt/after/main.yml` differs from `reworded-prompt/before/main.yml` (target `local`): \
1 component change
"
    );
    assert_eq!(code, 0);

    // …and the machine document carries the two prompts whole, which is what the
    // note points at.
    let (document, code) = document("reworded-prompt");
    assert!(
        document
            .contains("\"Review the draft with great care and much attention, then say alpha.\""),
        "{document}"
    );
    assert!(
        document
            .contains("\"Review the draft with great care and much attention, then say beta.\""),
        "{document}"
    );
    assert_eq!(code, 0);
}

/// A flow that arrived is one line, not one line per node it declares.
///
/// The added flow carries two nodes and three edges, and none of them is in the
/// topology section: its whole graph is new, the line that says the flow is new
/// already says so, and a plan that expanded it would bury the two edges that
/// moved in the flow beside it under five that did not move at all
/// (`docs/plan.md` §3).
#[test]
fn a_component_that_arrived_brings_nothing_with_it() {
    let (report, code) = report("added-flow");
    assert_eq!(
        report,
        "\
components
  + flow.summarize  added-flow/after/main.yml:64:1

`added-flow/after/main.yml` differs from `added-flow/before/main.yml` (target `local`): \
1 component change
"
    );
    assert_eq!(code, 0);
}

/// Diagnostics are **content**. The after spec of this pair is invalid, and the
/// command exits `0` having said so.
///
/// Both directions are here: two errors the after spec introduces, and one the
/// before spec had that the after spec no longer does. A CI step branching on
/// `plan` is asking "could this be planned", and `validate` is the verb that
/// asks whether a composition is valid (`crates/agent-compose/src/main.rs`).
///
/// The report still ends with the pointer at `explain` every human report
/// carrying a code ends with: a reader who met `dead-end` while reviewing a
/// change never typed `validate` at all.
#[test]
fn diagnostics_introduced_and_resolved_are_content_rather_than_a_refusal() {
    let (report, code) = report("broken-routing");
    assert_eq!(
        report,
        "\
topology
  - flow.review_loop.orphan  broken-routing/before/main.yml:58:13
  - flow.review_loop.orphan->end  broken-routing/before/main.yml:64:7
  - flow.review_loop.review->end  broken-routing/before/main.yml:63:7

validation
  + error[non-exhaustive]: node `review` of `flow.review_loop` routes on `verdict` and leaves \
`approve` unrouted  broken-routing/after/main.yml:57:5
  + error[dead-end]: the `max_iterations` edge from `review` to `write` in `flow.review_loop` \
has no escape  broken-routing/after/main.yml:61:7
  - error[unreachable-node]: node `orphan` of `flow.review_loop` is not reachable from `start`  \
broken-routing/before/main.yml:58:5

`broken-routing/after/main.yml` differs from `broken-routing/before/main.yml` (target `local`): \
3 topology changes, 3 validation changes
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(code, 0);
}

/// A spec with no artifact leaves nothing to compare.
///
/// The report is the composition's own diagnostics, rendered exactly as
/// `validate` renders them — the same snippet, the same help — followed by a
/// line naming which side of the comparison failed, and then the run's one
/// pointer at `explain`. Exit `1`.
#[test]
fn a_spec_that_does_not_resolve_is_reported_the_way_validate_reports_it() {
    let (report, code) = report("unresolvable-after");
    assert_eq!(
        report,
        "\
error[undefined-reference]: `provider.bedrock` is not defined in this composition
  --> unresolvable-after/after/main.yml:18:13
   |
18 |   provider: provider.bedrock
   |             ^^^^^^^^^^^^^^^^
   |
   = help: the composition defines `provider.anthropic`; a definition is part of it only when \
the entrypoint imports the file that declares it (grammar 1.4)

error: the after spec `unresolvable-after/after/main.yml` does not resolve (target `local`): 1 error
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(code, 1);

    // The machine document opens with the same `plan_version` a plan does, so
    // one dispatch reads either (`docs/plan.md` §2).
    let (document, code) = document("unresolvable-after");
    assert_eq!(
        document,
        r#"{
  "plan_version": 1,
  "failed": [
    {
      "spec": "after",
      "entrypoint": "unresolvable-after/after/main.yml",
      "diagnostics": [
        {
          "code": "undefined-reference",
          "severity": "error",
          "message": "`provider.bedrock` is not defined in this composition",
          "span": "main.yml:18:13..18:29",
          "labels": [],
          "help": "the composition defines `provider.anthropic`; a definition is part of it only when the entrypoint imports the file that declares it (grammar 1.4)"
        }
      ]
    }
  ]
}
"#
    );
    assert_eq!(code, 1);
}

/// Both sides are reported when both of them failed.
///
/// A person diffing two branches wants to know that neither of them resolves,
/// not to find out one at a time — so the two blocks are written in `before`,
/// `after` order with a blank line between them, and the machine document
/// carries two entries (`docs/plan.md` §2.3).
#[test]
fn two_specs_that_do_not_resolve_are_both_reported() {
    let (report, code) = report("unresolvable-both");
    assert_eq!(
        report,
        "\
error[undefined-reference]: `provider.bedrock` is not defined in this composition
  --> unresolvable-both/before/main.yml:10:13
   |
10 |   provider: provider.bedrock
   |             ^^^^^^^^^^^^^^^^
   |
   = help: the composition defines `provider.anthropic`; a definition is part of it only when \
the entrypoint imports the file that declares it (grammar 1.4)

error: the before spec `unresolvable-both/before/main.yml` does not resolve (target `local`): \
1 error

error[io-error]: cannot read `models.yml`: No such file or directory (os error 2)
 --> unresolvable-both/after/main.yml:6:5
  |
6 |   - models.yml
  |     ^^^^^^^^^^
  |
  = help: imports are relative paths resolved against the entrypoint's directory, and there is \
no directory scanning: the file has to be there (grammar 1.4)

error: the after spec `unresolvable-both/after/main.yml` does not resolve (target `local`): 1 error
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(code, 1);

    let (document, code) = document("unresolvable-both");
    let failed: Vec<&str> = document
        .lines()
        .filter_map(|line| line.trim().strip_prefix("\"spec\": \""))
        .map(|rest| rest.trim_end_matches("\","))
        .collect();
    assert_eq!(failed, ["before", "after"], "{document}");
    assert_eq!(code, 1);
}

/// Each side may be named by its entrypoint or by the directory that holds one.
///
/// `validate` takes a file and only a file; what a `plan` is given is usually
/// two trees. The two spellings produce the same plan, and the report names each
/// side by the entrypoint it resolved — never by the directory it was handed —
/// so the two lines a reader compares are the two files.
#[test]
fn a_project_directory_names_its_own_entrypoint() {
    let directories = plan(&["renamed-model/before", "renamed-model/after"]);
    let files = plan(&[
        "renamed-model/before/main.yml",
        "renamed-model/after/main.yml",
    ]);
    assert_eq!(stderr(&directories), stderr(&files));
    assert_eq!(code(&directories), code(&files));
    assert!(
        stderr(&files).ends_with(
            "`renamed-model/after/main.yml` differs from `renamed-model/before/main.yml` \
             (target `local`): 4 component changes\n"
        ),
        "{}",
        stderr(&files)
    );
}

/// The same pair produces the same bytes, in both formats, on every run
/// (PRD 5.12, `docs/plan.md` §9).
///
/// A plan is built out of maps, sorted collections and a fixed section order,
/// and none of that is visible in a single run: a report ordered by hash
/// iteration would pass every assertion above on the run that produced its
/// golden and differ on the next machine. Running each format twice is what
/// makes the ordering rules a property rather than an accident.
#[test]
fn one_pair_produces_one_plan_however_often_it_is_asked() {
    for project in [
        "rerouted-flow",
        "broken-routing",
        "widened-surface",
        "reordered",
        "resequenced-edges",
    ] {
        let (first, _) = report(project);
        let (second, _) = report(project);
        assert_eq!(first, second, "{project}: the human report is stable");

        let (first, _) = document(project);
        let (second, _) = document(project);
        assert_eq!(first, second, "{project}: the machine document is stable");
    }
}

/// `2` is the code for "there was nothing to look at", as against `1` for "one
/// of them does not resolve". Both messages name **which** side failed, because
/// the command takes two and a message that did not would send a reader to the
/// wrong tree.
#[test]
fn an_unreadable_spec_exits_two_and_says_which_side_it_is() {
    let output = plan(&["no/such", "reformatted/after"]);
    assert_eq!(
        stderr(&output),
        "error: cannot read the before spec `no/such`: No such file or directory (os error 2)\n"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 2);

    let output = plan(&["reformatted/before", "no/such"]);
    assert_eq!(
        stderr(&output),
        "error: cannot read the after spec `no/such`: No such file or directory (os error 2)\n"
    );
    assert_eq!(code(&output), 2);

    // A directory is taken as naming its own `main.yml`, and one that holds no
    // such file is the command's own precondition failing rather than a guess
    // at some other file.
    let output = plan(&["reformatted", "reformatted/after"]);
    assert_eq!(
        stderr(&output),
        "error: `reformatted` holds no `main.yml`: the before spec is a spec entrypoint, or a \
         project directory that holds one\n"
    );
    assert_eq!(code(&output), 2);
}
