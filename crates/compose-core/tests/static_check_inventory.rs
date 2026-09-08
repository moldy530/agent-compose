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
//!
//! One row carries [`Evidence::Unreachable`] instead of a fixture, and says so
//! rather than borrowing another rule's: no composition v0 admits can trigger
//! it, so the code it names is pinned by fixtures for *other* rules and a check
//! that only asserted "the code is pinned somewhere" would report the row green
//! on evidence about something else.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::DiagnosticCode;

/// One static check: what it is, where it is decided, what it reports, and what
/// proves it fires.
struct Check {
    /// The rule, as PRD §7 M0 or `docs/grammar.md` names it.
    rule: &'static str,
    /// The module that decides it, as a path under `crates/compose-core/src/`.
    pass: &'static str,
    /// The diagnostic codes it reports through.
    codes: &'static [&'static str],
    /// What shows the rule really fires.
    evidence: Evidence,
}

/// What a row offers as proof that the rule behind it is a check rather than an
/// inventory entry.
enum Evidence {
    /// Some fixture in the negative corpora pins every code the row names, on
    /// this rule. The ordinary case.
    Fixture,
    /// No composition this grammar admits can trigger the rule, so there is no
    /// fixture to write and the codes it names are pinned by other rules'.
    /// The named unit test, in one of the row's own passes, holds the premise
    /// that makes it unreachable — when that premise stops being true the test
    /// fails, and the fixture becomes writable.
    Unreachable(&'static str),
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
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "schema compatibility across edges",
        pass: "check/bindings.rs, check/channels.rs",
        codes: &["type-mismatch", "unknown-field", "missing-binding"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "routing exhaustiveness (edges and map variants)",
        pass: "check/routing.rs (edges), check/maps.rs (map variants)",
        codes: &["non-exhaustive", "unknown-variant", "invalid-value"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "SCC cycle-termination",
        pass: "check/cycles.rs",
        codes: &["unbounded-cycle", "dead-end"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "fan-out bounding",
        pass: "check/maps.rs (the array `over` resolves to), parse/flow.rs (`max_concurrency`)",
        codes: &["unbounded-fan-out", "missing-key"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "reducer-channel write rules inside maps",
        pass: "check/maps.rs",
        codes: &["unreduced-write", "detached-write"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "trigger input-binding compatibility",
        pass: "check/triggers.rs",
        codes: &["type-mismatch", "unknown-field", "missing-binding"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "sync-trigger interrupt-free reachability",
        pass: "check/components.rs, over check/reach.rs",
        codes: &["sync-trigger-interrupt"],
        evidence: Evidence::Fixture,
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
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "session-scope/session-key coherence",
        pass: "check/stores.rs, over check/reach.rs",
        codes: &["missing-session-key"],
        evidence: Evidence::Fixture,
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
        evidence: Evidence::Fixture,
    },
    // Every v0 provider kind publishes both inference capabilities
    // (`check::providers`' table), so every route is capability-equivalent by
    // construction and this rule cannot fire on a composition the grammar
    // admits. `missing-capability` is pinned by the `embed.provider` rule, which
    // is a different rule reading the same table — so this row states its own
    // evidence rather than resting on that fixture.
    Check {
        rule: "route capability equivalence",
        pass: "check/providers.rs",
        codes: &["missing-capability"],
        evidence: Evidence::Unreachable("every_v0_kind_publishes_the_same_inference_capabilities"),
    },
    Check {
        rule: "env-ref syntax",
        pass: "parse/lexical.rs",
        codes: &["invalid-env-ref", "unexpected-env-ref"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "unreachable nodes",
        pass: "check/reachable.rs",
        codes: &["unreachable-node"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "undefined state channels",
        pass: "check/channels.rs, cel/mod.rs",
        codes: &["undefined-channel"],
        evidence: Evidence::Fixture,
    },
];

/// The rules `docs/grammar.md` Appendix B names that PRD §7 M0's sentence does
/// not enumerate individually.
///
/// All but one are the validator's; grammar 12.1's conditional credential (D120)
/// is the parser's, because a provider's `kind:`, `api_key:` and `base_url:` are
/// three literals in one mapping and the published schema enforces it — which
/// obliges the parser to as well (`tests/parse_invalid.rs`'s
/// `the_parser_rejects_everything_the_published_schema_rejects`). It is listed
/// here rather than left out because this file is the inventory of *static
/// checks*, and which pass decides one is the second column, not the entry
/// criterion.
///
/// Two rows are the exception and label themselves. Half of `duplicate-route` —
/// the half about two triggers rather than about the routes the app mounts for
/// itself — and the whole of `conflicting-session-key` are rules the compiler
/// applies that the grammar does not state, kept because each refuses an
/// ambiguity the emitted project would otherwise resolve by picking, and
/// reported as doc defects at their own rows and in `check/triggers.rs::routes`
/// and `::manual_session_keys`. A reader auditing this list against the grammar
/// should find every other row there.
const GRAMMAR: &[Check] = &[
    Check {
        rule: "balanced convergence (7.6.2, D112)",
        pass: "check/convergence.rs",
        codes: &["unbalanced-convergence"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "no silent dead ends: every node exits, and a skip keeps an escape (7.6.3 rules 1 and 3, D71)",
        pass: "check/routing.rs",
        codes: &["dead-end"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "concurrent branches write a reduced channel (7.6.1, 10.2, D32)",
        pass: "check/convergence.rs",
        codes: &["unreduced-write"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "recursion: no flow reaches itself (7.5, D26, D86)",
        pass: "check/components.rs, over check/reach.rs",
        codes: &["recursive-flow"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "`map.over` reads a dominating node (8.6 rule 11, D76)",
        pass: "check/fanout.rs",
        codes: &["non-dominating-source"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "`detach: true` under a durably checkpointed target (8.6 rule 7, D59)",
        pass: "check/fanout.rs",
        codes: &["unsupported-detach"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a detached dispatch reaches no `human` node (8.6 rule 7, 8.7, D118)",
        pass: "check/fanout.rs, over check/reach.rs",
        codes: &["detached-interrupt"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a detached dispatch's key is not part of its sink's declared input (9.4, D66)",
        pass: "check/maps.rs",
        codes: &["conflicting-keys"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The half of grammar 14.1's placement rules that needs two files. The
        // parser owns everything decidable from the deploy file alone — the
        // member forms, the `flow.*` deferral, disjointness, repeated members,
        // and the conditional join token — and the resolver owns whether a
        // member resolves. This is the rule that is about neither file on its
        // own: which agents attach what, and what those attachments reach, is
        // the composition's; which placement claims each is the active target's.
        rule: "an attachment colocates with the agent that attaches it, transitively through a `flow.*` (14.1, D129)",
        pass: "check/placements.rs, over check/reach.rs",
        codes: &["conflicting-placement"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The other half of grammar 14.1 that needs two files, and it needs a
        // third fact besides: which backend the *target* resolves the store to
        // (11.3, 14.3). The pass it runs over is `codegen/env.rs`'s partition
        // rather than a walk of its own — "can execute in a placement's
        // process" is the question that module already answers, and two
        // derivations of one closure is the drift the partition exists to
        // prevent (`docs/distributed.md` §9.1).
        rule: "a store on a process-local backend is opened by no placement's process (14.1 rule 5, D131)",
        pass: "check/placements.rs, over codegen/env.rs",
        codes: &["process-local-store"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "the effective write map is injective (8.0, D93)",
        pass: "check/channels.rs",
        codes: &["conflicting-writes"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "`flow:`-node and dispatch bindings are total (8.0, D68)",
        pass: "check/bindings.rs, check/maps.rs",
        codes: &["missing-binding"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "name-based wiring is type-checked in both directions (8.0, 7.5, D111)",
        pass: "check/bindings.rs, check/channels.rs",
        codes: &["type-mismatch", "undefined-channel"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a synthesized store tool does not collide with an attached one (11.5)",
        pass: "check/bindings.rs",
        codes: &["tool-name-collision"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The third row whose rule the spec does not state, and for the same
        // shape of reason as the two below. §11.5 states the collision rule for
        // a synthesized name against an attached one, and states the *reason* —
        // an attachment's name on the wire is its address's local name — but no
        // sentence and no Decision entry says what `tool.condense` beside
        // `flow.condense` on one agent means, and the published schema accepts
        // it. §5.4's duplicate rule is about entries rather than names, and the
        // parser raises that one. `check/bindings.rs::attached_tool_collisions`
        // reports the gap as a doc defect and says why the check is kept
        // meanwhile; the citation here is deliberately not a section number for
        // the rule itself, so this row cannot be read as evidence that the
        // grammar states it.
        rule: "two attached tools do not share a local name (compiler rule over 5.4's attachment naming and 11.5's collision rule)",
        pass: "check/bindings.rs",
        codes: &["tool-name-collision"],
        evidence: Evidence::Fixture,
    },
    // The built-ins of resolved q54, split across the two passes for the same
    // reason the server-tool rows below are. Every **bound** is a key beside
    // `builtin:` on one definition, so the whole of it is the parser's — which
    // name, which bounds that name takes, and the contract keys a built-in does
    // not declare. What the parser cannot decide is the only thing that needs
    // another file: whether the *name* the built-in takes on the wire is one
    // something else on this agent already takes.
    Check {
        rule: "a `builtin:` binding names one of the two and carries only the bounds that name takes (6.1, D135)",
        pass: "parse/definition.rs",
        codes: &[
            "unknown-variant",
            "unknown-key",
            "invalid-value",
            "wrong-type",
            "invalid-duration",
        ],
        evidence: Evidence::Fixture,
    },
    Check {
        // Two sites, as for a server tool and for the same reason: a built-in's
        // name meets an attached `tool.*`/`flow.*` in the agent's own list, and
        // meets a provider's suite only once the agent's model reaches that
        // provider. A *store's* synthesized names cannot collide with a built-in
        // at all — `<local>_<op>` is not a shape either name has — and
        // `check/bindings.rs` holds that premise as a test of its own rather
        // than as a comment. The comparison is over the **wire** name, which for
        // a configured built-in is the provider's rather than the definition
        // key's, so `tool.sandbox` binding `bash` collides exactly as the
        // shorthand does.
        rule: "a built-in's name is one no other tool this agent offers takes (5.5, 6.1, 11.5, D135)",
        pass: "check/bindings.rs",
        codes: &["tool-name-collision"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The third site of the same surface, and the one that is about a
        // *node* rather than about an agent. A `tool.*` binding a built-in is a
        // tool by address, so a `function:` node and a `map` dispatch can each
        // name one — and neither can call it: those two surfaces pass the
        // composition's arguments and read a declared result, and a built-in
        // declares neither. Left unchecked the spec reaches the emitter, which
        // writes no function for a built-in and then emits a node calling it.
        rule: "a built-in is called from an agent's `tools:` list and from nowhere else (5.5, 6.1, D135)",
        pass: "check/bindings.rs",
        codes: &["invalid-value"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a provider with a default endpoint declares `api_key:` or a `base_url:` (12.1, D120)",
        pass: "parse/definition.rs",
        codes: &["missing-credential"],
        evidence: Evidence::Fixture,
    },
    // The two-tier rule of resolved q30, split across the two passes it is
    // decidable in. The **kind gate** is one literal beside another in one
    // mapping, so it is the parser's, exactly as D120's credential rule is; the
    // **contents** need the kind's curated table read for a warning as well as
    // for an error, which is the same shape of work `settings:` already does
    // here.
    Check {
        rule: "`server_tools:` is declared on a kind whose wire carries them (12.1, D122)",
        pass: "parse/definition.rs",
        codes: &["unsupported-server-tools"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a `server_tools:` entry in the kind's curated table is checked against it, and one outside it — tool or field — is carried with a warning (12.1, D122)",
        pass: "check/providers.rs",
        codes: &[
            "unknown-server-tool",
            "unknown-server-tool-field",
            "missing-key",
            "conflicting-keys",
            "type-mismatch",
            "unknown-variant",
            "value-out-of-range",
            // The two spellings of "this key is read at compile time, and an
            // `${ENV}` reaches the wire as the string it expands to". A field
            // the table types as a **non-string** cannot carry one at all
            // (`type-mismatch`); a field it pins to a **single** string may
            // only carry that string, since the value is decided by the
            // entry's own `type:` and the service refuses any other
            // (`unexpected-env-ref`). A closed set of more than one is a knob a
            // deployment turns and interpolates like any class 2 value.
            "unexpected-env-ref",
        ],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a route's members declare one `server_tools:` suite (12.2, D122)",
        pass: "check/providers.rs",
        codes: &["mismatched-server-tools"],
        evidence: Evidence::Fixture,
    },
    Check {
        // §11.5's collision rule reached from the connection's side, which is
        // why the citation carries both sections: the *reason* is §11.5's
        // verbatim ("the model is offered two different things under one name")
        // and the array the names share is §12.1's, since a suite is appended
        // to the `tools` of every request the provider serves. Two sites,
        // because a suite collides with itself in one provider definition and
        // with a client tool only once an agent's model reaches that provider.
        rule: "a `server_tools:` suite offers each name once, and none an agent's own tools take (12.1, 11.5, D122)",
        pass: "check/providers.rs, check/bindings.rs",
        codes: &["tool-name-collision"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a `method: GET` trigger does not read through `payload.body` (13.3, D117)",
        pass: "check/triggers.rs",
        codes: &["invalid-expression"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The one row half of whose rule the spec does not state. Its **second**
        // half does: §13.3 has a generated app expose `status` and `resume`
        // beside each trigger's `start`, so those two routes are taken and a
        // trigger cannot claim one. Its **first** half is derived rather than
        // written — §13.3 fixes the mount model (one route per `http` trigger,
        // at the `path:` and `method:` each one *defaults*) but no sentence and
        // no Decision entry forbids two triggers landing on one pair, and the
        // published schema accepts it. `check/triggers.rs::routes` reports that
        // as a doc defect and says why the check is kept meanwhile; the citation
        // here is deliberately not a section number for the half the grammar
        // does not state, so this row cannot be read as evidence that it does.
        rule: "an `http` trigger's route is free: unclaimed by another trigger (compiler rule over 13.3's mount model), and not one the generated app mounts for itself (13.3)",
        pass: "check/triggers.rs",
        codes: &["duplicate-route"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The second row whose rule the spec does not state, and for the same
        // shape of reason as the one above. §13.2 makes a `manual` trigger's
        // `session_key:` a legal *remap* of the CLI's `--session`, and §13's
        // preamble makes the entry it remaps per **flow** ("it contributes no
        // entry to the IR's trigger table and has no name") — but no sentence
        // and no Decision entry says what two `manual` triggers on one flow
        // declaring two different remaps mean, and the published schema accepts
        // it. `check/triggers.rs::manual_session_keys` reports that as a doc
        // defect and says why the check is kept meanwhile; the citation here is
        // deliberately not a section number for the rule itself, so this row
        // cannot be read as evidence that the grammar states it.
        rule: "two `manual` triggers naming one flow agree about `session_key:` (compiler rule over 13.2's remap and 13's per-flow CLI entry)",
        pass: "check/triggers.rs",
        codes: &["conflicting-session-key"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The inbound half of grammar 13.3's authentication surface. Two rules
        // in one row because they are one shape read twice: an `auth:` block
        // declares exactly one scheme, and both schemes' secrets take the
        // env-ref value form §4.3 fixes for every credential in this grammar.
        rule: "an inbound `auth:` block declares exactly one scheme, whose secret is an `${ENV}` reference (13.3, 4.3, D41, D125)",
        pass: "parse/section.rs",
        codes: &["missing-key", "conflicting-keys", "invalid-env-ref"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The shape of the two keys every scheme block shares. A row of its own
        // rather than a clause on the two beside it, because it is the only
        // rule here about what a *resolved* value does downstream: a delivery
        // writes `header:` and `prefix:` onto its own request verbatim, so a
        // colon or a carriage return in either forges a second header — and a
        // name the delivery already writes, inside its own `X-AgentCompose-`
        // namespace or among the `Content-Type`, `Content-Length` and `Host` any
        // POST of a JSON body carries, collides with a header the same request
        // already sends (D127).
        rule: "an auth block's `header:` is one HTTP header name that no delivery already writes, and its `prefix:` carries no control character, inbound and outbound alike (13.3, 12.1, D127)",
        pass: "parse/section.rs",
        codes: &["invalid-value", "unexpected-env-ref"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The outbound half. `missing-callback-allowlist` is its own code for
        // the reason `missing-credential` is: what is absent is decided by a
        // sibling, and the repair is a choice of two.
        rule: "a `callback_auth:` block declares a scheme, brings a non-empty `callback_allow:` of absolute http/https patterns each naming a host, and neither key stands without a `callback:` (13.3, D126, D127)",
        pass: "parse/section.rs",
        codes: &[
            "missing-key",
            "missing-callback-allowlist",
            "conflicting-keys",
            "invalid-value",
        ],
        evidence: Evidence::Fixture,
    },
    // The `module:` binding of resolved q48 and q49, split across three passes
    // by the evidence each rule needs. The **path form**, the fence around the
    // project root, the collision with a name `build` emits, the exactness of a
    // version and the collision with the generated project's own pins are all
    // one file against a constant, so they are the parser's — the same split
    // D120's credential rule takes. What needs the whole composition is
    // agreement *between* tools, and what needs the filesystem is whether the
    // file is there.
    Check {
        rule: "a `module:` path is a project-relative TypeScript implementation — never a `.d.ts` — inside the project, at a name `build` does not emit (6.1, D132)",
        pass: "parse/binding.rs",
        codes: &["invalid-module-path", "missing-key", "unexpected-env-ref"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "a `module:` dependency is an npm package at an exact version the generated project does not contradict (6.1, D133)",
        pass: "parse/binding.rs",
        codes: &["invalid-dependency"],
        evidence: Evidence::Fixture,
    },
    Check {
        rule: "the `module:` bindings of a composition agree: one file per tool, one version per package (6.1, D132, D133)",
        pass: "check/modules.rs",
        codes: &["invalid-module-path", "invalid-dependency"],
        evidence: Evidence::Fixture,
    },
    Check {
        // The one rule in this file that reads the **filesystem**, which is why
        // it is not part of `check` at all: `build` scaffolds an absent module
        // rather than refusing over it, so a rule every verb ran would stop the
        // command that repairs it. `validate` and `build --check` run it; see
        // `check::modules`.
        rule: "a `module:` binding's authored file is on disk (6.1, D132)",
        pass: "check/modules.rs",
        codes: &["io-error"],
        evidence: Evidence::Fixture,
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
        evidence: Evidence::Fixture,
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
        if matches!(check.evidence, Evidence::Unreachable(_)) {
            continue;
        }
        for code in check.codes {
            assert!(
                pinned.contains(*code),
                "`{}` reports `{code}`, which no negative fixture pins",
                check.rule
            );
        }
    }
}

/// A row that offers no fixture names a unit test that exists, in one of its own
/// passes — the premise that makes its rule unreachable, held somewhere a change
/// to the table would break it.
///
/// Without this, [`Evidence::Unreachable`] would be a way to exempt a row from
/// evidence rather than a different kind of it: the marker has to cost something
/// to carry.
#[test]
fn a_row_that_offers_no_fixture_names_a_unit_test_that_holds_its_premise() {
    let mut unreachable = 0;
    for check in rows() {
        let Evidence::Unreachable(unit_test) = check.evidence else {
            continue;
        };
        unreachable += 1;
        let needle = format!("fn {unit_test}()");
        let found = modules(check).into_iter().any(|module| {
            fs::read_to_string(crate_root().join("src").join(module))
                .expect("a readable source file")
                .contains(&needle)
        });
        assert!(
            found,
            "`{}` rests on `{unit_test}`, which is not a test in `{}`",
            check.rule, check.pass
        );
    }
    assert_eq!(
        unreachable, 1,
        "a row that no composition can trigger is a claim worth counting; \
         update this number and say why in the header when one is added or removed"
    );
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

/// The modules one row names, as paths under `crates/compose-core/src/`.
fn modules(check: &'static Check) -> Vec<&'static str> {
    check
        .pass
        .split(|character: char| character.is_whitespace() || character == ',')
        .filter(|token| token.ends_with(".rs"))
        .collect()
}

/// Each pass named is a module that exists, so a renamed file cannot leave the
/// inventory pointing at nothing.
#[test]
fn every_named_pass_is_a_module() {
    for check in rows() {
        let modules = modules(check);
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
