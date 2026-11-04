//! The cron conformance corpus, run against the Rust evaluator (grammar 13.4,
//! PRD resolved q78, Decision D162).
//!
//! A `schedule` trigger's `cron:` has two readers, the way a CEL expression
//! does: `validate` holds it to [`compose_core::cron::Cron::parse`]'s grammar,
//! and `serve` evaluates it with the emitted `src/cron.ts`. Two readings of one
//! grammar drift unless something holds them together, and the mitigation is
//! CEL's: one shared corpus of fixtures executed against both — this file is
//! the Rust column, `tests/toolchain/cron-conformance.mjs` the JavaScript one,
//! run under Bun by the acceptance suite and under the Node fallback by gate 15
//! of `generated_code_gates.rs`. A divergence fails CI.
//!
//! # The format
//!
//! Each file under `tests/fixtures/cron-conformance/` is a JSON array of cases:
//!
//! ```jsonc
//! {
//!   "name":         "berlin-a-time-the-spring-gap-skips-fires-at-the-gap-end",
//!   "cron":         "30 2 * * *",
//!   "timezone":     "Europe/Berlin",
//!   "from":         "2026-03-28T12:00:00Z",       // the instant to start after
//!   "next":         ["2026-03-29T01:00:00Z", …],  // each tick strictly after the last, UTC
//!   "scheduled_at": ["2026-03-29T03:00:00+02:00", …] // the same ticks as `payload.scheduled_at`
//! }
//! ```
//!
//! A case that must be **refused** writes `"error"` — the sentence the
//! evaluator refuses it with — instead of the instants. Unlike the CEL corpus,
//! the sentence is pinned: both columns build it from the same field grammar,
//! and the one `validate` prints is the one an author reads.
//!
//! Daylight saving is the point of the named-zone cases, in New York and
//! Berlin: a wall-clock time a gap skips fires at the gap's end, two such times
//! fire once, and a time an overlap repeats fires once, at its first
//! occurrence. The Rust column reads those zones out of the system's TZif files
//! (`support/tzif.rs`), the same IANA data `Intl` is built from.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::cron::{Cron, Utc, Zone, days_from_civil, render, render_utc};
use serde_json::Value;

#[path = "support/tzif.rs"]
mod tzif;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cron-conformance")
}

/// Every case, with the file it came from.
fn corpus() -> Vec<(String, Value)> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("the cron-conformance corpus directory exists")
        .map(|entry| entry.expect("a readable entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .expect("a file name")
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&file).expect("a readable corpus file");
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("{name} is not JSON: {error}"));
        for case in parsed.as_array().expect("a corpus file is an array") {
            cases.push((name.clone(), case.clone()));
        }
    }
    cases
}

/// `YYYY-MM-DDTHH:MM:SSZ`, in milliseconds since the epoch.
fn instant(text: &str) -> i64 {
    let number = |range: std::ops::Range<usize>| -> i64 {
        text[range]
            .parse()
            .unwrap_or_else(|_| panic!("`{text}` is not a UTC instant the corpus writes"))
    };
    assert!(
        text.ends_with('Z') && text.len() == 20,
        "`{text}` is not `…Z`"
    );
    let days = days_from_civil(
        number(0..4),
        u32::try_from(number(5..7)).expect("a month"),
        u32::try_from(number(8..10)).expect("a day"),
    );
    (days * 86_400 + number(11..13) * 3600 + number(14..16) * 60 + number(17..19)) * 1000
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .expect("a list")
        .iter()
        .map(|item| item.as_str().expect("a string").to_string())
        .collect()
}

