//! Cron expressions: the five-field grammar a `schedule` trigger's `cron:` is
//! written in, and the evaluator that answers when one next fires (grammar
//! 13.4, PRD resolved q78, Decision D162).
//!
//! Two consumers, and the reason there are two is the reason this module is
//! written the way it is. `validate` asks [`Cron::parse`] whether an expression
//! is one the runtime can evaluate, and refuses it (`invalid-cron`) when it is
//! not; `serve` evaluates the same grammar in the emitted `src/cron.ts`. The two
//! are kept together the way CEL's two interpreters are — one shared corpus,
//! `tests/fixtures/cron-conformance/`, run against [`Cron::next_after`] here and
//! against the emitted evaluator under Node and Bun — so a field one side reads
//! differently from the other fails CI rather than a schedule. Nothing here
//! depends on a crate: the grammar and the calendar are small enough to state
//! in full, and a cron library would be a third reading of them.
//!
//! # The grammar (grammar 13.4)
//!
//! Five fields separated by ASCII whitespace — minute, hour, day of the month,
//! month, day of the week. The separators are exactly
//! [`str::split_ascii_whitespace`]'s — space, tab, line feed, form feed and
//! carriage return — and the emitted evaluator splits on the same five: Unicode
//! "whitespace" is read two ways (Rust's `White_Space` has U+0085, ECMAScript's
//! `\s` does not), and a separator either reader takes and the other refuses is
//! an expression `validate` passes and `serve` cannot evaluate. Any other
//! character is part of a field, which then refuses it. Each field is a list of
//! entries separated by `,`, and an entry is
//!
//! * `*` — every value of the field;
//! * a value — a decimal number, or, in the month and day-of-week fields only,
//!   a three-letter English name (`JAN`–`DEC`, `SUN`–`SAT`, any case);
//! * a range `a-b` with `a <= b`, both ends included;
//! * a step `*/n` or `a-b/n` — every `n`th value of the range from its start,
//!   `n >= 1`.
//!
//! Minutes run 0–59, hours 0–23, days of the month 1–31, months 1–12, days of
//! the week 0–7 with both 0 and 7 Sunday. A step over a single value (`5/15`),
//! a range that runs backwards (`22-2`), an empty list entry, a value outside
//! its field, a name in a field that takes none, and the extensions other
//! dialects carry (`?`, `L`, `W`, `#`, `@daily`) are refused: each is a field
//! this evaluator would otherwise have to guess at.
//!
//! **The day rule is POSIX's.** A field is *restricted* unless it is exactly
//! `*`. When both day fields are restricted a day matches if **either** does
//! (`0 0 13 * 5` is the 13th and every Friday); when one is, it alone decides.
//! An expression whose day-of-month field names no day any of its months has —
//! `0 0 30 2 *` — never fires, and is refused for that rather than accepted as
//! a schedule that waits for ever.
//!
//! # Time
//!
//! An expression is matched against the **wall clock** of the trigger's
//! `timezone:` ([`Zone`]), minute by minute, and fires at the instant that wall
//! clock shows a matching minute. Daylight saving is where that needs a rule,
//! and there are two:
//!
//! * a wall-clock time that **does not exist** — the hour a spring-forward
//!   skips — fires at the first instant after the gap, which is the transition
//!   itself; several such times collapse onto that one instant and fire once;
//! * a wall-clock time that **exists twice** — the hour a fall-back repeats —
//!   fires once, at its first occurrence.
//!
//! So a tick is a wall-clock minute, and the instants it fires at are
//! non-decreasing in it: [`Cron::next_after`] walks the matching minutes from
//! the one the starting instant shows, and the first whose firing instant is
//! after it is the answer.

use std::fmt;

/// A parsed, evaluable cron expression (grammar 13.4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cron {
    minutes: u64,
    hours: u64,
    days: u64,
    months: u64,
    weekdays: u64,
    days_restricted: bool,
    weekdays_restricted: bool,
}

/// Why an expression cannot be evaluated, in the words a diagnostic quotes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CronError {
    /// The whole sentence, naming the field and what is wrong with it.
    pub message: String,
}

