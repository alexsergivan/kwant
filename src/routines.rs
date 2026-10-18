//! Routine schedules: validation and next-run maths. Pure, so the scheduler and the API share it
//! and tests can pin any instant, including daylight-saving changes.

use chrono::{DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveTime, Utc, Weekday};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Shortest interval a routine may repeat at.
pub const MIN_INTERVAL_MINUTES: i64 = 5;
/// Longest interval (one week), so a typo can't park a routine for years.
pub const MAX_INTERVAL_MINUTES: i64 = 7 * 24 * 60;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Schedule {
    /// Runs once at `at` (RFC 3339), then the routine disables itself.
    Once { at: DateTime<Utc> },
    /// Every day at `time` (`HH:MM`) in `tz`.
    Daily { time: String, tz: String },
    /// On `weekdays` (`mon`..`sun`) at `time` in `tz`.
    Weekly {
        time: String,
        weekdays: Vec<String>,
        tz: String,
    },
    /// Every `every_minutes`, counted from the last run (or from when it was scheduled).
    Interval { every_minutes: i64 },
    /// On `days` of each month at `time` in `tz`. A day the month lacks (31 in April) runs on
    /// its last day; `"last"` is always the last day.
    Monthly {
        time: String,
        days: Vec<MonthDay>,
        tz: String,
    },
    /// A five-field cron `expression` (minute hour day-of-month month day-of-week) in `tz`.
    Cron { expression: String, tz: String },
}

/// A day of the month for [`Schedule::Monthly`]: `1`..`31`, or `"last"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonthDay {
    Day(u32),
    Last,
}

impl Serialize for MonthDay {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            MonthDay::Day(d) => s.serialize_u32(*d),
            MonthDay::Last => s.serialize_str("last"),
        }
    }
}

impl<'de> Deserialize<'de> for MonthDay {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let bad = || D::Error::custom("days are 1-31 or \"last\"");
        match serde_json::Value::deserialize(d)? {
            serde_json::Value::Number(n) => n
                .as_u64()
                .filter(|n| (1..=31).contains(n))
                .map(|n| MonthDay::Day(n as u32))
                .ok_or_else(bad),
            serde_json::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "last" | "l" | "-1" => Ok(MonthDay::Last),
                other => other
                    .parse::<u32>()
                    .ok()
                    .filter(|n| (1..=31).contains(n))
                    .map(MonthDay::Day)
                    .ok_or_else(bad),
            },
            _ => Err(bad()),
        }
    }
}

fn last_day_of_month(date: NaiveDate) -> u32 {
    let (y, m) = (date.year(), date.month());
    let first_next = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    first_next.and_then(|d| d.pred_opt()).map_or(28, |d| d.day())
}

impl MonthDay {
    /// Whether this is `date`'s day, with days past the month's end on its last day.
    fn matches(self, date: NaiveDate) -> bool {
        let last = last_day_of_month(date);
        match self {
            MonthDay::Last => date.day() == last,
            MonthDay::Day(d) => date.day() == d.min(last),
        }
    }
}

fn parse_time(time: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(time.trim(), "%H:%M")
        .map_err(|_| "time must be HH:MM (24-hour)".to_string())
}

/// Where the system tz database lives (Linux, macOS, BSDs).
const ZONEINFO_DIRS: &[&str] = &["/usr/share/zoneinfo", "/usr/lib/zoneinfo", "/etc/zoneinfo"];

/// An IANA time zone read from the system tz database. Reading it per use (rather than
/// bundling the database) keeps ~3 MB of relocated tables out of the binary and idle memory.
struct Zone(tz::TimeZone);

impl Zone {
    /// The UTC offset in force at `unix` seconds.
    fn offset_at(&self, unix: i64) -> Option<i64> {
        let local = self.0.find_local_time_type(unix).ok()?;
        Some(i64::from(local.ut_offset()))
    }

    fn date_at(&self, t: DateTime<Utc>) -> Option<NaiveDate> {
        let offset = self.offset_at(t.timestamp())?;
        Some((t.naive_utc() + Duration::seconds(offset)).date())
    }

    /// Every instant that reads as `local` on a wall clock here, earliest first: none in a
    /// spring-forward gap, two in a fall-back overlap.
    fn instants(&self, local: chrono::NaiveDateTime) -> LocalResult<DateTime<Utc>> {
        let wall = local.and_utc().timestamp();
        // Offsets are under a day and change at most once around any wall time
        let mut found: Vec<i64> = [wall - 86_400, wall + 86_400]
            .into_iter()
            .filter_map(|t| self.offset_at(t))
            .map(|offset| wall - offset)
            .filter(|&t| self.offset_at(t) == Some(wall - t))
            .collect();
        found.sort_unstable();
        found.dedup();
        let at = |t: i64| DateTime::<Utc>::from_timestamp(t, 0);
        match found[..] {
            [] => LocalResult::None,
            [t] => at(t).map_or(LocalResult::None, LocalResult::Single),
            [a, .., b] => match (at(a), at(b)) {
                (Some(a), Some(b)) => LocalResult::Ambiguous(a, b),
                _ => LocalResult::None,
            },
        }
    }
}

