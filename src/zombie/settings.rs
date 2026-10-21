//! The person's Zombie Hunter settings, one JSON value in the `settings` table.

use crate::db::{Database, DbError};
use crate::routines::Schedule;
use serde::{Deserialize, Serialize};

pub const SETTINGS_KEY: &str = "zombie_settings";
/// When the next weekly review is due (RFC 3339), kept apart so saving settings doesn't race it.
pub const NEXT_REVIEW_KEY: &str = "zombie_next_review_at";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Off: no scans, no reviews. Cases already found stay.
    pub enabled: bool,
    /// Email and GitHub: days without an answer before a conversation counts.
    pub stale_days: i64,
    /// kwant's own threads, tasks and questions.
    pub kwant_stale_days: i64,
    /// How far back a scan reads.
    pub lookback_days: i64,
    /// Read kwant's own threads, tasks and questions.
    pub kwant_source: bool,
    /// The weekly "Loose ends review".
    pub review_enabled: bool,
    pub review_time: String,
    pub review_weekdays: Vec<String>,
    /// IANA time zone; empty uses the person's kwant time zone, then the computer's.
    pub review_tz: String,
    /// Most cases one review shows.
    pub max_per_review: usize,
    /// `off`: rules only. `auto`: a model double-checks each candidate (toolless, quote verified).
    pub llm: String,
    /// The model the check uses; empty picks the first engine that can run.
    pub llm_model: String,
    /// Closed and dismissed cases are wiped down to their dedupe key after this many days.
    pub retention_days: i64,
    /// Addresses that are the person's, besides each mailbox's login.
    pub my_addresses: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            stale_days: 14,
            kwant_stale_days: 3,
            lookback_days: 120,
            kwant_source: true,
            review_enabled: true,
            review_time: "09:00".into(),
            review_weekdays: vec!["mon".into()],
            review_tz: String::new(),
            max_per_review: 5,
            llm: "off".into(),
            llm_model: String::new(),
            retention_days: 90,
            my_addresses: Vec::new(),
        }
    }
}

impl Settings {
    pub async fn load(db: &Database) -> Result<Self, DbError> {
        Ok(db
            .get_setting(SETTINGS_KEY)
            .await?
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default())
    }

    pub async fn save(&self, db: &Database) -> Result<(), DbError> {
        db.set_setting(SETTINGS_KEY, &serde_json::to_string(self).unwrap_or_default()).await
    }

    /// A short reason when a value is out of range.
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=365).contains(&self.stale_days) {
            return Err("stale_days must be between 1 and 365".into());
        }
        if !(1..=365).contains(&self.kwant_stale_days) {
            return Err("kwant_stale_days must be between 1 and 365".into());
        }
        if !(7..=730).contains(&self.lookback_days) {
            return Err("lookback_days must be between 7 and 730".into());
        }
        if !(1..=20).contains(&self.max_per_review) {
            return Err("max_per_review must be between 1 and 20".into());
        }
        if !(1..=3650).contains(&self.retention_days) {
            return Err("retention_days must be between 1 and 3650".into());
        }
        if !matches!(self.llm.as_str(), "off" | "auto") {
            return Err("llm must be \"off\" or \"auto\"".into());
        }
        if self.my_addresses.len() > 20 || self.my_addresses.iter().any(|a| !a.contains('@') || a.len() > 254) {
            return Err("my_addresses must be up to 20 email addresses".into());
        }
        if self.llm_model.len() > 200 {
            return Err("llm_model is too long".into());
        }
        // Reuses the routines' own checks for the time, the days and the zone
        self.schedule("UTC").validate()?;
        if !self.review_tz.trim().is_empty() {
            self.schedule(&self.review_tz).validate()?;
        }
        Ok(())
    }

    /// The weekly review's schedule in `fallback_tz` unless the settings name a zone.
    pub fn schedule(&self, fallback_tz: &str) -> Schedule {
        let tz = if self.review_tz.trim().is_empty() { fallback_tz } else { self.review_tz.trim() };
        Schedule::Weekly { time: self.review_time.clone(), weekdays: self.review_weekdays.clone(), tz: tz.to_string() }
    }
}

/// The time zone a review runs in: the person's kwant setting, else this computer's, else UTC.
pub async fn local_tz(db: &Database) -> String {
    if let Some(tz) = db
        .get_setting(crate::server::bot_actions::TIMEZONE_SETTING)
        .await
        .ok()
        .flatten()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
    {
        return tz;
    }
    host_tz().unwrap_or_else(|| "UTC".into())
}

/// This computer's IANA zone: `TZ`, `/etc/timezone` (Debian), or the `/etc/localtime` link
/// (macOS and most Linux).
pub fn host_tz() -> Option<String> {
    let valid = |s: &str| {
        let s = s.trim().trim_start_matches(':');
        (!s.is_empty() && !s.starts_with('/') && s.contains('/')).then(|| s.to_string())
    };
    if let Some(tz) = std::env::var("TZ").ok().and_then(|t| valid(&t)) {
        return Some(tz);
    }
    if let Some(tz) = std::fs::read_to_string("/etc/timezone").ok().and_then(|t| valid(&t)) {
        return Some(tz);
    }
    let link = std::fs::read_link("/etc/localtime").ok()?;
    let link = link.to_string_lossy();
    link.split_once("zoneinfo/").and_then(|(_, name)| valid(name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_monday_nine() {
        let s = Settings::default();
        s.validate().unwrap();
        assert_eq!(s.schedule("Europe/Kyiv"), Schedule::Weekly { time: "09:00".into(), weekdays: vec!["mon".into()], tz: "Europe/Kyiv".into() });
    }

    #[test]
    fn bad_values_are_refused() {
        let bad = [
            Settings { stale_days: 0, ..Default::default() },
            Settings { llm: "always".into(), ..Default::default() },
            Settings { review_time: "25:00".into(), ..Default::default() },
            Settings { review_weekdays: vec!["someday".into()], ..Default::default() },
            Settings { review_tz: "../../etc/passwd".into(), ..Default::default() },
            Settings { my_addresses: vec!["not-an-address".into()], ..Default::default() },
        ];
        for s in bad {
            assert!(s.validate().is_err(), "{s:?}");
        }
    }

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{"stale_days": 7}"#).unwrap();
        assert_eq!(s.stale_days, 7);
        assert_eq!(s.max_per_review, 5);
    }
}