impl fmt::Display for CronError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// What every refusal is followed by: the grammar in one sentence.
pub const GRAMMAR_HELP: &str = "a field is `*`, a number, a range `a-b`, a list `a,b`, or a step `*/n` or `a-b/n`; minutes run 0-59, hours 0-23, days of the month 1-31, months 1-12 or JAN-DEC, days of the week 0-7 or SUN-SAT with 0 and 7 both Sunday (grammar 13.4)";

/// A UTC offset source: how far a zone's wall clock is from UTC at an instant.
///
/// The emitted runtime answers this from `Intl.DateTimeFormat`, which carries
/// the IANA database; the conformance runner answers it from the TZif files a
/// system keeps under `/usr/share/zoneinfo`. Neither is a dependency of this
/// crate, which is what keeps `validate` free of a timezone database: it only
/// ever asks [`Cron::parse`].
pub trait Zone {
    /// The offset, in seconds east of UTC, in force at `instant` (seconds
    /// since the Unix epoch).
    fn offset_at(&self, instant: i64) -> i32;
}

/// The zone a `schedule` trigger with no `timezone:` is evaluated in.
#[derive(Clone, Copy, Debug, Default)]
pub struct Utc;

impl Zone for Utc {
    fn offset_at(&self, _instant: i64) -> i32 {
        0
    }
}

/// One field's bounds and spelling.
struct Field {
    name: &'static str,
    min: u32,
    max: u32,
    names: &'static [&'static str],
}

const MONTH_NAMES: &[&str] = &[
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];

const WEEKDAY_NAMES: &[&str] = &["SUN", "MON", "TUE", "WED", "THU", "FRI", "SAT"];

const FIELDS: [Field; 5] = [
    Field {
        name: "minute",
        min: 0,
        max: 59,
        names: &[],
    },
    Field {
        name: "hour",
        min: 0,
        max: 23,
        names: &[],
    },
    Field {
        name: "day-of-month",
        min: 1,
        max: 31,
        names: &[],
    },
    Field {
        name: "month",
        min: 1,
        max: 12,
        names: MONTH_NAMES,
    },
    Field {
        name: "day-of-week",
        min: 0,
        max: 7,
        names: WEEKDAY_NAMES,
    },
];

/// How many days ahead [`Cron::next_after`] looks before answering that an
/// expression has no next tick.
///
/// The longest gap a parsable expression can have is February 29th with the
/// day of the week unrestricted, and the longest run of years without one is
/// eight (2096 to 2104, since 2100 is not a leap year), so nine years of days
/// covers every expression that fires at all.
pub const HORIZON_DAYS: i64 = 366 * 9;

impl Cron {
    /// Parse `text` as grammar 13.4's five-field expression, refusing what the
    /// runtime could not evaluate.
    ///
    /// # Errors
    ///
    /// A [`CronError`] naming the first field that is wrong and how, or saying
    /// that the expression can never fire.
    pub fn parse(text: &str) -> Result<Self, CronError> {
        let parts: Vec<&str> = text.split_ascii_whitespace().collect();
        if parts.len() != 5 {
            return Err(CronError {
                message: format!(
                    "a cron expression has 5 fields, and this one has {}",
                    parts.len()
                ),
            });
        }
        let mut sets = [0_u64; 5];
        for (index, (part, field)) in parts.iter().zip(FIELDS.iter()).enumerate() {
            sets[index] = parse_field(part, field).map_err(|problem| CronError {
                message: format!("the {} field `{part}` {problem}", field.name),
            })?;
        }
        // Sunday is both 0 and 7; the evaluator reads 0.
        let weekdays = (sets[4] | (sets[4] >> 7)) & 0x7f;
        let cron = Self {
            minutes: sets[0],
            hours: sets[1],
            days: sets[2],
            months: sets[3],
            weekdays,
            days_restricted: parts[2] != "*",
            weekdays_restricted: parts[4] != "*",
        };
        if cron.days_restricted && !cron.weekdays_restricted && !cron.some_month_has_a_day() {
            return Err(CronError {
                message: format!(
                    "never fires: the day-of-month field `{}` names no day that a month in the month field `{}` has",
                    parts[2], parts[3]
                ),
            });
        }
        Ok(cron)
    }

    /// Whether some month this expression names has some day it names.
    fn some_month_has_a_day(&self) -> bool {
        (1..=12).any(|month| {
            self.months & (1 << month) != 0
                && (1..=longest_month(month)).any(|day| self.days & (1 << day) != 0)
        })
    }

