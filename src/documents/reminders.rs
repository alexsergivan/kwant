//! Deadline reminders for letters: 7 days and 1 day before a confirmed deadline, and on the day,
//! at 09:00 in the person's time zone. Stored next to the case (`document_reminders`), replaced
//! when the deadline changes, cleared when the case closes. An uncertain or unconfirmed deadline
//! schedules nothing. A notification says how many letters are due, never who sent them or what
//! they say.

use crate::db::Database;
use chrono::{DateTime, Duration, NaiveDate, NaiveTime, Utc};

/// Days before the deadline a reminder fires.
pub const OFFSETS: [i64; 3] = [7, 1, 0];
/// How often the background loop looks for due reminders.
pub const TICK: std::time::Duration = std::time::Duration::from_secs(300);

fn remind_at() -> NaiveTime {
    NaiveTime::from_hms_opt(9, 0, 0).expect("valid time")
}

/// When each reminder for `deadline` fires: 09:00 local on the day, skipping days already past;
/// one due earlier today fires at `now`.
pub fn schedule(deadline: NaiveDate, tz: &str, now: DateTime<Utc>) -> Vec<(i64, DateTime<Utc>)> {
    let today = crate::routines::local_date(tz, now).ok().flatten().unwrap_or_else(|| now.date_naive());
    OFFSETS
        .iter()
        .filter_map(|&offset| {
            let day = deadline - Duration::days(offset);
            if day < today {
                return None;
            }
            let at = crate::routines::local_instant(tz, day, remind_at())
                .ok()
                .flatten()
                .unwrap_or_else(|| day.and_time(remind_at()).and_utc());
            Some((offset, at.max(now)))
        })
        .collect()
}

/// What one tick fired: the cases reminded about, by the offset that fired.
#[derive(Debug, Default, PartialEq)]
pub struct Fired {
    pub by_offset: Vec<(i64, Vec<String>)>,
}

impl Fired {
    pub fn is_empty(&self) -> bool {
        self.by_offset.is_empty()
    }

    pub fn total(&self) -> usize {
        self.by_offset.iter().map(|(_, c)| c.len()).sum()
    }

    /// Counts only: "1 letter is due today. 2 letters have a deadline in 7 days."
    pub fn body(&self) -> String {
        let mut parts = Vec::new();
        for offset in [0, 1, 7] {
            let Some((_, cases)) = self.by_offset.iter().find(|(o, _)| *o == offset) else { continue };
            let n = cases.len();
            let letters = if n == 1 { "1 letter".to_string() } else { format!("{n} letters") };
            parts.push(match (offset, n == 1) {
                (0, true) => format!("{letters} is due today."),
                (0, false) => format!("{letters} are due today."),
                (1, true) => format!("{letters} is due tomorrow."),
                (1, false) => format!("{letters} are due tomorrow."),
                (_, true) => format!("{letters} has a deadline in {offset} days."),
                (_, false) => format!("{letters} have a deadline in {offset} days."),
            });
        }
        parts.join(" ")
    }

    pub fn title(&self) -> String {
        let n = self.total();
        format!("Letter deadline{}: {n} to deal with", if n == 1 { "" } else { "s" })
    }
}

/// Fires the reminders due at `now`: per case only the nearest one (a server that was off for a
/// week says "due tomorrow", not also "in 7 days"); the others due with it are marked fired.
/// A case that is closed, dismissed, wiped or gone loses its reminders instead.
pub async fn tick(db: &Database, now: DateTime<Utc>) -> Result<Fired, crate::db::DbError> {
    let due = db.document_due_reminders(now).await?;
    let mut fired = Fired::default();
    let mut i = 0;
    while i < due.len() {
        let case_id = due[i].case_id.clone();
        let mut j = i;
        while j < due.len() && due[j].case_id == case_id {
            j += 1;
        }
        let group = &due[i..j];
        i = j;
        let live = db.zombie_case(&case_id).await?.filter(|c| !c.redacted && !matches!(c.status.as_str(), "closed" | "dismissed"));
        if live.is_none() {
            db.document_clear_reminders(&case_id).await?;
            continue;
        }
        let nearest = group.iter().map(|r| r.offset_days).min().unwrap_or(0);
        for r in group {
            db.document_mark_fired(&case_id, r.offset_days, now).await?;
        }
        match fired.by_offset.iter_mut().find(|(o, _)| *o == nearest) {
            Some((_, cases)) => cases.push(case_id),
            None => fired.by_offset.push((nearest, vec![case_id])),
        }
    }
    Ok(fired)
}

