//! A timezone read out of the system's TZif files, for the cron conformance
//! corpus's Rust column (grammar 13.4, PRD resolved q78).
//!
//! The compiler itself carries no timezone database and needs none: `validate`
//! only asks whether a `cron:` parses ([`compose_core::cron::Cron::parse`]), and
//! the instants a schedule fires at are `serve`'s to compute, through `Intl`.
//! The corpus holds both evaluators to one reading of a zone's wall clock, so
//! the Rust column needs *a* source of offsets that is not the JavaScript
//! engine's — and the IANA database every Unix system keeps under
//! `/usr/share/zoneinfo` is the same data `Intl` is built from. This reads it:
//! RFC 8536's version-2 block (64-bit transition times), and for an instant past
//! the last transition, the POSIX TZ string in the footer, which is what a
//! "slim" file relies on for every year after its rules settled.
//!
//! A zone that cannot be read is a failed test rather than a skipped one: a
//! corpus that quietly stopped checking its daylight-saving cases would be the
//! divergence it exists to catch, unchecked.

use std::path::Path;

use compose_core::cron::{Zone, days_from_civil};

/// One zone: its transitions, and the rule after them.
pub struct Tzif {
    /// `(instant, offset)` — the offset in force from each transition on.
    transitions: Vec<(i64, i32)>,
    /// The offset before the first transition.
    initial: i32,
    /// The footer's rule, for instants past the last transition.
    rule: Option<Rule>,
}

/// A POSIX TZ string's rule: a standard offset and, where the zone observes
/// daylight saving, the daylight offset and when it starts and ends.
struct Rule {
    standard: i32,
    daylight: Option<(i32, Change, Change)>,
}

/// `Mm.w.d/time`: the `w`th `d`ay of month `m` (week 5 meaning the last), at
/// `time` seconds of local time.
#[derive(Clone, Copy)]
struct Change {
    month: u32,
    week: u32,
    weekday: u32,
    time: i64,
}

impl Tzif {
    /// Read `name` from `/usr/share/zoneinfo`.
    ///
    /// # Panics
    ///
    /// When the file is absent or is not a TZif file this reader understands.
    pub fn named(name: &str) -> Self {
        let path = Path::new("/usr/share/zoneinfo").join(name);
        let bytes = std::fs::read(&path).unwrap_or_else(|error| {
            panic!(
                "the cron corpus names `{name}`, and {} could not be read ({error}): install the system's tzdata",
                path.display()
            )
        });
        Self::parse(&bytes).unwrap_or_else(|| panic!("{} is not a TZif file", path.display()))
    }

    fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.get(..4)? != b"TZif" {
            return None;
        }
        let version = *bytes.get(4)?;
        let (first, after) = block(bytes, 4)?;
        if version < b'2' {
            return Some(first.into_zone(None));
        }
        let rest = bytes.get(after..)?;
        let (second, end) = block(rest, 8)?;
        let footer = rest.get(end..)?;
        let text = std::str::from_utf8(footer).ok()?;
        let rule = text.trim_matches('\n').split('\n').next().and_then(rule);
        Some(second.into_zone(rule))
    }
}

/// One header-and-data block, with transition times `width` bytes wide.
struct Block {
    times: Vec<i64>,
    indices: Vec<u8>,
    offsets: Vec<i32>,
}

impl Block {
    fn into_zone(self, rule: Option<Rule>) -> Tzif {
        let transitions = self
            .times
            .iter()
            .zip(&self.indices)
            .map(|(time, index)| (*time, self.offsets[usize::from(*index)]))
            .collect();
        Tzif {
            transitions,
            initial: self.offsets.first().copied().unwrap_or(0),
            rule,
        }
    }
}

fn be32(bytes: &[u8], at: usize) -> Option<i64> {
    let slice: [u8; 4] = bytes.get(at..at + 4)?.try_into().ok()?;
    Some(i64::from(i32::from_be_bytes(slice)))
}

fn be64(bytes: &[u8], at: usize) -> Option<i64> {
    let slice: [u8; 8] = bytes.get(at..at + 8)?.try_into().ok()?;
    Some(i64::from_be_bytes(slice))
}

/// Read the block at the start of `bytes`, and where it ends.
fn block(bytes: &[u8], width: usize) -> Option<(Block, usize)> {
    let count =
        |index: usize| -> Option<usize> { usize::try_from(be32(bytes, 20 + index * 4)?).ok() };
    let (isut, isstd, leap, time, kinds, chars) = (
        count(0)?,
        count(1)?,
        count(2)?,
        count(3)?,
        count(4)?,
        count(5)?,
    );
    let mut at = 44;
    let mut times = Vec::with_capacity(time);
    for _ in 0..time {
        times.push(if width == 8 {
            be64(bytes, at)?
        } else {
            be32(bytes, at)?
        });
        at += width;
    }
    let indices = bytes.get(at..at + time)?.to_vec();
    at += time;
    let mut offsets = Vec::with_capacity(kinds);
    for _ in 0..kinds {
        offsets.push(i32::try_from(be32(bytes, at)?).ok()?);
        at += 6;
    }
    at += chars + leap * (width + 4) + isstd + isut;
    Some((
        Block {
            times,
            indices,
            offsets,
        },
        at,
    ))
}