    /// Whether the civil day `day` (days since the Unix epoch) matches the
    /// month and the two day fields, under the POSIX day rule.
    fn day_matches(&self, day: i64) -> bool {
        let (_, month, date) = civil_from_days(day);
        if self.months & (1 << month) == 0 {
            return false;
        }
        let weekday = (day + 4).rem_euclid(7);
        let by_date = self.days & (1 << date) != 0;
        let by_weekday = self.weekdays & (1 << weekday) != 0;
        match (self.days_restricted, self.weekdays_restricted) {
            (true, true) => by_date || by_weekday,
            (true, false) => by_date,
            (false, true) => by_weekday,
            (false, false) => true,
        }
    }

    /// The first instant **strictly after** `from` (milliseconds since the
    /// Unix epoch) at which this expression fires in `zone`, or `None` when it
    /// has no tick inside [`HORIZON_DAYS`].
    #[must_use]
    pub fn next_after(&self, from: i64, zone: &dyn Zone) -> Option<i64> {
        let from_seconds = from.div_euclid(1000);
        let local = from_seconds + i64::from(zone.offset_at(from_seconds));
        let start_minute = local.div_euclid(60);
        let start_day = start_minute.div_euclid(1440);
        let start_of_day = start_minute.rem_euclid(1440);
        for day in start_day..start_day + HORIZON_DAYS {
            if !self.day_matches(day) {
                continue;
            }
            let (first_hour, first_minute) = if day == start_day {
                (start_of_day / 60, start_of_day % 60)
            } else {
                (0, 0)
            };
            for hour in first_hour..24 {
                if self.hours & (1 << hour) == 0 {
                    continue;
                }
                let from_minute = if day == start_day && hour == first_hour {
                    first_minute
                } else {
                    0
                };
                for minute in from_minute..60 {
                    if self.minutes & (1 << minute) == 0 {
                        continue;
                    }
                    let wall = (day * 1440 + hour * 60 + minute) * 60;
                    let fires = fire_instant(wall, zone) * 1000;
                    if fires > from {
                        return Some(fires);
                    }
                }
            }
        }
        None
    }
}

/// The longest a month can be, February's leap day included.
const fn longest_month(month: u32) -> u32 {
    match month {
        2 => 29,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// One field's set of values, as bits.
fn parse_field(text: &str, field: &Field) -> Result<u64, String> {
    let mut set = 0_u64;
    for entry in text.split(',') {
        if entry.is_empty() {
            return Err("has an empty entry in its list".to_string());
        }
        let (range, step) = match entry.split_once('/') {
            Some((range, step)) => (range, Some(step)),
            None => (entry, None),
        };
        let step = match step {
            None => 1,
            Some(step) => {
                let n = number(step)
                    .ok_or_else(|| format!("steps by `{step}`, which is not a whole number"))?;
                if n == 0 {
                    return Err("steps by 0, and a step is at least 1".to_string());
                }
                n
            }
        };
        let (low, high) = if range == "*" {
            (field.min, field.max)
        } else if let Some((low, high)) = range.split_once('-') {
            let low = value(low, field)?;
            let high = value(high, field)?;
            if low > high {
                return Err(format!(
                    "has the range `{range}`, which runs backwards: write the smaller end first"
                ));
            }
            (low, high)
        } else {
            let single = value(range, field)?;
            if entry.contains('/') {
                return Err(format!(
                    "steps the single value `{range}`: a step needs a range, so write `{range}-{}/{step}` or `*/{step}`",
                    field.max
                ));
            }
            (single, single)
        };
        let mut at = low;
        while at <= high {
            set |= 1 << at;
            at = match at.checked_add(step) {
                Some(next) => next,
                None => break,
            };
        }
    }
    Ok(set)
}

/// A decimal number of at most four digits, or `None`.
///
/// Four is generous — no field reaches 100 — and bounding it keeps the
/// arithmetic out of overflow; a longer run of digits is out of range anyway.
fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.len() > 4 {
        return Some(u32::MAX);
    }
    text.parse().ok()
}

/// One value of a field: a number inside its bounds, or one of its names.
fn value(text: &str, field: &Field) -> Result<u32, String> {
    if text.is_empty() {
        return Err("is missing a value on one side of a `-`".to_string());
    }
    if let Some(n) = number(text) {
        if n < field.min || n > field.max {
            let shown = if n == u32::MAX {
                text.to_string()
            } else {
                n.to_string()
            };
            return Err(format!(
                "names {shown}, outside {}-{}",
                field.min, field.max
            ));
        }
        return Ok(n);
    }
    if text.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        if field.names.is_empty() {
            return Err(format!(
                "names `{text}`, and only the month and day-of-week fields take names"
            ));
        }
        let upper = text.to_ascii_uppercase();
        if let Some(index) = field.names.iter().position(|name| *name == upper) {
            let index = u32::try_from(index).expect("a name table is short");
            return Ok(field.min + index);
        }
        return Err(format!(
            "names `{text}`, which is not one of {}",
            field.names.join(", ")
        ));
    }
    Err(format!(
        "holds `{text}`, which is not a number, a name, `*`, a range or a step"
    ))
}