/// Old IANA names (the tz `backward` links) that browsers' ICU still reports but some systems
/// (e.g. Ubuntu 24.04 without tzdata-legacy) no longer ship. Mirrors frontend/src/continuity.ts.
const LEGACY_TZ: &[(&str, &str)] = &[
    ("Africa/Asmera", "Africa/Asmara"),
    ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
    ("America/Catamarca", "America/Argentina/Catamarca"),
    ("America/Cordoba", "America/Argentina/Cordoba"),
    ("America/Godthab", "America/Nuuk"),
    ("America/Indianapolis", "America/Indiana/Indianapolis"),
    ("America/Jujuy", "America/Argentina/Jujuy"),
    ("America/Louisville", "America/Kentucky/Louisville"),
    ("America/Mendoza", "America/Argentina/Mendoza"),
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("Asia/Saigon", "Asia/Ho_Chi_Minh"),
    ("Atlantic/Faeroe", "Atlantic/Faroe"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("Pacific/Enderbury", "Pacific/Kanton"),
    ("Pacific/Ponape", "Pacific/Pohnpei"),
    ("Pacific/Truk", "Pacific/Chuuk"),
];

/// The current IANA name for `tz`, trimmed; unknown names pass through for `parse_tz` to judge.
pub fn canonical_tz(tz: &str) -> String {
    let name = tz.trim();
    LEGACY_TZ
        .iter()
        .find(|(old, _)| *old == name)
        .map_or(name, |(_, new)| new)
        .to_string()
}

fn parse_tz(tz: &str) -> Result<Zone, String> {
    let canonical = canonical_tz(tz);
    let name = canonical.as_str();
    let unknown = || format!("unknown time zone '{tz}'; use an IANA name like Europe/Kyiv");
    if name == "UTC" {
        return Ok(Zone(tz::TimeZone::utc()));
    }
    // A name, never a path: no leading '/', no '..', only the characters IANA names use
    let well_formed = !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c));
    if !well_formed {
        return Err(unknown());
    }
    ZONEINFO_DIRS
        .iter()
        .find_map(|dir| std::fs::read(Path::new(dir).join(name)).ok())
        .and_then(|bytes| tz::TimeZone::from_tz_data(&bytes).ok())
        .map(Zone)
        .ok_or_else(unknown)
}

fn parse_weekday(day: &str) -> Result<Weekday, String> {
    match day.trim().to_ascii_lowercase().as_str() {
        "mon" => Ok(Weekday::Mon),
        "tue" => Ok(Weekday::Tue),
        "wed" => Ok(Weekday::Wed),
        "thu" => Ok(Weekday::Thu),
        "fri" => Ok(Weekday::Fri),
        "sat" => Ok(Weekday::Sat),
        "sun" => Ok(Weekday::Sun),
        _ => Err(format!(
            "unknown weekday '{day}'; use mon, tue, wed, thu, fri, sat or sun"
        )),
    }
}

impl Schedule {
    /// The same schedule with its time zone under its current IANA name, as it should be stored.
    pub fn canonical(self) -> Self {
        match self {
            Schedule::Daily { time, tz } => Schedule::Daily {
                time,
                tz: canonical_tz(&tz),
            },
            Schedule::Weekly { time, weekdays, tz } => Schedule::Weekly {
                time,
                weekdays,
                tz: canonical_tz(&tz),
            },
            Schedule::Monthly { time, days, tz } => Schedule::Monthly {
                time,
                days,
                tz: canonical_tz(&tz),
            },
            Schedule::Cron { expression, tz } => Schedule::Cron {
                expression: expression.split_whitespace().collect::<Vec<_>>().join(" "),
                tz: canonical_tz(&tz),
            },
            other => other,
        }
    }