#[test]
fn every_case_fires_where_it_declares_or_is_refused_as_it_declares() {
    let cases = corpus();
    let mut divergences = Vec::new();
    for (file, case) in &cases {
        let name = case["name"].as_str().expect("every case is named");
        let text = case["cron"].as_str().expect("every case has a `cron`");
        let parsed = Cron::parse(text);
        if let Some(expected) = case.get("error") {
            let expected = expected.as_str().expect("an error is a sentence");
            match parsed {
                Err(error) if error.message == expected => {}
                Err(error) => divergences.push(format!(
                    "{file}: {name}: refused as {:?}, expected {expected:?}",
                    error.message
                )),
                Ok(_) => divergences.push(format!("{file}: {name}: parsed, expected {expected:?}")),
            }
            continue;
        }
        let cron = match parsed {
            Ok(cron) => cron,
            Err(error) => {
                divergences.push(format!("{file}: {name}: refused as {:?}", error.message));
                continue;
            }
        };
        let timezone = case["timezone"].as_str().expect("a timezone");
        let zone: Box<dyn Zone> = if timezone == "UTC" {
            Box::new(Utc)
        } else {
            Box::new(tzif::Tzif::named(timezone))
        };
        let expected = strings(&case["next"]);
        let scheduled = strings(&case["scheduled_at"]);
        let mut at = instant(case["from"].as_str().expect("a `from`"));
        let mut next = Vec::new();
        let mut rendered = Vec::new();
        for _ in 0..expected.len() {
            let Some(fires) = cron.next_after(at, zone.as_ref()) else {
                break;
            };
            next.push(render_utc(fires));
            rendered.push(render(fires, zone.as_ref()));
            at = fires;
        }
        if next != expected {
            divergences.push(format!(
                "{file}: {name}: fired {next:?}, expected {expected:?}"
            ));
        }
        if rendered != scheduled {
            divergences.push(format!(
                "{file}: {name}: rendered {rendered:?}, expected {scheduled:?}"
            ));
        }
    }
    assert!(
        divergences.is_empty(),
        "the Rust cron evaluator disagrees with the shared corpus:\n{}",
        divergences.join("\n")
    );
}

#[test]
fn every_case_is_named_once() {
    let mut seen = std::collections::BTreeSet::new();
    for (file, case) in corpus() {
        let name = case["name"].as_str().expect("named").to_string();
        assert!(seen.insert(name.clone()), "{file}: `{name}` is named twice");
    }
}

/// The corpus covers what the ruling names: refusals, a named zone's gap and
/// its overlap, and the POSIX day rule — so a file that lost its daylight-saving
/// cases would fail here rather than pass with less to say.
#[test]
fn the_corpus_covers_refusals_both_daylight_saving_transitions_and_the_day_rule() {
    let cases = corpus();
    let named = |needle: &str| {
        cases.iter().any(|(_, case)| {
            case["name"]
                .as_str()
                .is_some_and(|name| name.contains(needle))
        })
    };
    assert!(
        cases
            .iter()
            .filter(|(_, case)| case.get("error").is_some())
            .count()
            >= 15
    );
    for needle in [
        "spring-gap",
        "autumn-repeat",
        "both-day-fields",
        "leap-day",
        "new-york",
        "berlin",
    ] {
        assert!(named(needle), "the corpus has no case about `{needle}`");
    }
    assert!(
        cases.len() >= 40,
        "the corpus is substantial, found {}",
        cases.len()
    );
}

/// The zone reader agrees with what it reads: a fall-back hour in New York is
/// an hour whose offset is first `-04:00` and then `-05:00`.
#[test]
fn the_zone_reader_knows_when_new_york_falls_back() {
    let zone = tzif::Tzif::named("America/New_York");
    let before = instant("2026-11-01T05:59:59Z") / 1000;
    let after = instant("2026-11-01T06:00:00Z") / 1000;
    assert_eq!(zone.offset_at(before), -4 * 3600);
    assert_eq!(zone.offset_at(after), -5 * 3600);
    // Past the last transition a file lists, the footer's POSIX rule answers —
    // which is all a "slim" TZif file has for any year after its rules settled.
    assert_eq!(
        zone.offset_at(instant("2040-07-01T12:00:00Z") / 1000),
        -4 * 3600
    );
    assert_eq!(
        zone.offset_at(instant("2040-01-01T12:00:00Z") / 1000),
        -5 * 3600
    );
    let berlin = tzif::Tzif::named("Europe/Berlin");
    assert_eq!(
        berlin.offset_at(instant("2045-10-29T00:59:59Z") / 1000),
        2 * 3600
    );
    assert_eq!(
        berlin.offset_at(instant("2045-10-29T01:00:00Z") / 1000),
        3600
    );
}
