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

fn parse_tz(tz: &str) -> Result<Zone, String> {
    let name = tz.trim();
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
        }
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
    let start = tz.date_at(after)?;
    (0..=8)
        .filter_map(|offset| start.checked_add_days(chrono::Days::new(offset)))
        .filter(|d| day_ok(*d))
        .filter_map(|d| resolve_local(&tz, d, time))
        .find(|t| *t > after)
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
}