    /// Checks every field; the API returns the message as a 400.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Schedule::Once { .. } => Ok(()),
            Schedule::Daily { time, tz } => {
                parse_time(time)?;
                parse_tz(tz).map(|_| ())
            }
            Schedule::Weekly { time, weekdays, tz } => {
                parse_time(time)?;
                parse_tz(tz)?;
                if weekdays.is_empty() {
                    return Err("weekly schedules need at least one weekday".into());
                }
                weekdays
                    .iter()
                    .try_for_each(|d| parse_weekday(d).map(|_| ()))
            }
            Schedule::Interval { every_minutes } => {
                if (MIN_INTERVAL_MINUTES..=MAX_INTERVAL_MINUTES).contains(every_minutes) {
                    Ok(())
                } else {
                    Err(format!("every_minutes must be between {MIN_INTERVAL_MINUTES} and {MAX_INTERVAL_MINUTES}"))
                }
            }
            Schedule::Monthly { time, days, tz } => {
                parse_time(time)?;
                parse_tz(tz)?;
                if days.is_empty() {
                    return Err("monthly schedules need at least one day (1-31 or \"last\")".into());
                }
                Ok(())
            }
            Schedule::Cron { expression, tz } => {
                parse_tz(tz)?;
                Cron::parse(expression)?;
                // e.g. "0 9 30 2 *": valid fields, but February never has a 30th
                if self.next_after(Utc::now(), None).is_none() {
                    return Err(format!("the cron expression '{expression}' never runs"));
                }
                Ok(())
            }
        }
    }

    /// The first run strictly after `after`. `last_run` anchors intervals. `None` when the
    /// schedule will never run again (a `once` in the past) or is invalid.
    pub fn next_after(
        &self,
        after: DateTime<Utc>,
        last_run: Option<DateTime<Utc>>,
    ) -> Option<DateTime<Utc>> {
        match self {
            Schedule::Once { at } => (*at > after).then_some(*at),
            Schedule::Interval { every_minutes } => {
                let step = Duration::minutes(*every_minutes);
                match last_run {
                    // Missed runs are not replayed: never schedule in the past
                    Some(last) if last + step > after => Some(last + step),
                    _ => Some(after + step),
                }
            }
            Schedule::Daily { time, tz } => {
                let (time, tz) = (parse_time(time).ok()?, parse_tz(tz).ok()?);
                next_local(after, tz, time, |_| true)
            }
            Schedule::Weekly { time, weekdays, tz } => {
                let (time, tz) = (parse_time(time).ok()?, parse_tz(tz).ok()?);
                let days: Vec<Weekday> = weekdays
                    .iter()
                    .filter_map(|d| parse_weekday(d).ok())
                    .collect();
                if days.is_empty() {
                    return None;
                }
                next_local(after, tz, time, |d| days.contains(&d.weekday()))
            }
            Schedule::Monthly { time, days, tz } => {
                let (time, tz) = (parse_time(time).ok()?, parse_tz(tz).ok()?);
                if days.is_empty() {
                    return None;
                }
                next_local_within(after, &tz, 62, |d| {
                    days.iter().any(|m| m.matches(d)).then_some(vec![time])
                })
            }
            Schedule::Cron { expression, tz } => {
                let (cron, tz) = (Cron::parse(expression).ok()?, parse_tz(tz).ok()?);
                // Long enough for "29 Feb" across a skipped leap year (2096 to 2104)
                next_local_within(after, &tz, 366 * 8 + 2, |d| {
                    cron.day_matches(d).then(|| cron.times())
                })
            }
        }
    }
}

/// A parsed five-field cron expression. Minute, hour, day of month (also `L` for the last
/// day), month (also `jan`..`dec`) and day of week (0-7 with 0 and 7 Sunday, also `mon`..`sun`,
/// `fri#2` for the second Friday, `5L` for the last Friday). Fields take `*`, lists, ranges and
/// steps. When both day fields are restricted a day matching either runs, as in classic cron.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cron {
    minutes: Vec<u32>,
    hours: Vec<u32>,
    dom: Vec<u32>,
    dom_last: bool,
    dom_any: bool,
    months: Vec<u32>,
    dow: Vec<u32>,
    dow_nth: Vec<(u32, u32)>,
    dow_last: Vec<u32>,
    dow_any: bool,
}

const MONTH_NAMES: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];
const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// One cron field as sorted values in `min..=max`; `names` map to `min + index`.
fn cron_field(field: &str, what: &str, min: u32, max: u32, names: &[&str]) -> Result<Vec<u32>, String> {
    let bad = || format!("cron {what} field '{field}' is not valid; use numbers {min}-{max}, *, lists, ranges or steps");
    let value = |v: &str| -> Result<u32, String> {
        let v = v.to_ascii_lowercase();
        if let Some(i) = names.iter().position(|n| *n == v) {
            return Ok(min + i as u32);
        }
        v.parse::<u32>().ok().filter(|n| (min..=max).contains(n)).ok_or_else(bad)
    };
    let mut out = Vec::new();
    for part in field.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((r, s)) => (r, s.parse::<u32>().ok().filter(|s| *s > 0).ok_or_else(bad)?),
            None => (part, 1),
        };
        let (lo, hi) = match range {
            "*" | "?" => (min, max),
            r => match r.split_once('-') {
                Some((a, b)) => (value(a)?, value(b)?),
                // "5/15" means from 5 to the end in steps of 15
                None if part.contains('/') => (value(r)?, max),
                None => {
                    let v = value(r)?;
                    (v, v)
                }
            },
        };
        if lo > hi {
            return Err(bad());
        }
        out.extend((lo..=hi).step_by(step as usize));
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