/// Parse a POSIX TZ string such as `EST5EDT,M3.2.0,M11.1.0`.
fn rule(text: &str) -> Option<Rule> {
    let mut rest = text;
    rest = skip_name(rest)?;
    let (standard, after) = offset(rest)?;
    rest = after;
    // POSIX writes hours *west* of UTC; an offset here is seconds east.
    let standard = -standard;
    if rest.is_empty() {
        return Some(Rule {
            standard,
            daylight: None,
        });
    }
    rest = skip_name(rest)?;
    let mut daylight = standard + 3600;
    if !rest.starts_with(',') {
        let (written, after) = offset(rest)?;
        daylight = -written;
        rest = after;
    }
    let rest = rest.strip_prefix(',')?;
    let (start, end) = rest.split_once(',')?;
    Some(Rule {
        standard,
        daylight: Some((daylight, change(start)?, change(end)?)),
    })
}

fn skip_name(text: &str) -> Option<&str> {
    if let Some(rest) = text.strip_prefix('<') {
        let close = rest.find('>')?;
        return Some(&rest[close + 1..]);
    }
    let end = text
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(text.len());
    if end < 3 {
        return None;
    }
    Some(&text[end..])
}

/// `[+-]hh[:mm[:ss]]`, in seconds, and what follows it.
fn offset(text: &str) -> Option<(i32, &str)> {
    let (sign, digits) = match text.as_bytes().first()? {
        b'-' => (-1, &text[1..]),
        b'+' => (1, &text[1..]),
        _ => (1, text),
    };
    let end = digits
        .find(|c: char| !(c.is_ascii_digit() || c == ':'))
        .unwrap_or(digits.len());
    let mut seconds = 0;
    for (index, part) in digits[..end].split(':').enumerate() {
        let value: i32 = part.parse().ok()?;
        seconds += value * [3600, 60, 1][index.min(2)];
    }
    Some((sign * seconds, &digits[end..]))
}

/// `Mm.w.d[/time]`.
fn change(text: &str) -> Option<Change> {
    let text = text.strip_prefix('M')?;
    let (date, time) = text
        .split_once('/')
        .map_or((text, None), |(d, t)| (d, Some(t)));
    let mut parts = date.split('.');
    let month = parts.next()?.parse().ok()?;
    let week = parts.next()?.parse().ok()?;
    let weekday = parts.next()?.parse().ok()?;
    let time = match time {
        None => 7200,
        Some(written) => i64::from(offset(written)?.0),
    };
    Some(Change {
        month,
        week,
        weekday,
        time,
    })
}

/// The day count of the `week`th `weekday` of `month` in `year`.
fn nth_weekday(year: i64, change: Change) -> i64 {
    let first = days_from_civil(year, change.month, 1);
    let first_weekday = (first + 4).rem_euclid(7);
    let mut day = first + (i64::from(change.weekday) - first_weekday).rem_euclid(7);
    day += 7 * (i64::from(change.week) - 1);
    let next_month = if change.month == 12 {
        days_from_civil(year + 1, 1, 1)
    } else {
        days_from_civil(year, change.month + 1, 1)
    };
    while day >= next_month {
        day -= 7;
    }
    day
}

impl Rule {
    fn offset_at(&self, instant: i64) -> i32 {
        let Some((daylight, start, end)) = self.daylight else {
            return self.standard;
        };
        let year = compose_core::cron::civil_from_days(instant.div_euclid(86_400)).0;
        let starts = nth_weekday(year, start) * 86_400 + start.time - i64::from(self.standard);
        let ends = nth_weekday(year, end) * 86_400 + end.time - i64::from(daylight);
        let inside = if starts < ends {
            starts <= instant && instant < ends
        } else {
            !(ends <= instant && instant < starts)
        };
        if inside { daylight } else { self.standard }
    }
}

impl Zone for Tzif {
    fn offset_at(&self, instant: i64) -> i32 {
        match self.transitions.last() {
            Some(&(last, _)) if instant >= last => {
                if let Some(rule) = &self.rule {
                    return rule.offset_at(instant);
                }
            }
            None => {
                if let Some(rule) = &self.rule {
                    return rule.offset_at(instant);
                }
                return self.initial;
            }
            _ => {}
        }
        let index = self
            .transitions
            .partition_point(|&(time, _)| time <= instant);
        if index == 0 {
            self.initial
        } else {
            self.transitions[index - 1].1
        }
    }
}