/// Runs `tick` every `TICK` and sends one notification per tick that fired anything.
pub fn spawn(state: crate::server::AppState, shutdown: tokio_util::sync::CancellationToken) {
    tokio::spawn(async move {
        crate::server::document_routes::mark_interrupted(&state).await;
        loop {
            run_once(&state, Utc::now()).await;
            tokio::select! {
                _ = shutdown.cancelled() => return,
                _ = tokio::time::sleep(TICK) => {}
            }
        }
    });
}

/// One tick and its notification.
pub async fn run_once(state: &crate::server::AppState, now: DateTime<Utc>) -> Fired {
    let fired = match tick(&state.db, now).await {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("documents: reminder tick failed: {e}");
            return Fired::default();
        }
    };
    if !fired.is_empty() {
        tracing::info!(letters = fired.total(), "documents: deadline reminders fired");
        crate::server::continuity::notify(
            state,
            crate::server::continuity::Notification {
                kind: crate::zombie::hunt::NOTIFY_KIND,
                kwant_id: String::new(),
                kwant_name: "Loose ends".into(),
                channel_id: None,
                title: fired.title(),
                body: fired.body(),
            },
        )
        .await;
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn three_reminders_at_nine_local_time() {
        let now = Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap();
        let s = schedule(d(2026, 11, 30), "Europe/London", now);
        assert_eq!(
            s,
            vec![
                (7, Utc.with_ymd_and_hms(2026, 11, 23, 9, 0, 0).unwrap()),
                (1, Utc.with_ymd_and_hms(2026, 11, 29, 9, 0, 0).unwrap()),
                (0, Utc.with_ymd_and_hms(2026, 11, 30, 9, 0, 0).unwrap()),
            ]
        );
        // Kyiv is UTC+2 in winter: 09:00 there is 07:00 UTC
        let s = schedule(d(2026, 11, 30), "Europe/Kyiv", now);
        assert_eq!(s[2].1, Utc.with_ymd_and_hms(2026, 11, 30, 7, 0, 0).unwrap());
    }

    #[test]
    fn past_days_are_skipped_and_one_due_earlier_today_fires_now() {
        let now = Utc.with_ymd_and_hms(2026, 11, 29, 15, 0, 0).unwrap();
        let s = schedule(d(2026, 11, 30), "UTC", now);
        assert_eq!(s, vec![(1, now), (0, Utc.with_ymd_and_hms(2026, 11, 30, 9, 0, 0).unwrap())]);
        assert!(schedule(d(2026, 11, 28), "UTC", now).is_empty(), "a past deadline schedules nothing");
        // A bad zone falls back to UTC instead of losing the reminder
        assert_eq!(schedule(d(2026, 11, 30), "Not/AZone", now).len(), 2);
    }

    #[test]
    fn the_body_has_counts_only() {
        let f = Fired { by_offset: vec![(7, vec!["a".into(), "b".into()]), (0, vec!["c".into()])] };
        assert_eq!(f.body(), "1 letter is due today. 2 letters have a deadline in 7 days.");
        assert_eq!(f.title(), "Letter deadlines: 3 to deal with");
        let f = Fired { by_offset: vec![(1, vec!["a".into()])] };
        assert_eq!(f.body(), "1 letter is due tomorrow.");
    }
}
