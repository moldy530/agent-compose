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
//! The corpus is one composition and eight edits of it, each chosen for a
//! sentence `docs/plan.md` makes:
//!
//! | project | what it pins |
//! |---|---|
//! | `reformatted` | §1's first property: imports, ordering, and comments are not changes |
//! | `renamed-model` | a rename is one removal, one addition, and the references that followed it |
//! | `rerouted-flow` | §5: a node added, an edge retargeted, a guard and a cycle budget moved, a channel edited |
//! | `retimed-policy` | §5's policy half, at all three sites that carry one |
//! | `widened-surface` | §6: what a caller feels, and nothing else |
//! | `redeployed` | §4's deploy-layer components, which the `local` target admits |
//! | `added-flow` | §3's rule that a component which arrived brings nothing with it |
//! | `broken-routing` | §7 in both directions, and §1's second property: an error introduced is a plan, exit `0` |
//! | `unresolvable-after` | §2.3: a spec with no artifact leaves nothing to compare, exit `1` |
//! | `unresolvable-both` | §2.3 again, when neither side resolves |
//!
//! Every run sets `NO_COLOR` and reads the streams through a pipe. The plan
//! report itself is unstyled either way — it is a list, not a diagnostic
//! (`crates/agent-compose/src/plan.rs`) — but the **refusal** goes through the
//! snippet renderer, which is where the variable would be.

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
      runtime: \"colocated\" -> \"isolated\"

`redeployed/after/main.yml` differs from `redeployed/before/main.yml` (target `local`): \
2 component changes
"
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
"
    );
    assert_eq!(code, 0);
}

/// A spec with no artifact leaves nothing to compare.
///
/// The report is the composition's own diagnostics, rendered exactly as
/// `validate` renders them — the same snippet, the same help — followed by a
/// line naming which side of the comparison failed. Exit `1`.
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
    for project in ["rerouted-flow", "broken-routing", "widened-surface"] {
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
