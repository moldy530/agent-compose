//! The OTLP conformance corpus: the exporter's answer, pinned byte for byte, and
//! held to the shape `docs/trace.md` §12 publishes.
//!
//! PRD resolved q51 buys a hand-written exporter by paying for it here. The trade
//! it states is explicit — "OTLP/JSON over HTTP is a stable spec the hub can emit
//! directly, so the exporter is hand-written and held by a **conformance fixture
//! corpus** (the CEL-corpus discipline applied to span output) rather than by a
//! vendored SDK" — so what stands between this project and a collector that drops
//! its exports is this file and the fixtures beside it.
//!
//! # The two halves, and why neither is enough alone
//!
//! **The bytes.** Each fixture under `tests/fixtures/otlp-conformance/` pairs a
//! trace envelope and an export context with the exact
//! `ExportTraceServiceRequest` the mapping must produce.
//! `tests/toolchain/otlp-conformance.mjs` runs every one of them through the
//! **emitted** `src/otlp.ts` — the module a deployment really ships, not a port
//! of it — and reports divergences. That is what makes a change to the mapping
//! visible in a diff.
//!
//! Expectations that are *bytes* are regenerated rather than typed
//! (`UPDATE_GOLDENS=1`), which on its own would be a test that agrees with
//! itself. So:
//!
//! **The shape.** This file reads the corpus in Rust and asserts the invariants
//! §12 states as properties rather than as literals: ids are lowercase hex of the
//! right width and never all zero, every span but the root names a parent that is
//! a span of the same export, no two spans share an id, every timestamp — a
//! span's window and every event's instant alike — is an integer written as a
//! string and sits where §12.4 says it sits, every link resolves, and the
//! resource attributes are exactly the documented seven. A regenerated
//! expectation that broke any of those fails here, whatever the runner said.
//!
//! Together they are the CEL corpus's two directions: one implementation answering
//! a fixed corpus, and the corpus itself held to a specification.

#[path = "support/goldens.rs"]
mod goldens;
#[path = "support/toolchain.rs"]
mod toolchain;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use goldens::{files_under, goldens_root};
use toolchain::{installed, runner};

/// Where the corpus lives.
fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/otlp-conformance")
}

/// The `traceparent` corpus: one file, because each case is one string.
fn traceparent_corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/traceparent-conformance.json")
}

/// Every fixture, by file name, oldest-sorted so a failure names a stable one.
fn fixtures() -> Vec<(String, Value)> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus())
        .expect("the otlp-conformance corpus directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|held| held == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "the OTLP conformance corpus is empty");
    files
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|held| held.to_str())
                .unwrap_or_default()
                .to_string();
            let source = fs::read_to_string(&path).expect("a readable corpus file");
            let value: Value = serde_json::from_str(&source)
                .unwrap_or_else(|error| panic!("{name} is not a JSON object: {error}"));
            (name, value)
        })
        .collect()
}

/// One golden, copied into the toolchain fixture so the runner can import it.
///
/// The exporter is a compiler constant — `codegen::otlp`'s
/// `the_exporter_is_the_same_module_in_every_project` is what says so — so one
/// golden answers the corpus for all of them, exactly as one golden answers the
/// CEL corpus.
fn staged(root: &Path) -> PathBuf {
    let golden = goldens::golden("review-loop");
    let destination = root
        .join("projects/otlp-conformance")
        .join(golden.directory);
    let _ = fs::remove_dir_all(&destination);
    let source = goldens_root().join(golden.directory);
    for relative in files_under(&source) {
        let target = destination.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(target.parent().expect("a staged path has a parent"))
            .expect("the scratch area is writable");
        fs::copy(source.join(&relative), &target).expect("a golden file is copyable");
    }
    destination
}

/// **The corpus, answered by the module a deployment ships.**
///
/// `UPDATE_GOLDENS=1` rewrites the expectations from what the exporter produced,
/// the way every other byte-for-byte expectation in this repository is
/// maintained; the run after it is what has to pass.
#[test]
fn the_emitted_exporter_answers_the_conformance_corpus() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(root);
    let mut command = runner("otlp-conformance.mjs");
    command.arg(&project).arg(corpus());
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        command.arg("--write");
    }
    let output = command.output().expect("bun runs");
    assert!(
        output.status.success(),
        "the OTLP corpus did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answered = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        answered.trim(),
        "[]",
        "the emitted OTLP exporter answers the shared corpus differently from what it holds; \
         `docs/trace.md` §12 is the mapping and PRD resolved q51 is why this corpus stands in \
         for an SDK. Regenerate with `UPDATE_GOLDENS=1` only after reading the diff."
    );
}

