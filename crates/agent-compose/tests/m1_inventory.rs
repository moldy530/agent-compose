//! The M1 completion inventory: every criterion PRD §7 M1 promises, mapped to
//! the acceptance test that decides it and to whether that test runs yet.
//!
//! `crates/compose-core/tests/m0_inventory.rs` is the companion for M0, and this
//! file is the same idea one milestone on: the PRD's own sentences, transcribed
//! verbatim and made executable. The difference is what a row points at. M0's
//! rows point at diagnostic codes, because a static check is a thing the
//! compiler *says*; M1's point at acceptance tests, because codegen is a thing a
//! compiled graph *does* (CLAUDE.md: "each milestone defines its acceptance
//! criteria as runnable tests before implementation of that milestone begins").
//!
//! Four assertions keep it honest, and each covers a different way a plan rots:
//!
//! * the criteria are the PRD's own phrases, split from its own sentences, so a
//!   milestone that grows a promise fails here rather than shipping untested;
//! * every test a row names exists;
//! * every test in the suite is named by a row — the direction that catches a
//!   test drifting out of the plan rather than a plan drifting out of the suite;
//! * a row's status matches the test's `#[ignore]` attribute **exactly**, so
//!   un-ignoring a test is the only way to claim a criterion is met, and
//!   claiming it without the feature is a failure here.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One acceptance criterion.
struct Criterion {
    /// Which of PRD §7 M1's bullets it comes from.
    bullet: Bullet,
    /// The PRD's own phrase, verbatim.
    phrase: &'static str,
    /// The tests in `tests/m1_acceptance.rs` that decide it, each with whether
    /// it runs yet.
    ///
    /// Status is per **test**, not per criterion: one criterion's tests can be
    /// unlocked by different features — "state models (incl. tagged unions via
    /// Zod)" needs the state model for one of its tests and Zod discriminated
    /// unions for the other — and a status that could not say so would either
    /// lie about one of them or force the reasons to be vague.
    tests: &'static [(&'static str, Status)],
}

/// Which sentence of PRD §7 M1 a criterion is transcribed from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bullet {
    /// "IR → deterministic LangGraph TypeScript: …"
    Codegen,
    /// "`agent-compose build`, `agent-compose run` (manual trigger), …"
    Commands,
    /// "Mock provider server + e2e harness: …"
    Harness,
    /// Not PRD §7 M1's own enumeration: CLAUDE.md's *Validation strategy*.
    Strategy,
}