impl Cron {
    fn parse(expression: &str) -> Result<Self, String> {
        let fields: Vec<&str> = expression.split_whitespace().collect();
        let [minute, hour, dom, month, dow] = fields[..] else {
            return Err(format!(
                "cron expression '{expression}' needs exactly five fields: minute hour day-of-month month day-of-week, e.g. \"0 9 1 * *\""
            ));
        };
        let minutes = cron_field(minute, "minute", 0, 59, &[])?;
        // At most every MIN_INTERVAL_MINUTES, like interval routines
        let too_often = minutes.len() > 1
            && (minutes.windows(2).any(|w| w[1] - w[0] < MIN_INTERVAL_MINUTES as u32)
                || 60 + minutes[0] - minutes[minutes.len() - 1] < MIN_INTERVAL_MINUTES as u32);
        if too_often {
            return Err(format!("cron routines may run at most every {MIN_INTERVAL_MINUTES} minutes"));
        }
        let hours = cron_field(hour, "hour", 0, 23, &[])?;
        let months = cron_field(month, "month", 1, 12, &MONTH_NAMES)?;
        let dom_any = dom == "*" || dom == "?";
        let mut dom_last = false;
        let mut dom_days = Vec::new();
        if !dom_any {
            for part in dom.split(',') {
                if part.eq_ignore_ascii_case("l") {
                    dom_last = true;
                } else {
                    dom_days.extend(cron_field(part, "day-of-month", 1, 31, &[])?);
                }
            }
        }
        let dow_any = dow == "*" || dow == "?";
        let (mut dow_days, mut dow_nth, mut dow_last) = (Vec::new(), Vec::new(), Vec::new());
        if !dow_any {
            let day = |v: &str| -> Result<u32, String> {
                cron_field(v, "day-of-week", 0, 7, &DAY_NAMES)?
                    .first()
                    .map(|d| d % 7)
                    .ok_or_else(|| format!("cron day-of-week '{v}' is not valid"))
            };
            for part in dow.split(',') {
                if let Some((d, n)) = part.split_once('#') {
                    let n = n.parse::<u32>().ok().filter(|n| (1..=5).contains(n)).ok_or_else(|| {
                        format!("cron day-of-week '{part}': use #1 to #5, e.g. mon#2 for the second Monday")
                    })?;
                    dow_nth.push((day(d)?, n));
                } else if let Some(d) = part.strip_suffix(['L', 'l']).filter(|d| !d.is_empty()) {
                    dow_last.push(day(d)?);
                } else {
                    dow_days.extend(cron_field(part, "day-of-week", 0, 7, &DAY_NAMES)?.into_iter().map(|d| d % 7));
                }
            }
        }
        Ok(Cron {
            minutes,
            hours,
            dom: dom_days,
            dom_last,
            dom_any,
            months,
            dow: dow_days,
            dow_nth,
            dow_last,
            dow_any,
        })
    }

    fn day_matches(&self, d: NaiveDate) -> bool {
        if !self.months.contains(&d.month()) {
            return false;
        }
        let last = last_day_of_month(d);
        let dom = || self.dom.contains(&d.day()) || (self.dom_last && d.day() == last);
        let wd = d.weekday().num_days_from_sunday();
        let dow = || {
            self.dow.contains(&wd)
                || self.dow_nth.iter().any(|&(w, n)| w == wd && (d.day() - 1) / 7 + 1 == n)
                || self.dow_last.iter().any(|&w| w == wd && d.day() + 7 > last)
        };
        match (self.dom_any, self.dow_any) {
            (true, true) => true,
            (true, false) => dow(),
            (false, true) => dom(),
            (false, false) => dom() || dow(),
        }
    }

    fn times(&self) -> Vec<NaiveTime> {
        self.hours
            .iter()
            .flat_map(|h| self.minutes.iter().filter_map(move |m| NaiveTime::from_hms_opt(*h, *m, 0)))
            .collect()
    }
}

/// `date` at wall-clock `time` in `tz`. A time skipped by a DST jump runs at the first valid
/// minute after it; a repeated time runs at its first occurrence.
fn resolve_local(tz: &Zone, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let naive = date.and_time(time);
    match tz.instants(naive) {
        LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t),
        LocalResult::None => {
            (1..=180).find_map(|m| match tz.instants(naive + Duration::minutes(m)) {
                LocalResult::Single(t) | LocalResult::Ambiguous(t, _) => Some(t),
                LocalResult::None => None,
            })
        }
    }
}

fn next_local(
    after: DateTime<Utc>,
    tz: Zone,
    time: NaiveTime,
    day_ok: impl Fn(NaiveDate) -> bool,
) -> Option<DateTime<Utc>> {
    next_local_within(after, &tz, 8, |d| day_ok(d).then_some(vec![time]))
}

