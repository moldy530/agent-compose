//! The M0 completion inventory: every static check the PRD promises, mapped to
//! the pass that decides it and the diagnostic codes that report it.
//!
//! PRD §7 M0 lists the static checks by name and calls `agent-compose validate`
//! "the product's core loop". This file is that list, transcribed verbatim and
//! made executable: each entry names the pass and the codes, and the tests below
//! prove the codes exist, that each one really fires somewhere in the negative
//! corpora, and — the direction that matters most — that **nothing the validator
//! can report is missing from the inventory**. The last assertion reads the
//! validator's own source for every `DiagnosticCode` it mentions, so a new check
//! added without a line here fails this file rather than passing unnoticed.
//!
//! [`GRAMMAR`] is the companion list: the static rules `docs/grammar.md`
//! Appendix B assigns to the validator that PRD §7 M0's sentence does not name
//! individually. They are not extras — balanced convergence and the no-dead-end
//! rules are what make PRD 5.3's and 5.4's promises hold at run time — but they
//! come from the grammar rather than from the PRD's own enumeration, so they are
//! listed apart to keep the transcription honest.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::DiagnosticCode;

/// One static check: what it is, where it is decided, and what it reports.
struct Check {
    /// The rule, as PRD §7 M0 or `docs/grammar.md` names it.
    rule: &'static str,
    /// The module that decides it, as a path under `crates/compose-core/src/`.
    pass: &'static str,
    /// The diagnostic codes it reports through.
    codes: &'static [&'static str],
}

/// PRD §7 M0's static-check list, in the order the PRD writes it.
///
/// The `rule` strings are the PRD's own phrases, so
/// [`the_inventory_transcribes_the_prd_list`] can compare them against the
/// sentence itself rather than against a paraphrase.
const M0: &[Check] = &[
    Check {
        rule: "reference/type resolution",
        pass: "resolve/references.rs",
        codes: &["undefined-reference", "invalid-reference"],
    },
    Check {
        rule: "schema compatibility across edges",
        pass: "check/bindings.rs, check/channels.rs",
        codes: &["type-mismatch", "unknown-field", "missing-binding"],
    },
    Check {
        rule: "routing exhaustiveness (edges and map variants)",
        pass: "check/routing.rs (edges), check/maps.rs (map variants)",
        codes: &["non-exhaustive", "unknown-variant", "invalid-value"],
    },
    Check {
        rule: "SCC cycle-termination",
        pass: "check/cycles.rs",
        codes: &["unbounded-cycle", "dead-end"],
    },
    Check {
        rule: "fan-out bounding",
        pass: "check/maps.rs (the array `over` resolves to), parse/flow.rs (`max_concurrency`)",
        codes: &["unbounded-fan-out", "missing-key"],
    },
    Check {
        rule: "reducer-channel write rules inside maps",
        pass: "check/maps.rs",
        codes: &["unreduced-write", "detached-write"],
    },
    Check {
        rule: "trigger input-binding compatibility",
        pass: "check/triggers.rs",
        codes: &["type-mismatch", "unknown-field", "missing-binding"],
    },
    Check {
        rule: "sync-trigger interrupt-free reachability",
        pass: "check/components.rs, over check/reach.rs",
        codes: &["sync-trigger-interrupt"],
    },
    Check {
        rule: "store-op schema checks and map-write keying",
        pass: "check/stores.rs",
        codes: &[
            "type-mismatch",
            "unknown-field",
            "missing-key",
            "unkeyed-map-write",
        ],
    },
    Check {
        rule: "session-scope/session-key coherence",
        pass: "check/stores.rs, over check/reach.rs",
        codes: &["missing-session-key"],
    },
    Check {
        rule: "provider settings-schema and capability checks",
        pass: "check/providers.rs",
        codes: &[
            "unknown-key",
            "unknown-variant",
            "value-out-of-range",
            "type-mismatch",
            "missing-capability",
        ],
    },
    Check {
        rule: "route capability equivalence",
        pass: "check/providers.rs",
        codes: &["missing-capability"],
    },
    Check {
        rule: "env-ref syntax",
        pass: "parse/lexical.rs",
        codes: &["invalid-env-ref", "unexpected-env-ref"],
    },
    Check {
        rule: "unreachable nodes",
        pass: "check/reachable.rs",
        codes: &["unreachable-node"],
    },
    Check {
        rule: "undefined state channels",
        pass: "check/channels.rs, cel/mod.rs",
        codes: &["undefined-channel"],
    },
];