/// The instant (seconds) a wall-clock minute `wall` (seconds of local time
/// since the epoch) fires at in `zone`.
///
/// The earliest instant whose local time is `wall`, which is the first
/// occurrence of a time a fall-back repeats; and, for a time a spring-forward
/// skips, the transition that skipped it — the first instant after the gap.
///
/// The candidate offsets are the zone's a day either side of the wall time
/// and at it, read as instants: every offset in use is within fourteen hours
/// of UTC, so the instant a day before `wall` precedes any transition the
/// minute could fall into, and the one a day after follows it. The emitted
/// evaluator asks exactly these three, in this order.
fn fire_instant(wall: i64, zone: &dyn Zone) -> i64 {
    let candidates = [
        zone.offset_at(wall - 86_400),
        zone.offset_at(wall),
        zone.offset_at(wall + 86_400),
    ];
    let mut earliest: Option<i64> = None;
    for offset in candidates {
        let instant = wall - i64::from(offset);
        if zone.offset_at(instant) == offset {
            earliest = Some(earliest.map_or(instant, |held| held.min(instant)));
        }
    }
    if let Some(instant) = earliest {
        return instant;
    }
    // A gap: the clock jumped from the smaller offset to the larger one, so
    // read with the larger one the wall time is before the transition and read
    // with the smaller one it is after. The transition is the first second in
    // between whose offset is no longer the earlier one.
    let smaller = candidates.iter().copied().min().unwrap_or(0);
    let larger = candidates.iter().copied().max().unwrap_or(0);
    let mut before = wall - i64::from(larger);
    let mut after = wall - i64::from(smaller);
    let held = zone.offset_at(before);
    while after - before > 1 {
        let middle = before + (after - before) / 2;
        if zone.offset_at(middle) == held {
            before = middle;
        } else {
            after = middle;
        }
    }
    after
}

/// An instant (milliseconds) as RFC 3339 in `zone`: the zone's wall clock and
/// its numeric offset, `+00:00` for UTC — the spelling `payload.scheduled_at`
/// carries (grammar 13.4).
#[must_use]
pub fn render(instant: i64, zone: &dyn Zone) -> String {
    let seconds = instant.div_euclid(1000);
    let offset = zone.offset_at(seconds);
    let local = seconds + i64::from(offset);
    let (year, month, day) = civil_from_days(local.div_euclid(86_400));
    let of_day = local.rem_euclid(86_400);
    let sign = if offset < 0 { '-' } else { '+' };
    let magnitude = offset.unsigned_abs();
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}{sign}{:02}:{:02}",
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60,
        magnitude / 3600,
        (magnitude % 3600) / 60
    )
}

/// An instant (milliseconds) as RFC 3339 in UTC, `Z`-suffixed, to the second.
#[must_use]
pub fn render_utc(instant: i64) -> String {
    let seconds = instant.div_euclid(1000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let of_day = seconds.rem_euclid(86_400);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        (of_day % 3600) / 60,
        of_day % 60
    )
}

/// The civil date of a day count since the Unix epoch, proleptic Gregorian
/// (Howard Hinnant's `civil_from_days`).
#[must_use]
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u32::try_from(month).expect("a month is 1-12"),
        u32::try_from(day).expect("a day is 1-31"),
    )
}