/// **The inbound `traceparent`, arm by arm, through the emitted parser.**
///
/// The export corpus above cannot ask this. An ignored header and a refused one
/// produce the same well-formed export — the run derives a trace id of its own
/// either way — so every refusal in `docs/trace.md` §12.3 is invisible from the
/// bytes and has to be asked of the function. A guard that stopped guarding would
/// otherwise leave the whole suite green while the exporter adopted a reserved
/// version or minted an all-zero trace id a collector drops.
#[test]
fn the_emitted_parser_answers_the_traceparent_corpus() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(root);
    let mut command = runner("traceparent-conformance.mjs");
    command.arg(&project).arg(traceparent_corpus());
    let output = command.output().expect("bun runs");
    assert!(
        output.status.success(),
        "the traceparent corpus did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "[]",
        "the emitted `parseTraceparent` answers the shared corpus differently from what it \
         holds; `docs/trace.md` §12.3 is what a header is allowed to be and PRD resolved q51's \
         first amendment is why an inbound one matters at all"
    );
}

/// **Every refusal arm is reached by some case, and no case invents an arm.**
///
/// The structural half of the corpus above, and the one that keeps it from
/// decaying: a suite that ran the parser over four valid headers would be green
/// and would prove nothing about the refusals. `codegen::otlp`'s
/// `TRACEPARENT_REFUSALS` is the list, its
/// `the_header_parser_has_the_guards_the_corpus_answers` is what stops a guard
/// being added without one, and this is what stops a case being deleted.
#[test]
fn every_traceparent_refusal_arm_has_a_case() {
    let source = fs::read_to_string(traceparent_corpus()).expect("the traceparent corpus is there");
    let cases: Vec<Value> = serde_json::from_str(&source).expect("…and is an array of cases");
    let mut named: BTreeSet<String> = BTreeSet::new();
    let mut names: BTreeSet<String> = BTreeSet::new();
    let mut accepted = 0;
    for held in &cases {
        let name = held["name"]
            .as_str()
            .expect("a case has a name")
            .to_string();
        assert!(
            names.insert(name.clone()),
            "two traceparent cases are named `{name}`; a divergence has to name one"
        );
        assert!(
            held["about"].is_string(),
            "the case `{name}` says nothing about what it is for"
        );
        match held["refuses"].as_str() {
            Some(arm) => {
                assert!(
                    held["expected"].is_null(),
                    "the case `{name}` names the arm `{arm}` and still expects a parse"
                );
                named.insert(arm.to_string());
            }
            None => {
                assert!(
                    held["expected"].is_object(),
                    "the case `{name}` refuses nothing, so it expects a parsed header"
                );
                accepted += 1;
            }
        }
    }
    let wanted: BTreeSet<String> = compose_core::codegen::otlp::TRACEPARENT_REFUSALS
        .iter()
        .map(|held| (*held).to_string())
        .collect();
    assert_eq!(
        named, wanted,
        "the corpus covers a different set of refusal arms than \
         `codegen::otlp::TRACEPARENT_REFUSALS` names"
    );
    assert!(
        accepted > 0,
        "a corpus of refusals alone would not notice a parser that refused everything"
    );
}