/// The validator-owned rules `docs/grammar.md` Appendix B names that PRD §7 M0's
/// sentence does not enumerate individually.
const GRAMMAR: &[Check] = &[
    Check {
        rule: "balanced convergence (7.6.2, D112)",
        pass: "check/convergence.rs",
        codes: &["unbalanced-convergence"],
    },
    Check {
        rule: "no silent dead ends: every node exits, and a skip keeps an escape (7.6.3 rules 1 and 3, D71)",
        pass: "check/routing.rs",
        codes: &["dead-end"],
    },
    Check {
        rule: "concurrent branches write a reduced channel (7.6.1, 10.2, D32)",
        pass: "check/convergence.rs",
        codes: &["unreduced-write"],
    },
    Check {
        rule: "recursion: no flow reaches itself (7.5, D26, D86)",
        pass: "check/components.rs, over check/reach.rs",
        codes: &["recursive-flow"],
    },
    Check {
        rule: "`map.over` reads a dominating node (8.6 rule 11, D76)",
        pass: "check/fanout.rs",
        codes: &["non-dominating-source"],
    },
    Check {
        rule: "`detach: true` under a durably checkpointed target (8.6 rule 7, D59)",
        pass: "check/fanout.rs",
        codes: &["unsupported-detach"],
    },
    Check {
        rule: "the effective write map is injective (8.0, D93)",
        pass: "check/channels.rs",
        codes: &["conflicting-writes"],
    },
    Check {
        rule: "`flow:`-node and dispatch bindings are total (8.0, D68)",
        pass: "check/bindings.rs, check/maps.rs",
        codes: &["missing-binding"],
    },
    Check {
        rule: "name-based wiring is type-checked in both directions (8.0, 7.5, D111)",
        pass: "check/bindings.rs, check/channels.rs",
        codes: &["type-mismatch", "undefined-channel"],
    },
    Check {
        rule: "a synthesized store tool does not collide with an attached one (11.5)",
        pass: "check/bindings.rs",
        codes: &["tool-name-collision"],
    },
    Check {
        rule: "a `method: GET` trigger does not read through `payload.body` (13.3, D117)",
        pass: "check/triggers.rs",
        codes: &["invalid-expression"],
    },
    Check {
        rule: "every CEL surface: roots, paths, constructs, and result type (4.1)",
        pass: "cel/mod.rs, over check/expr.rs",
        codes: &[
            "invalid-expression",
            "unknown-root",
            "unknown-field",
            "type-mismatch",
            "undefined-channel",
        ],
    },
];

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn rows() -> impl Iterator<Item = &'static Check> {
    M0.iter().chain(GRAMMAR.iter())
}

/// Every code, by the variant name the source spells it with.
fn by_variant() -> BTreeMap<String, &'static str> {
    DiagnosticCode::ALL
        .iter()
        .map(|code| (format!("{code:?}"), code.as_str()))
        .collect()
}

/// Every `DiagnosticCode::<Variant>` a source tree mentions.
fn mentioned(directories: &[&str]) -> BTreeSet<String> {
    let variants = by_variant();
    let mut found = BTreeSet::new();
    for directory in directories {
        let mut queue = vec![crate_root().join(directory)];
        while let Some(path) = queue.pop() {
            let entries = fs::read_dir(&path).expect("the source directory exists");
            for entry in entries {
                let path = entry.expect("a readable directory entry").path();
                if path.is_dir() {
                    queue.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }
                let source = fs::read_to_string(&path).expect("a readable source file");
                for (at, _) in source.match_indices("DiagnosticCode::") {
                    let name: String = source[at + "DiagnosticCode::".len()..]
                        .chars()
                        .take_while(char::is_ascii_alphanumeric)
                        .collect();
                    if let Some(code) = variants.get(&name) {
                        found.insert((*code).to_string());
                    }
                }
            }
        }
    }
    found
}