/// The first local time after `after` within `days` days: `times_on` gives a day's wall-clock
/// times in order, or `None` when the day doesn't run.
fn next_local_within(
    after: DateTime<Utc>,
    tz: &Zone,
    days: u64,
    times_on: impl Fn(NaiveDate) -> Option<Vec<NaiveTime>>,
) -> Option<DateTime<Utc>> {
    let start = tz.date_at(after)?;
    (0..=days)
        .filter_map(|offset| start.checked_add_days(chrono::Days::new(offset)))
        .find_map(|d| {
            times_on(d)?
                .into_iter()
                .filter_map(|time| resolve_local(tz, d, time))
                .find(|t| *t > after)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn daily_today_or_tomorrow() {
        let s = Schedule::Daily {
            time: "09:00".into(),
            tz: "UTC".into(),
        };
        assert_eq!(
            s.next_after(utc("2026-10-17T08:00:00Z"), None),
            Some(utc("2026-10-17T09:00:00Z"))
        );
        assert_eq!(
            s.next_after(utc("2026-10-17T09:00:00Z"), None),
            Some(utc("2026-10-18T09:00:00Z"))
        );
    }

    #[test]
    fn daily_follows_dst() {
        // Kyiv: UTC+3 in summer, UTC+2 after 25 Oct 2026
        let s = Schedule::Daily {
            time: "09:00".into(),
            tz: "Europe/Kyiv".into(),
        };
        assert_eq!(
            s.next_after(utc("2026-10-24T12:00:00Z"), None),
            Some(utc("2026-10-25T07:00:00Z"))
        );
        assert_eq!(
            s.next_after(utc("2026-10-25T07:00:00Z"), None),
            Some(utc("2026-10-26T07:00:00Z"))
        );
        assert_eq!(
            s.next_after(utc("2026-10-23T12:00:00Z"), None),
            Some(utc("2026-10-24T06:00:00Z"))
        );
    }

    #[test]
    fn dst_rules_hold_past_the_listed_transitions() {
        // Slim tz files list no transitions this far out; the zone's rule string covers it
        let s = Schedule::Daily {
            time: "09:00".into(),
            tz: "Europe/Kyiv".into(),
        };
        assert_eq!(
            s.next_after(utc("2061-07-01T00:00:00Z"), None),
            Some(utc("2061-07-01T06:00:00Z"))
        );
        assert_eq!(
            s.next_after(utc("2061-12-01T00:00:00Z"), None),
            Some(utc("2061-12-01T07:00:00Z"))
        );
        // Southern hemisphere: Sydney is UTC+11 in January
        let s = Schedule::Daily {
            time: "09:00".into(),
            tz: "Australia/Sydney".into(),
        };
        assert_eq!(
            s.next_after(utc("2027-01-10T00:00:00Z"), None),
            Some(utc("2027-01-10T22:00:00Z"))
        );
    }

    #[test]
    fn skipped_and_repeated_local_times() {
        // New York springs forward 8 Mar 2026: 02:30 doesn't exist, runs at 03:00 EDT (07:00Z)
        let s = Schedule::Daily {
            time: "02:30".into(),
            tz: "America/New_York".into(),
        };
        assert_eq!(
            s.next_after(utc("2026-03-08T00:00:00Z"), None),
            Some(utc("2026-03-08T07:00:00Z"))
        );
        // Falls back 1 Nov 2026: 01:30 happens twice, runs at the first (EDT, 05:30Z) only
        let s = Schedule::Daily {
            time: "01:30".into(),
            tz: "America/New_York".into(),
        };
        assert_eq!(
            s.next_after(utc("2026-11-01T00:00:00Z"), None),
            Some(utc("2026-11-01T05:30:00Z"))
        );
        assert_eq!(
            s.next_after(utc("2026-11-01T05:30:00Z"), None),
            Some(utc("2026-11-02T06:30:00Z"))
        );
    }

    #[test]
    fn weekly_picks_next_listed_day() {
        // 2026-10-17 is a Saturday
        let s = Schedule::Weekly {
            time: "10:00".into(),
            weekdays: vec!["mon".into(), "thu".into()],
            tz: "UTC".into(),
        };
        assert_eq!(
            s.next_after(utc("2026-10-17T12:00:00Z"), None),
            Some(utc("2026-10-19T10:00:00Z"))
        );
        assert_eq!(
            s.next_after(utc("2026-10-19T10:00:00Z"), None),
            Some(utc("2026-10-22T10:00:00Z"))
        );
        // Same weekday, later time: a full week ahead
        let s = Schedule::Weekly {
            time: "10:00".into(),
            weekdays: vec!["sat".into()],
            tz: "UTC".into(),
        };
        assert_eq!(
            s.next_after(utc("2026-10-17T10:00:00Z"), None),
            Some(utc("2026-10-24T10:00:00Z"))
        );
    }

    #[test]
    fn interval_does_not_replay_missed_runs() {
        let s = Schedule::Interval { every_minutes: 30 };
        let now = utc("2026-10-17T12:00:00Z");
        assert_eq!(
            s.next_after(now, Some(utc("2026-10-17T11:45:00Z"))),
            Some(utc("2026-10-17T12:15:00Z"))
        );
        // Last ran hours ago (kwnt was down): next run is a full interval from now, not a backlog
        assert_eq!(
            s.next_after(now, Some(utc("2026-10-17T06:00:00Z"))),
            Some(utc("2026-10-17T12:30:00Z"))
        );
        assert_eq!(s.next_after(now, None), Some(utc("2026-10-17T12:30:00Z")));
    }

    #[test]
    fn once_in_the_past_never_runs() {
        let s = Schedule::Once {
            at: utc("2026-10-17T09:00:00Z"),
        };
        assert_eq!(
            s.next_after(utc("2026-10-17T08:00:00Z"), None),
            Some(utc("2026-10-17T09:00:00Z"))
        );
        assert_eq!(s.next_after(utc("2026-10-17T10:00:00Z"), None), None);
    }

    #[test]
    fn validation() {
        assert!(Schedule::Daily {
            time: "25:00".into(),
            tz: "UTC".into()
        }
        .validate()
        .is_err());
        assert!(Schedule::Daily {
            time: "09:00".into(),
            tz: "Mars/Olympus".into()
        }
        .validate()
        .is_err());
        assert!(Schedule::Weekly {
            time: "09:00".into(),
            weekdays: vec![],
            tz: "UTC".into()
        }
        .validate()
        .is_err());
        assert!(Schedule::Weekly {
            time: "09:00".into(),
            weekdays: vec!["funday".into()],
            tz: "UTC".into()
        }
        .validate()
        .is_err());
        for tz in [
            "/etc/passwd",
            "../../etc/passwd",
            "Europe/../../etc/passwd",
            "EST5EDT,M3.2.0,M11.1.0",
            "",
        ] {
            let s = Schedule::Daily {
                time: "09:00".into(),
                tz: tz.into(),
            };
            assert!(s.validate().is_err(), "{tz}");
        }
        assert!(Schedule::Interval { every_minutes: 4 }.validate().is_err());
        assert!(Schedule::Interval { every_minutes: 5 }.validate().is_ok());
        let parsed: Schedule = serde_json::from_str(
            r#"{"type":"weekly","time":"07:30","weekdays":["mon"],"tz":"Europe/Kyiv"}"#,
        )
        .unwrap();
        assert!(parsed.validate().is_ok());
        assert!(serde_json::from_str::<Schedule>(r#"{"type":"daily","time":"07:30"}"#).is_err());
    }

    #[test]
    fn legacy_zone_names_are_accepted_and_stored_canonically() {
        for (old, new) in [("Europe/Kiev", "Europe/Kyiv"), ("Asia/Calcutta", "Asia/Kolkata")] {
            let s = Schedule::Daily {
                time: "09:00".into(),
                tz: format!(" {old} "),
            };
            assert!(s.validate().is_ok(), "{old}");
            let canonical = s.clone().canonical();
            assert_eq!(
                canonical,
                Schedule::Daily {
                    time: "09:00".into(),
                    tz: new.into()
                }
            );
            let after = utc("2026-10-17T00:00:00Z");
            assert_eq!(s.next_after(after, None), canonical.next_after(after, None));
        }
        let weekly = Schedule::Weekly {
            time: "09:00".into(),
            weekdays: vec!["mon".into()],
            tz: "Asia/Calcutta".into(),
        }
        .canonical();
        assert!(matches!(weekly, Schedule::Weekly { ref tz, .. } if tz == "Asia/Kolkata"));
        // Kyiv is UTC+3 in summer: 09:00 local is 06:00Z
        assert_eq!(
            Schedule::Daily {
                time: "09:00".into(),
                tz: "Europe/Kiev".into()
            }
            .next_after(utc("2026-07-01T00:00:00Z"), None),
            Some(utc("2026-07-01T06:00:00Z"))
        );
        assert_eq!(canonical_tz("Europe/Kyiv"), "Europe/Kyiv");
        assert_eq!(canonical_tz("Mars/Olympus"), "Mars/Olympus");
        // Aliasing never opens a path around the name checks
        assert!(Schedule::Daily {
            time: "09:00".into(),
            tz: "Europe/Kiev/../../../etc/passwd".into()
        }
        .validate()
        .is_err());
    }

    fn monthly(days: &str, time: &str, tz: &str) -> Schedule {
        serde_json::from_str(&format!(r#"{{"type":"monthly","time":"{time}","days":{days},"tz":"{tz}"}}"#)).unwrap()
    }

    fn cron(expression: &str, tz: &str) -> Schedule {
        Schedule::Cron { expression: expression.into(), tz: tz.into() }
    }

    #[test]
    fn monthly_runs_every_month_not_once() {
        // "the 1st of each month at 09:00 Berlin": Berlin is UTC+2 until 25 Oct, then UTC+1
        let s = monthly("[1]", "09:00", "Europe/Berlin");
        assert!(s.validate().is_ok());
        let first = s.next_after(utc("2026-10-18T21:45:00Z"), None).unwrap();
        assert_eq!(first, utc("2026-11-01T08:00:00Z"));
        let second = s.next_after(first, Some(first)).unwrap();
        assert_eq!(second, utc("2026-12-01T08:00:00Z"));
        // Across spring forward: 1 April is summer time again
        assert_eq!(
            s.next_after(utc("2027-03-02T00:00:00Z"), None),
            Some(utc("2027-04-01T07:00:00Z"))
        );
    }

    #[test]
    fn monthly_last_day_and_short_months() {
        let last = monthly(r#"["last"]"#, "18:00", "UTC");
        assert_eq!(last.next_after(utc("2027-02-01T00:00:00Z"), None), Some(utc("2027-02-28T18:00:00Z")));
        assert_eq!(last.next_after(utc("2028-02-01T00:00:00Z"), None), Some(utc("2028-02-29T18:00:00Z")));
        assert_eq!(last.next_after(utc("2026-10-31T18:00:00Z"), None), Some(utc("2026-11-30T18:00:00Z")));
        // The 31st falls back to a short month's last day, once
        let s = monthly("[31]", "08:00", "UTC");
        assert_eq!(s.next_after(utc("2026-11-01T00:00:00Z"), None), Some(utc("2026-11-30T08:00:00Z")));
        assert_eq!(s.next_after(utc("2026-11-30T08:00:00Z"), None), Some(utc("2026-12-31T08:00:00Z")));
        // Several days, and "last" written as a string number
        let s = monthly(r#"[15, "1"]"#, "08:00", "UTC");
        assert_eq!(s.next_after(utc("2026-10-18T00:00:00Z"), None), Some(utc("2026-11-01T08:00:00Z")));
        assert_eq!(s.next_after(utc("2026-11-01T08:00:00Z"), None), Some(utc("2026-11-15T08:00:00Z")));
        assert!(serde_json::from_str::<Schedule>(r#"{"type":"monthly","time":"08:00","days":[32],"tz":"UTC"}"#).is_err());
        assert!(serde_json::from_str::<Schedule>(r#"{"type":"monthly","time":"08:00","days":[0],"tz":"UTC"}"#).is_err());
        assert!(monthly("[]", "08:00", "UTC").validate().is_err());
        assert!(monthly("[1]", "8am", "UTC").validate().is_err());
        assert!(monthly("[1]", "08:00", "Mars/Base").validate().is_err());
        // Round-trips as stored
        let stored = serde_json::to_value(monthly(r#"[1, "last"]"#, "08:00", "UTC")).unwrap();
        assert_eq!(stored["days"], serde_json::json!([1, "last"]));
    }

    #[test]
    fn monthly_at_a_skipped_local_time() {
        // New York, 02:30 on 8 Mar 2026 doesn't exist: runs at 03:00 EDT
        let s = monthly("[8]", "02:30", "America/New_York");
        assert_eq!(s.next_after(utc("2026-03-01T00:00:00Z"), None), Some(utc("2026-03-08T07:00:00Z")));
        assert_eq!(s.next_after(utc("2026-03-08T07:00:00Z"), None), Some(utc("2026-04-08T06:30:00Z")));
    }

    #[test]
    fn cron_calendar_rules() {
        let after = utc("2026-10-18T21:45:00Z"); // a Sunday
        // 1st of the month at 09:00 Kyiv (UTC+2 from 25 Oct)
        assert_eq!(cron("0 9 1 * *", "Europe/Kyiv").next_after(after, None), Some(utc("2026-11-01T07:00:00Z")));
        // Last day of the month
        assert_eq!(cron("0 9 L * *", "UTC").next_after(after, None), Some(utc("2026-10-31T09:00:00Z")));
        // Weekdays at 07:30
        let s = cron("30 7 * * mon-fri", "UTC");
        assert_eq!(s.next_after(after, None), Some(utc("2026-10-19T07:30:00Z")));
        assert_eq!(s.next_after(utc("2026-10-23T07:30:00Z"), None), Some(utc("2026-10-26T07:30:00Z")));
        // Second Monday of each month; last Friday of each month
        assert_eq!(cron("0 10 * * MON#2", "UTC").next_after(after, None), Some(utc("2026-11-09T10:00:00Z")));
        assert_eq!(cron("0 17 * * 5L", "UTC").next_after(after, None), Some(utc("2026-10-30T17:00:00Z")));
        // Quarterly on the 1st; Sunday as 0 and 7
        assert_eq!(cron("0 8 1 jan,apr,jul,oct *", "UTC").next_after(after, None), Some(utc("2027-01-01T08:00:00Z")));
        assert_eq!(cron("0 8 * * 7", "UTC").next_after(after, None), Some(utc("2026-10-25T08:00:00Z")));
        assert_eq!(cron("0 8 * * 0", "UTC").next_after(after, None), Some(utc("2026-10-25T08:00:00Z")));
        // Both day fields restricted: either matches (classic cron)
        assert_eq!(cron("0 8 1 * mon", "UTC").next_after(after, None), Some(utc("2026-10-19T08:00:00Z")));
        // Steps: every 15 minutes during 9-10 on the hour range
        let s = cron("*/15 9-10 * * *", "UTC");
        assert_eq!(s.next_after(utc("2026-10-19T09:50:00Z"), None), Some(utc("2026-10-19T10:00:00Z")));
        assert_eq!(s.next_after(utc("2026-10-19T10:45:00Z"), None), Some(utc("2026-10-20T09:00:00Z")));
        // 29 Feb only in leap years
        assert_eq!(cron("0 0 29 2 *", "UTC").next_after(after, None), Some(utc("2028-02-29T00:00:00Z")));
    }

    #[test]
    fn cron_across_dst() {
        // 01:30 daily in New York around the fall-back on 1 Nov 2026: first 01:30 only
        let s = cron("30 1 * * *", "America/New_York");
        assert_eq!(s.next_after(utc("2026-11-01T00:00:00Z"), None), Some(utc("2026-11-01T05:30:00Z")));
        assert_eq!(s.next_after(utc("2026-11-01T05:30:00Z"), None), Some(utc("2026-11-02T06:30:00Z")));
        // 02:30 on spring-forward day doesn't exist: 03:00 EDT, then normal
        let s = cron("30 2 * * *", "America/New_York");
        assert_eq!(s.next_after(utc("2026-03-08T00:00:00Z"), None), Some(utc("2026-03-08T07:00:00Z")));
        assert_eq!(s.next_after(utc("2026-03-08T07:00:00Z"), None), Some(utc("2026-03-09T06:30:00Z")));
        // Every 30 minutes through the spring-forward gap: no duplicate, no stall
        let s = cron("0,30 * * * *", "America/New_York");
        let mut t = utc("2026-03-08T06:15:00Z"); // 01:15 EST
        let mut seen = Vec::new();
        for _ in 0..4 {
            t = s.next_after(t, Some(t)).unwrap();
            seen.push(t);
        }
        assert_eq!(
            seen,
            vec![
                utc("2026-03-08T06:30:00Z"), // 01:30 EST
                utc("2026-03-08T07:00:00Z"), // 03:00 EDT (02:00 and 02:30 skipped into it)
                utc("2026-03-08T07:30:00Z"),
                utc("2026-03-08T08:00:00Z"),
            ]
        );
    }

    #[test]
    fn cron_validation() {
        for bad in [
            "",
            "0 9 * *",
            "0 9 * * * *",
            "60 9 * * *",
            "0 24 * * *",
            "0 9 32 * *",
            "0 9 * 13 *",
            "0 9 * * 8",
            "0 9 * * mon#6",
            "0 9 * * funday",
            "* * * * *",
            "*/2 * * * *",
            "0,58 * * * *",
            "0 9 10-5 * *",
            "0 9 */0 * *",
            "@daily",
            "0 9 30 2 *",
        ] {
            assert!(cron(bad, "UTC").validate().is_err(), "{bad:?}");
        }
        assert!(cron("0 9 1 * *", "Nowhere/Zone").validate().is_err());
        for ok in ["0 9 1 * *", "*/5 * * * *", "0 9 L * *", "0 9 * * MON#1", "15 8 * * sat,sun", " 0  9 1 * * "] {
            assert!(cron(ok, "Europe/Kiev").validate().is_ok(), "{ok:?}");
        }
        let c = cron(" 0  9 1 * * ", "Europe/Kiev").canonical();
        assert_eq!(c, cron("0 9 1 * *", "Europe/Kyiv"));
        let parsed: Schedule = serde_json::from_str(r#"{"type":"cron","expression":"0 9 1 * *","tz":"UTC"}"#).unwrap();
        assert!(parsed.validate().is_ok());
        assert!(serde_json::from_str::<Schedule>(r#"{"type":"cron","expression":"0 9 1 * *"}"#).is_err());
    }
}