/// **Every root-span status arm is reached by some fixture** (`docs/trace.md`
/// §12.4).
///
/// The sibling of `every_traceparent_refusal_arm_has_a_case`, aimed at the other
/// place this corpus stands in for an SDK. §12.4's first row publishes three
/// mappings, and a reader may rely on any of them; the exporter's own callers
/// reach only two — `shipTrace` is handed *settled* executions, and a run holding
/// a pause is not one — so the `interrupted` arm has no path into these bytes
/// except a fixture that names it. Without this the row could be published,
/// exported wrongly, and never noticed: `agent-compose run` writes exactly that
/// document (`docs/trace.md` §2), and a later change that made a parked
/// execution exportable would ship whatever the arm had drifted into.
///
/// `codegen::otlp::ENVELOPE_STATUSES` is the list, and its
/// `the_root_status_arms_are_the_formats` is what stops a status being added to
/// the format without one.
#[test]
fn every_root_status_arm_has_a_fixture() {
    let mut reached: BTreeSet<String> = BTreeSet::new();
    for (file, fixture) in fixtures() {
        let status = fixture["document"]["status"]
            .as_str()
            .unwrap_or_else(|| panic!("{file}: an envelope declares a `status`"));
        reached.insert(status.to_string());
    }
    let wanted: BTreeSet<String> = compose_core::codegen::otlp::ENVELOPE_STATUSES
        .iter()
        .map(|held| (*held).to_string())
        .collect();
    assert_eq!(
        reached, wanted,
        "the corpus reaches a different set of envelope statuses than \
         `codegen::otlp::ENVELOPE_STATUSES` names; `docs/trace.md` §12.4's first row maps each \
         of them to a root-span status, and an arm no fixture reaches is a published mapping \
         nothing holds honest"
    );
}

/// Every fixture carries an input **and** the bytes it expects.
///
/// A fixture with no expectation is one the runner would answer with a
/// `no-expectation` divergence — this says the same thing where a reader adding
/// a fixture will meet it, and it is the check the shape assertions below all
/// depend on.
#[test]
fn every_fixture_declares_its_input_and_its_expectation() {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for (file, fixture) in fixtures() {
        for key in ["name", "about", "context", "document", "expected"] {
            assert!(
                fixture.get(key).is_some(),
                "{file} declares no `{key}`: a corpus entry is an input, the bytes it expects, \
                 and a sentence saying what it is for"
            );
        }
        let name = fixture["name"].as_str().expect("a string name").to_string();
        assert!(
            names.insert(name.clone()),
            "two fixtures are named `{name}`; a divergence has to name one file"
        );
    }
}