/// Every code some negative fixture pins, across the three corpora.
fn pinned() -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for corpus in ["invalid-parse", "invalid-resolve", "invalid-check"] {
        let directory = crate_root().join("tests/fixtures").join(corpus);
        for entry in fs::read_dir(&directory).expect("the fixture corpus exists") {
            let path = entry.expect("a readable directory entry").path();
            let main = if path.is_dir() {
                path.join("main.yml")
            } else {
                path
            };
            let Ok(source) = fs::read_to_string(&main) else {
                continue;
            };
            for line in source.lines() {
                let Some(comment) = line.strip_prefix("# ") else {
                    break;
                };
                if let Some(code) = comment.strip_prefix("code:") {
                    found.insert(code.trim().to_string());
                }
            }
        }
    }
    found
}

/// The PRD's own sentence, so the transcription cannot drift from it.
#[test]
fn the_inventory_transcribes_the_prd_list() {
    let prd = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("the manifest directory has a grandparent")
            .join("prd.md"),
    )
    .expect("the PRD is readable");
    let sentence = prd
        .lines()
        .find(|line| line.trim_start().starts_with("- Static checks:"))
        .expect("PRD §7 M0 lists the static checks");
    for check in M0 {
        assert!(
            sentence.contains(check.rule),
            "`{}` is not a phrase of PRD §7 M0's static-check list",
            check.rule
        );
    }
    let listed: Vec<&str> = sentence
        .trim_start()
        .trim_start_matches("- Static checks:")
        .trim_end_matches('.')
        .split(", ")
        .map(str::trim)
        .collect();
    assert_eq!(
        listed.len(),
        M0.len(),
        "PRD §7 M0 lists {} checks and the inventory has {}: {listed:?}",
        listed.len(),
        M0.len()
    );
}

/// Every code the inventory names is a real one.
#[test]
fn every_named_code_exists() {
    let known: BTreeSet<&str> = DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    for check in rows() {
        assert!(!check.codes.is_empty(), "`{}` names no code", check.rule);
        for code in check.codes {
            assert!(
                known.contains(code),
                "`{}` names `{code}`, which is not a `DiagnosticCode`",
                check.rule
            );
        }
    }
}

/// Every code the inventory names really fires: some fixture in the negative
/// corpora pins it. A rule mapped to a code nothing can produce is an inventory
/// entry, not a check.
#[test]
fn every_named_code_is_pinned_by_a_fixture() {
    let pinned = pinned();
    for check in rows() {
        for code in check.codes {
            assert!(
                pinned.contains(*code),
                "`{}` reports `{code}`, which no negative fixture pins",
                check.rule
            );
        }
    }
}

/// The direction that keeps the inventory honest: everything the validator — the
/// checks and the CEL front-end they run — can report is accounted for here.
#[test]
fn the_inventory_accounts_for_every_code_the_validator_raises() {
    let inventoried: BTreeSet<&str> = rows()
        .flat_map(|check| check.codes.iter().copied())
        .collect();
    let raised = mentioned(&["src/check", "src/cel"]);
    let missing: Vec<&String> = raised
        .iter()
        .filter(|code| !inventoried.contains(code.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "the validator raises these codes and the M0 inventory does not account for them: {missing:?}"
    );
}

/// Each pass named is a module that exists, so a renamed file cannot leave the
/// inventory pointing at nothing.
#[test]
fn every_named_pass_is_a_module() {
    for check in rows() {
        let modules: Vec<&str> = check
            .pass
            .split(|character: char| character.is_whitespace() || character == ',')
            .filter(|token| token.ends_with(".rs"))
            .collect();
        assert!(!modules.is_empty(), "`{}` names no pass", check.rule);
        for module in modules {
            assert!(
                crate_root().join("src").join(module).is_file(),
                "`{}` names the pass `{module}`, which is not a module",
                check.rule
            );
        }
    }
}