/// Whether a criterion's tests run today.
enum Status {
    /// They run. This PR delivers what they test.
    Live,
    /// They are `#[ignore]`d, with this reason — verbatim, so the attribute and
    /// the inventory cannot drift apart.
    Pending(&'static str),
}

/// PRD §7 M1's first bullet, decomposed into the ten items its own sentence
/// lists, in order.
const CODEGEN: &[Criterion] = &[
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "state models (incl. tagged unions via Zod)",
        tests: &[
            (
                "state_channels_carry_their_declared_types_and_defaults",
                Status::Pending(
                    "M1: `agent-compose build` must emit the state model, and `run` must execute it",
                ),
            ),
            (
                "a_tagged_union_output_is_narrowed_per_variant_and_a_bad_tag_is_rejected",
                Status::Pending(
                    "M1: codegen must emit Zod discriminated unions for tagged-union outputs",
                ),
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "node fns",
        tests: &[
            (
                "an_agent_node_sends_its_prompt_input_and_output_schema",
                Status::Pending("M1: codegen must emit agent node fns"),
            ),
            (
                "an_agent_node_bounds_its_tool_loop_at_max_tool_iterations",
                Status::Pending("M1: codegen must emit the agent tool loop"),
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "routers with embedded CEL",
        tests: &[(
            "an_edge_guard_routes_on_the_source_nodes_structured_output",
            Status::Pending("M1: codegen must emit routers with embedded CEL"),
        )],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "bounded cycles",
        tests: &[(
            "a_bounded_cycle_leaves_through_its_escape_edge_when_the_budget_is_spent",
            Status::Pending("M1: codegen must emit the per-cycle iteration counter"),
        )],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "homogeneous + discriminator-routed `map`→`Send` with index-tagged reducers",
        tests: &[
            (
                "a_homogeneous_map_dispatches_one_instance_per_item",
                Status::Pending("M1: codegen must emit `map` as LangGraph `Send`"),
            ),
            (
                "a_routed_map_sends_each_variant_to_its_own_route",
                Status::Pending("M1: codegen must emit discriminator-routed `map` dispatch"),
            ),
            (
                "appended_results_are_ordered_by_source_item_index",
                Status::Pending("M1: codegen must emit index-tagged reducers"),
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "subgraphs",
        tests: &[(
            "a_subgraph_runs_with_explicit_bindings_and_isolated_history",
            Status::Pending("M1: codegen must emit subgraphs"),
        )],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "retry/timeout policy",
        tests: &[
            (
                "a_node_retries_its_model_call_per_its_declared_policy",
                Status::Pending("M1: codegen must emit retry policy"),
            ),
            (
                "a_node_timeout_fires_and_its_error_policy_takes_over",
                Status::Pending("M1: codegen must emit timeout policy"),
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "store-op nodes + synthesized store tools with SQLite/local-disk backends",
        tests: &[
            (
                "a_store_op_node_reads_and_writes_the_local_backend",
                Status::Pending(
                    "M1: codegen must emit store-op nodes over the SQLite/local-disk backends",
                ),
            ),
            (
                "an_attached_store_synthesizes_its_tool_surface_in_the_model_request",
                Status::Pending("M1: codegen must synthesize store tools"),
            ),
            (
                "agent_access_read_withholds_the_write_tool",
                Status::Pending("M1: codegen must synthesize store tools"),
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "model routing with trace-recorded failover",
        tests: &[
            (
                "a_route_fails_over_to_its_next_member_and_the_trace_records_it",
                Status::Pending("M1: codegen must emit model routing with trace-recorded failover"),
            ),
            (
                "a_condition_outside_route_on_fails_the_node_instead_of_failing_over",
                Status::Pending("M1: codegen must emit model routing with trace-recorded failover"),
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "env-ref presence checks at process start",
        tests: &[(
            "a_missing_env_ref_fails_at_process_start_naming_the_variable",
            Status::Pending("M1: generated code must check env-ref presence at process start"),
        )],
    },
];

/// PRD §7 M1's second bullet, decomposed into the four items it lists.
const COMMANDS: &[Criterion] = &[
    Criterion {
        bullet: Bullet::Commands,
        phrase: "`agent-compose build`",
        tests: &[(
            "build_writes_a_typescript_project_for_the_target",
            Status::Pending("M1: `agent-compose build` must exist"),
        )],
    },
    Criterion {
        bullet: Bullet::Commands,
        phrase: "`agent-compose run` (manual trigger)",
        tests: &[(
            "run_executes_a_manual_trigger_and_prints_the_flow_outputs",
            Status::Pending("M1: `agent-compose run` must exist"),
        )],
    },
    Criterion {
        bullet: Bullet::Commands,
        phrase: "`agent-compose serve` (generated Fastify app for http triggers: start/resume/status)",
        tests: &[(
            "serve_exposes_start_resume_and_status_for_an_http_trigger",
            Status::Pending(
                "M1: `agent-compose serve` must exist, and the `human` node runtime with it",
            ),
        )],
    },
    Criterion {
        bullet: Bullet::Commands,
        phrase: "golden-file codegen tests",
        tests: &[(
            "build_is_byte_identical_for_byte_identical_input",
            Status::Pending("M1: `agent-compose build` must exist"),
        )],
    },
];

/// PRD §7 M1's third bullet — what this PR delivers, so its rows are the live
/// ones. It is one criterion rather than a split list because its sentence is
/// one claim ("compiled graphs execute end-to-end in CI …") with a trailing
/// condition, not an enumeration of deliverables.
const HARNESS: &[Criterion] = &[Criterion {
    bullet: Bullet::Harness,
    phrase: "Mock provider server + e2e harness: compiled graphs execute end-to-end in CI with scripted model responses, no API keys.",
    tests: &[
        ("the_acceptance_fixtures_validate_clean", Status::Live),
        (
            "the_harness_serves_both_provider_surfaces_without_api_keys",
            Status::Live,
        ),
        (
            "a_scripted_structured_output_is_what_an_agent_node_will_receive",
            Status::Live,
        ),
        (
            "a_scripted_failover_condition_is_what_a_model_route_will_see",
            Status::Live,
        ),
        (
            "every_model_call_is_recorded_in_order_with_its_parsed_body",
            Status::Live,
        ),
        (
            "a_request_no_codegen_should_send_is_refused_and_recorded",
            Status::Live,
        ),
        (
            "an_unscripted_model_call_fails_the_run_loudly",
            Status::Live,
        ),
        (
            "a_scripted_delay_makes_completion_order_differ_from_item_order",
            Status::Live,
        ),
    ],
}];

/// The M1 requirements CLAUDE.md's *Validation strategy* adds that PRD §7 M1's
/// own sentences do not name individually. They are not extras — "CI green has
/// to mean actually done" is the reason this file exists — but they come from
/// the working agreement rather than from the PRD's enumeration, so they are
/// listed apart to keep the transcription honest. This mirrors `GRAMMAR` in
/// `m0_inventory.rs`.
const STRATEGY: &[Criterion] = &[
    Criterion {
        bullet: Bullet::Strategy,
        phrase: "every golden fixture must type-check (`tsc`) and construct its graph under the pinned LangGraph version",
        tests: &[(
            "every_generated_project_type_checks_and_constructs_its_graph",
            Status::Pending(
                "M1: `agent-compose build` must exist, and the pinned LangGraph toolchain with it",
            ),
        )],
    },
    Criterion {
        bullet: Bullet::Strategy,
        phrase: "shared fixtures (expression + input + expected result) executed against both the Rust validator's interpreter and the JS evaluator embedded in generated code",
        tests: &[(
            "the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus",
            Status::Pending("M1: generated routers must embed a JS CEL evaluator"),
        )],
    },
];

fn rows() -> impl Iterator<Item = &'static Criterion> {
    CODEGEN
        .iter()
        .chain(COMMANDS)
        .chain(HARNESS)
        .chain(STRATEGY)
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// The text of PRD §7 M1 — the three bullets between its heading and M2's.
fn milestone() -> Vec<String> {
    let prd = fs::read_to_string(repository().join("prd.md")).expect("the PRD is readable");
    prd.lines()
        .skip_while(|line| !line.starts_with("**M1 —"))
        .take_while(|line| !line.starts_with("**M2 —"))
        .map(str::to_string)
        .collect()
}

/// One bullet of PRD §7 M1, by the phrase that opens it.
fn bullet(opening: &str) -> String {
    milestone()
        .into_iter()
        .find(|line| line.starts_with(&format!("- {opening}")))
        .unwrap_or_else(|| panic!("PRD §7 M1 has a bullet opening `{opening}`"))
}

/// A bullet's list of items: everything after the first `: ` (where the bullet
/// has a lead-in), split on `, `.
fn items(bullet: &str, lead_in: Option<&str>) -> Vec<String> {
    let list = match lead_in {
        Some(lead_in) => {
            bullet
                .split_once(lead_in)
                .unwrap_or_else(|| panic!("the bullet opens with `{lead_in}`"))
                .1
        }
        None => bullet.trim_start_matches("- "),
    };
    list.trim()
        .trim_end_matches('.')
        .split(", ")
        .map(|item| item.trim().to_string())
        .collect()
}

/// One test in `tests/m1_acceptance.rs`: its name, and the `#[ignore]` reason it
/// carries if it carries one.
struct Test {
    name: String,
    ignore: Option<String>,
}

/// Every `#[test]` in the acceptance suite, read from its source.
///
/// Reading the source rather than asking the harness is what lets this file
/// assert about `#[ignore]` **reasons**, which no test-runner API exposes — and
/// the reason is exactly what tells a later PR which feature un-ignores which
/// test.
fn suite() -> Vec<Test> {
    let source =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/m1_acceptance.rs"))
            .expect("the acceptance suite is readable");

    let mut tests = Vec::new();
    let mut is_test = false;
    let mut ignore = None;
    for line in source.lines() {
        let line = line.trim();
        if line == "#[test]" {
            is_test = true;
        } else if let Some(reason) = line
            .strip_prefix("#[ignore = \"")
            .and_then(|rest| rest.strip_suffix("\"]"))
        {
            ignore = Some(reason.to_string());
        } else if let Some(rest) = line.strip_prefix("fn ") {
            if is_test {
                let name = rest.split('(').next().expect("a function name").to_string();
                tests.push(Test {
                    name,
                    ignore: ignore.clone(),
                });
            }
            is_test = false;
            ignore = None;
        }
    }
    tests
}

/// The criteria are PRD §7 M1's own phrases, split from its own sentences.
///
/// The two enumerated bullets are compared item by item, so a criterion this
/// file paraphrases — or one the PRD adds — is a failure rather than a silent
/// gap. The third bullet is one claim rather than a list, so it is checked by
/// containment.
#[test]
fn the_inventory_transcribes_the_prd_m1_bullets() {
    let codegen = items(
        &bullet("IR → deterministic LangGraph TypeScript"),
        Some("LangGraph TypeScript: "),
    );
    assert_eq!(
        codegen,
        CODEGEN
            .iter()
            .map(|criterion| criterion.phrase.to_string())
            .collect::<Vec<_>>(),
        "the codegen bullet and the inventory's transcription of it have diverged"
    );

    let commands = items(&bullet("`agent-compose build`"), None);
    assert_eq!(
        commands,
        COMMANDS
            .iter()
            .map(|criterion| criterion.phrase.to_string())
            .collect::<Vec<_>>(),
        "the commands bullet and the inventory's transcription of it have diverged"
    );

    let harness = bullet("Mock provider server");
    assert_eq!(
        harness.trim_start_matches("- "),
        HARNESS[0].phrase,
        "the harness bullet and the inventory's transcription of it have diverged"
    );
}

/// The rows CLAUDE.md contributes are its own phrases too.
#[test]
fn the_strategy_rows_transcribe_claude_md() {
    let agreement =
        fs::read_to_string(repository().join("CLAUDE.md")).expect("CLAUDE.md is readable");
    for criterion in STRATEGY {
        assert!(
            agreement.contains(criterion.phrase),
            "`{}` is not a phrase of CLAUDE.md's validation strategy",
            criterion.phrase
        );
    }
}

/// Every test a row names exists in the acceptance suite.
#[test]
fn every_named_test_exists() {
    let tests = suite();
    let suite: BTreeSet<&str> = tests.iter().map(|test| test.name.as_str()).collect();
    for criterion in rows() {
        assert!(
            !criterion.tests.is_empty(),
            "`{}` names no test, so nothing decides it",
            criterion.phrase
        );
        for (test, _) in criterion.tests {
            assert!(
                suite.contains(test),
                "`{}` names `{test}`, which is not a test in `tests/m1_acceptance.rs`",
                criterion.phrase
            );
        }
    }
}

/// Every test in the acceptance suite is named by a row.
///
/// The direction that keeps the plan honest: a test written outside the
/// inventory is a criterion nobody wrote down, and a criterion nobody wrote down
/// is one the next milestone review cannot see.
#[test]
fn the_inventory_accounts_for_every_acceptance_test() {
    let inventoried: BTreeSet<&str> = rows()
        .flat_map(|criterion| criterion.tests.iter().map(|(test, _)| *test))
        .collect();
    let orphans: Vec<String> = suite()
        .into_iter()
        .filter(|test| !inventoried.contains(test.name.as_str()))
        .map(|test| test.name)
        .collect();
    assert!(
        orphans.is_empty(),
        "these acceptance tests belong to no M1 criterion: {orphans:?}"
    );
}

/// A row's status is the test's `#[ignore]` attribute, verbatim.
///
/// This is what makes un-ignoring the definition of done: a `Live` row whose
/// test is still ignored is a criterion claimed and not met, and a `Pending` row
/// whose test has been un-ignored is one met and not claimed. Comparing the
/// reason **verbatim** also means the inventory says which feature unlocks which
/// test, in the same words the source does.
#[test]
fn a_pending_criterion_names_what_must_land_first() {
    let suite = suite();
    for criterion in rows() {
        for (name, status) in criterion.tests {
            let test = suite
                .iter()
                .find(|test| test.name == *name)
                .unwrap_or_else(|| panic!("`{name}` is a test"));
            match status {
                Status::Live => assert!(
                    test.ignore.is_none(),
                    "`{name}` is inventoried as live but carries `#[ignore = \"{}\"]`",
                    test.ignore.clone().unwrap_or_default()
                ),
                Status::Pending(reason) => {
                    assert_eq!(
                        test.ignore.as_deref(),
                        Some(*reason),
                        "`{name}` is inventoried as pending `{reason}`, and its \
                         `#[ignore]` says otherwise"
                    );
                    assert!(
                        reason.starts_with("M1: "),
                        "a pending reason names the milestone and the feature: `{reason}`"
                    );
                }
            }
        }
    }
}

/// Every criterion is decided by a test, and every bullet contributes criteria —
/// a count worth pinning, because a milestone that loses a bullet loses it
/// quietly.
#[test]
fn every_bullet_contributes_criteria() {
    for (bullet, expected, what) in [
        (Bullet::Codegen, 10, "the codegen bullet's ten items"),
        (Bullet::Commands, 4, "the commands bullet's four items"),
        (Bullet::Harness, 1, "the harness bullet's single claim"),
        (Bullet::Strategy, 2, "CLAUDE.md's two generated-code gates"),
    ] {
        let count = rows()
            .filter(|criterion| criterion.bullet == bullet)
            .count();
        assert_eq!(count, expected, "{what}");
    }
}

/// Every acceptance fixture the suite names is a project on disk.
#[test]
fn every_acceptance_fixture_exists() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/m1");
    let mut found: Vec<String> = fs::read_dir(&projects)
        .expect("the acceptance fixture corpus exists")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();

    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/m1_acceptance/harness.rs"),
    )
    .expect("the harness is readable");
    for name in &found {
        assert!(
            source.contains(&format!("\"{name}\"")),
            "the fixture project `{name}` is not in the harness's `FIXTURES` list, \
             so nothing validates it"
        );
        assert!(
            projects.join(name).join("main.yml").is_file(),
            "the fixture project `{name}` has no entrypoint"
        );
    }
    assert!(!found.is_empty(), "the acceptance suite has fixtures");
}