/// **The shape `docs/trace.md` §12 publishes, over every expectation.**
///
/// The structural half of this corpus. It reads the committed bytes rather than
/// running anything, so it holds equally over an expectation somebody
/// regenerated: an exporter that started minting eight-byte trace ids, reusing a
/// span id, or parenting a span at an id nothing in the export declares would
/// still round-trip through the runner and would fail here.
#[test]
fn every_expectation_is_a_well_formed_export() {
    for (file, fixture) in fixtures() {
        let export = &fixture["expected"];
        let resources = export["resourceSpans"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: an export carries `resourceSpans`"));
        assert_eq!(
            resources.len(),
            1,
            "{file}: one execution is one resource (`docs/trace.md` §12)"
        );
        let scopes = resources[0]["scopeSpans"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: a resource carries `scopeSpans`"));
        assert_eq!(scopes.len(), 1, "{file}: one export is one scope");
        assert_eq!(
            scopes[0]["scope"]["name"], "agent-compose",
            "{file}: the instrumentation scope is this project's"
        );

        // §12.6's promise, read as a list rather than as a set: a later metrics
        // exporter correlates by resourcing itself the same way.
        let resource: Vec<&str> = resources[0]["resource"]["attributes"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: a resource carries attributes"))
            .iter()
            .map(|attribute| {
                attribute["key"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{file}: an attribute has a string key"))
            })
            .collect();
        assert_eq!(
            resource,
            compose_core::codegen::otlp::RESOURCE_ATTRIBUTES,
            "{file}: the resource attributes are not `docs/trace.md` §12.6's"
        );

        let spans = scopes[0]["spans"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: a scope carries spans"));
        assert!(!spans.is_empty(), "{file}: an execution is at least a root");

        let trace = hex(&file, &spans[0]["traceId"], 32);
        let mut ids: BTreeSet<String> = BTreeSet::new();
        let mut roots = 0;
        for span in spans {
            let id = hex(&file, &span["spanId"], 16);
            assert!(
                ids.insert(id.clone()),
                "{file}: two spans share the id `{id}`, which is one span to a collector \
                 (`docs/trace.md` §12.2)"
            );
            assert_eq!(
                hex(&file, &span["traceId"], 32),
                trace,
                "{file}: one execution is one trace"
            );
            let opened = stamp(&file, &span["startTimeUnixNano"], "startTimeUnixNano");
            let closed = stamp(&file, &span["endTimeUnixNano"], "endTimeUnixNano");

            // §12.4's third clause, over the instants only the events carry. The
            // runner cannot ask this: a regenerated expectation agrees with
            // whatever the mapper produced, so a stamp in milliseconds, or the
            // `"0"` its instant parser falls back to, would round-trip through
            // the corpus and reach a collector as an event in 1970. What the
            // document publishes is a placement, so a placement is what this
            // reads: an event sits at its span's start, and the one exception —
            // the second instant a pause contributes — sits at its span's end,
            // which is `settledAt` (§12.4's second clause).
            for event in span["events"]
                .as_array()
                .unwrap_or_else(|| panic!("{file}: a span carries an event array"))
            {
                let name = event["name"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{file}: an event has a string name"));
                let at = stamp(&file, &event["timeUnixNano"], "an event's `timeUnixNano`");
                let (wanted, which) = if name == "human.settled" {
                    (&closed, "its span's end, which is the pause's `settledAt`")
                } else {
                    (&opened, "its span's start")
                };
                assert_eq!(
                    &at, wanted,
                    "{file}: the event `{name}` on `{}` is stamped `{at}`; `docs/trace.md` §12.4 \
                     stamps it at {which} (`{wanted}`)",
                    span["name"]
                );
            }
            let kind = span["kind"].as_u64().unwrap_or_else(|| {
                panic!("{file}: a span's `kind` is the enum's number, not its name")
            });
            assert!(
                kind == 1 || kind == 3,
                "{file}: `kind` is {kind}; §12.1 uses internal (1) and client (3)"
            );
            let code = span["status"]["code"].as_u64().unwrap_or_else(|| {
                panic!("{file}: a span's status code is the enum's number, not its name")
            });
            assert!(
                code <= 2,
                "{file}: status code {code} is not one of §12.4's"
            );
            if span.get("parentSpanId").is_none() {
                roots += 1;
            } else {
                hex(&file, &span["parentSpanId"], 16);
            }
            for attribute in span["attributes"]
                .as_array()
                .unwrap_or_else(|| panic!("{file}: a span carries an attribute array"))
            {
                let key = attribute["key"].as_str().expect("a string key");
                assert!(
                    key.starts_with("agentcompose."),
                    "{file}: the span attribute `{key}` is outside this project's namespace \
                     (`docs/trace.md` §12.5)"
                );
            }
        }
        assert!(
            roots <= 1,
            "{file}: {roots} spans name no parent; an export is one tree"
        );

        // Every parent, and every link, resolves to a span of this export — with
        // the one exception §12.3 states: a root parented at the **caller's**
        // span, which is not one of ours.
        let caller = fixture["context"]["parent"]["spanId"].as_str();
        for span in spans {
            if let Some(parent) = span["parentSpanId"].as_str() {
                assert!(
                    ids.contains(parent) || Some(parent) == caller,
                    "{file}: the span `{}` is parented at `{parent}`, which this export does \
                     not declare and no `traceparent` names",
                    span["name"]
                );
            }
            for link in span["links"].as_array().into_iter().flatten() {
                let target = hex(&file, &link["spanId"], 16);
                assert!(
                    ids.contains(&target),
                    "{file}: a link points at `{target}`, which this export does not declare; \
                     a flow-as-tool join links a model call to the dispatch it filed \
                     (`docs/trace.md` §12.2)"
                );
                assert_eq!(
                    hex(&file, &link["traceId"], 32),
                    trace,
                    "{file}: a link leaves this trace"
                );
            }
        }
    }
}

/// The corpus holds a **harness run**, and holds one of each shape the record
/// has two of (`docs/trace.md` §7.6, PRD resolved q57).
///
/// PRD resolved q51 makes this corpus the thing standing between the exporter
/// and a collector that drops its spans, and a record type nothing in the corpus
/// carries is a mapping nothing exercises: the expectation would regenerate to
/// whatever the emitter answered, and the `UPDATE_GOLDENS` half of this suite
/// would agree with itself. So the fixture is required, and required to reach
/// the corners the record was designed around:
///
///  * both **harnesses**, because the two report different things and the whole
///    argument for the record's optional members is that one shape must not lie
///    about which;
///  * a run that **failed**, because a span with `code: 2` and an error message
///    is the reading an operator opens a trace for;
///  * a **cost in money** and a **usage per turn**, which are the two halves of
///    the rollup and the two attribute types — a `doubleValue` and an
///    `intValue` — this exporter emits for it.
#[test]
fn the_corpus_holds_a_harness_run_of_each_shape() {
    let mut harnesses: BTreeSet<String> = BTreeSet::new();
    let mut outcomes: BTreeSet<String> = BTreeSet::new();
    let mut money = false;
    let mut per_turn_usage = false;
    for (_, fixture) in fixtures() {
        for entry in fixture["document"]["entries"]
            .as_array()
            .into_iter()
            .flatten()
        {
            for run in entry["harness"].as_array().into_iter().flatten() {
                if let Some(name) = run["harness"].as_str() {
                    harnesses.insert(name.to_string());
                }
                if let Some(outcome) = run["outcome"].as_str() {
                    outcomes.insert(outcome.to_string());
                }
                money |= run["cost"]["usd"].is_number();
                per_turn_usage |= run["turns"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|turn| turn["usage"].is_object());
            }
        }
    }
    assert_eq!(
        harnesses,
        ["cc", "codex"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<String>>(),
        "the corpus exercises one harness, and the record's optional members are \
         exactly what the two disagree about"
    );
    assert_eq!(
        outcomes,
        ["completed", "failed"]
            .into_iter()
            .map(str::to_string)
            .collect::<BTreeSet<String>>(),
        "a failed run is the span a reader opens a trace for"
    );
    assert!(money, "no fixture carries a harness's own cost estimate");
    assert!(per_turn_usage, "no fixture carries usage reported per turn");
}

/// **The uniqueness above is asked of data that could break it.**
///
/// `every_expectation_is_a_well_formed_export` refuses two spans of one id, which
/// is worth nothing if no fixture can produce two. The shape that can is the one
/// §8 builds on purpose: a repeated attempt at one effect **reuses its instance
/// path**, so a retried `flow:` node files two `inner` instances at one path and a
/// retried tool loop files two dispatch records at one idempotency key. Derive a
/// span id from the bare path and the two siblings still differ — they carry
/// their own position — while everything *beneath* them collides, which a
/// collector renders as one span with two parents.
///
/// So this insists the corpus holds that shape: two spans at one instance path,
/// each with children of its own. It is the guard on the guard, and without it a
/// fixture deleted in good faith would silently disarm §12.2.
#[test]
fn the_corpus_holds_two_spans_at_one_instance_path_with_children() {
    let mut found: Vec<String> = Vec::new();
    for (file, fixture) in fixtures() {
        let spans = fixture["expected"]["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let mut children: BTreeMap<String, usize> = BTreeMap::new();
        for span in &spans {
            if let Some(parent) = span["parentSpanId"].as_str() {
                *children.entry(parent.to_string()).or_default() += 1;
            }
        }
        let mut at_path: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for span in &spans {
            let Some(id) = span["spanId"].as_str() else {
                continue;
            };
            for attribute in span["attributes"].as_array().into_iter().flatten() {
                if attribute["key"] == "agentcompose.instance_path"
                    && let Some(path) = attribute["value"]["stringValue"].as_str()
                {
                    at_path
                        .entry(path.to_string())
                        .or_default()
                        .push(id.to_string());
                }
            }
        }
        for (path, ids) in at_path {
            if ids.len() >= 2
                && ids
                    .iter()
                    .all(|id| children.get(id).copied().unwrap_or_default() >= 1)
            {
                found.push(format!("{file}: {path}"));
            }
        }
    }
    assert!(
        !found.is_empty(),
        "no fixture files two records at one instance path with entries beneath them, so \
         `every_expectation_is_a_well_formed_export`'s span-id uniqueness is asserted over data \
         that cannot violate it. `docs/trace.md` §8 is the shape: a retried `flow:` node's two \
         instances, or a retried tool loop's two dispatch records."
    );
}

/// **The event placement above is asked of data that could break it.**
///
/// `every_expectation_is_a_well_formed_export` reads every event's instant, which
/// is worth little while every one of them is the execution's own start: a mapper
/// that stamped the whole export at `window.start` would satisfy it. The instants
/// that can tell the difference are the two `docs/trace.md` §12.4 exempts — a
/// pause's `pausedAt` and `settledAt`, the only clock this format really records
/// (§3.4) — so this insists the corpus carries both, each away from the
/// execution's start, and that the pair straddles the wait rather than collapsing
/// onto one instant.
///
/// Without it a pause fixture retired in good faith would leave the placement
/// rule true of nothing, and a regression that stamped a wait of a day at the
/// instant the run began would ship.
#[test]
fn the_corpus_holds_a_pause_whose_two_events_carry_the_waits_own_instants() {
    let mut straddled: Vec<String> = Vec::new();
    for (file, fixture) in fixtures() {
        let spans = fixture["expected"]["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let Some(root) = spans.first() else { continue };
        let began = root["startTimeUnixNano"].clone();
        for span in &spans {
            let mut paused: Option<Value> = None;
            let mut settled: Option<Value> = None;
            for event in span["events"].as_array().into_iter().flatten() {
                match event["name"].as_str() {
                    Some("human.paused") => paused = Some(event["timeUnixNano"].clone()),
                    Some("human.settled") => settled = Some(event["timeUnixNano"].clone()),
                    _ => {}
                }
            }
            let (Some(paused), Some(settled)) = (paused, settled) else {
                continue;
            };
            if paused != began && settled != paused {
                straddled.push(format!("{file}: {}", span["name"]));
            }
        }
    }
    assert!(
        !straddled.is_empty(),
        "no fixture carries a settled `human` pause whose `human.paused` and `human.settled` \
         events fall away from the execution's start and away from each other, so \
         `every_expectation_is_a_well_formed_export`'s event placement is asserted over instants \
         that are all the same number. `docs/trace.md` §12.4 exempts exactly these two from being \
         stamped at their span's start, and §3.4 is the wait they measure."
    );
}

/// **The instant guard is asked of an instant that needs it.**
///
/// The exporter reads its instants with `Date.parse` and writes a fallback for
/// one it cannot read, because a clock that became the string `"NaN"` in a field
/// a collector parses as a number is worse than a clock that is merely
/// approximate. Every other fixture's instants are this runtime's own
/// `toISOString()`, so nothing reaches that fallback and whatever it degraded to
/// would round-trip through the corpus unnoticed — including `"0"`, the epoch,
/// which [`stamp`] refuses precisely because a wait rendered in 1970 is not an
/// instant anything happened at.
///
/// So the corpus keeps one document whose pause clocks cannot be read, and this
/// says what its export has to look like: the wait lands on the execution's own
/// start. `every_expectation_is_a_well_formed_export` then holds the other half
/// — a span's edges degrade to the same instants as the events stamped on them,
/// so §12.4's placement survives the degradation rather than being suspended by
/// it.
#[test]
fn the_corpus_holds_a_pause_whose_clock_cannot_be_read() {
    let mut reached: Vec<String> = Vec::new();
    for (file, fixture) in fixtures() {
        if !carries_an_unreadable_pause(&fixture["document"]) {
            continue;
        }
        let spans = fixture["expected"]["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let began = spans
            .first()
            .map(|root| root["startTimeUnixNano"].clone())
            .unwrap_or(Value::Null);
        let landed = spans.iter().any(|span| {
            span["events"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|event| event["name"] == "human.paused" && event["timeUnixNano"] == began)
        });
        assert!(
            landed,
            "{file}: a pause carries a `pausedAt` this exporter cannot read and no \
             `human.paused` event is stamped at the execution's start ({began}); an unreadable \
             instant falls back to a point inside the execution's window, never to the epoch \
             (`docs/trace.md` §12.4)"
        );
        reached.push(file);
    }
    assert!(
        !reached.is_empty(),
        "no fixture carries a `human` pause whose instants this exporter cannot parse, so its \
         instant fallback is reached by nothing in the corpus and a regression that degraded an \
         unreadable clock to the epoch would ship green"
    );
}

/// Whether some `human` pause under this envelope carries an instant the
/// exporter's `Date.parse` would refuse.
///
/// Read as the shape the runtime's own `toISOString()` writes, which is the only
/// shape it ever writes — a date, a `T`, and a clock. Recursive because an entry
/// nests: a pause may sit under `inner` or under a dispatch record's own entries.
fn carries_an_unreadable_pause(value: &Value) -> bool {
    match value {
        Value::Object(fields) => {
            if let Some(Value::Object(pause)) = fields.get("human")
                && ["pausedAt", "settledAt"].iter().any(|key| {
                    pause
                        .get(*key)
                        .and_then(Value::as_str)
                        .is_some_and(|held| !is_iso_instant(held))
                })
            {
                return true;
            }
            fields.values().any(carries_an_unreadable_pause)
        }
        Value::Array(held) => held.iter().any(carries_an_unreadable_pause),
        _ => false,
    }
}

/// `YYYY-MM-DDT…`, the leading shape of an ISO 8601 instant.
fn is_iso_instant(held: &str) -> bool {
    let bytes = held.as_bytes();
    bytes.len() >= 20
        && bytes[..4].iter().all(u8::is_ascii_digit)
        && bytes[4] == b'-'
        && bytes[5..7].iter().all(u8::is_ascii_digit)
        && bytes[7] == b'-'
        && bytes[8..10].iter().all(u8::is_ascii_digit)
        && bytes[10] == b'T'
}

/// **The caller's trace is adopted, not merely noted** (`docs/trace.md` §12.3).
///
/// The fixture that carries a `traceparent` and the one that does not are the
/// same document under the same context but for the header, so what differs
/// between their expectations is exactly what the amendment promises: the trace
/// id becomes the caller's, the root gains the caller's span as its parent, and
/// **every span id is unchanged** — an embedded graph joins its caller's trace
/// rather than becoming a different set of spans inside it.
#[test]
fn an_inbound_traceparent_moves_the_trace_id_and_the_roots_parent_and_nothing_else() {
    let held: BTreeMap<String, Value> = fixtures().into_iter().collect();
    let plain = &held["minimal-run.json"]["expected"];
    let joined = &held["traceparent-parented.json"]["expected"];
    let parent = held["traceparent-parented.json"]["context"]["parent"].clone();

    let spans = |export: &Value| -> Vec<Value> {
        export["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .expect("spans")
            .clone()
    };
    let one = spans(plain);
    let two = spans(joined);
    assert_eq!(one.len(), two.len(), "the two fixtures describe one run");

    assert_eq!(
        two[0]["traceId"], parent["traceId"],
        "the export adopts the caller's trace id outright"
    );
    assert_ne!(
        one[0]["traceId"], two[0]["traceId"],
        "…and the unparented run derives one of its own"
    );
    assert_eq!(
        two[0]["parentSpanId"], parent["spanId"],
        "the root span hangs off the caller's span"
    );
    assert!(
        one[0].get("parentSpanId").is_none(),
        "a run nobody traced into has no parent to name"
    );
    for (unparented, parented) in one.iter().zip(two.iter()) {
        assert_eq!(
            unparented["spanId"], parented["spanId"],
            "a span id is derived from the execution and the record, never from the trace it \
             ends up in (`docs/trace.md` §12.2)"
        );
        assert_eq!(unparented["name"], parented["name"]);
        assert_eq!(unparented["attributes"], parented["attributes"]);
    }
}

/// One OTLP instant: nanoseconds since the epoch, written as a string because
/// proto3's JSON mapping writes every 64-bit integer as one.
fn stamp(file: &str, value: &Value, what: &str) -> String {
    let held = value
        .as_str()
        .unwrap_or_else(|| panic!("{file}: {what} is a string (proto3 JSON int64), not {value}"));
    assert!(
        !held.is_empty() && held.chars().all(|character| character.is_ascii_digit()),
        "{file}: {what} is `{held}`, which is not an integer"
    );
    assert!(
        held.chars().any(|character| character != '0'),
        "{file}: {what} is `{held}` — the epoch, which is what an unreadable instant falls back \
         to rather than an instant anything really happened at"
    );
    held.to_string()
}

/// A hex id of the documented width, never all zero.
fn hex(file: &str, value: &Value, width: usize) -> String {
    let held = value
        .as_str()
        .unwrap_or_else(|| panic!("{file}: an id is a hex string, not {value}"));
    assert_eq!(
        held.len(),
        width,
        "{file}: `{held}` is not {width} hex characters (`docs/trace.md` §12.2)"
    );
    assert!(
        held.chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)),
        "{file}: `{held}` is not lowercase hex"
    );
    assert!(
        held.chars().any(|character| character != '0'),
        "{file}: `{held}` is all zero, which the W3C trace context forbids"
    );
    held.to_string()
}