/// The day count since the Unix epoch of a civil date (Howard Hinnant's
/// `days_from_civil`).
#[must_use]
pub fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let mp = if month > 2 { month - 3 } else { month + 9 };
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(year: i64, month: u32, day: u32, hour: i64, minute: i64) -> i64 {
        (days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60) * 1000
    }

    #[test]
    fn the_calendar_round_trips() {
        for days in [-800_000, -1, 0, 1, 19_000, 20_000, 2_000_000] {
            let (year, month, day) = civil_from_days(days);
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    }

    #[test]
    fn a_daily_expression_fires_at_its_time_and_strictly_after_the_start() {
        let cron = Cron::parse("0 3 * * *").expect("parses");
        let first = cron.next_after(at(2026, 11, 4, 1, 0), &Utc);
        assert_eq!(first, Some(at(2026, 11, 4, 3, 0)));
        let second = cron.next_after(at(2026, 11, 4, 3, 0), &Utc);
        assert_eq!(second, Some(at(2026, 11, 5, 3, 0)));
    }

    #[test]
    fn both_restricted_day_fields_match_either() {
        let cron = Cron::parse("0 0 13 * 5").expect("parses");
        // 2026-11-04 is a Wednesday: the next Friday is the 6th, before the 13th.
        assert_eq!(
            cron.next_after(at(2026, 11, 4, 0, 0), &Utc),
            Some(at(2026, 11, 6, 0, 0))
        );
    }

    #[test]
    fn sunday_is_both_zero_and_seven() {
        let zero = Cron::parse("0 0 * * 0").expect("parses");
        let seven = Cron::parse("0 0 * * 7").expect("parses");
        assert_eq!(zero, seven);
    }

    #[test]
    fn names_are_read_in_any_case_where_a_field_takes_them() {
        assert_eq!(
            Cron::parse("0 9 * jan-Mar MON-fri"),
            Cron::parse("0 9 * 1-3 1-5")
        );
        let refused = Cron::parse("MON * * * *").expect_err("a minute takes no names");
        assert!(refused.message.contains("only the month and day-of-week"));
    }

    #[test]
    fn an_expression_no_month_can_hold_is_refused() {
        let refused = Cron::parse("0 0 30 2 *").expect_err("February has no 30th");
        assert!(refused.message.starts_with("never fires"));
        // A leap day is a day February has.
        assert!(Cron::parse("0 0 29 2 *").is_ok());
        // With the day of the week restricted the POSIX rule makes it fire.
        assert!(Cron::parse("0 0 30 2 1").is_ok());
    }

    #[test]
    fn every_field_the_evaluator_could_not_read_is_refused() {
        for (text, problem) in [
            ("60 * * * *", "names 60, outside 0-59"),
            ("* 24 * * *", "names 24, outside 0-23"),
            ("* * 0 * *", "names 0, outside 1-31"),
            ("* * * 13 *", "names 13, outside 1-12"),
            ("* * * * 8", "names 8, outside 0-7"),
            ("5/15 * * * *", "steps the single value `5`"),
            ("*/0 * * * *", "steps by 0"),
            ("10-2 * * * *", "runs backwards"),
            ("1,,2 * * * *", "empty entry"),
            ("? * * * *", "not a number"),
            ("L * * * *", "only the month and day-of-week"),
            ("* * * * MOO", "not one of SUN"),
            ("*/x * * * *", "not a whole number"),
            ("-5 * * * *", "missing a value"),
            ("99999 * * * *", "names 99999, outside 0-59"),
        ] {
            let refused = Cron::parse(text).expect_err(text);
            assert!(
                refused.message.contains(problem),
                "{text}: {}",
                refused.message
            );
        }
        assert!(Cron::parse("0 3 * *").is_err());
    }

    #[test]
    fn a_leap_day_is_found_eight_years_out() {
        let cron = Cron::parse("0 0 29 2 *").expect("parses");
        assert_eq!(
            cron.next_after(at(2096, 3, 1, 0, 0), &Utc),
            Some(at(2104, 2, 29, 0, 0))
        );
    }

    #[test]
    fn rendering_writes_the_zone_offset() {
        assert_eq!(
            render(at(2026, 11, 4, 3, 0), &Utc),
            "2026-11-04T03:00:00+00:00"
        );
        assert_eq!(render_utc(at(2026, 11, 4, 3, 0)), "2026-11-04T03:00:00Z");
    }
}
