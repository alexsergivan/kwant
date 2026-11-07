//! Show it once (0.7.9): a recording of what the person did in a taken-over browser, kept as a
//! list of steps until a skill is written from it, then for a short while after. Steps are masked
//! and cleaned before they get here (`crate::server::show_once`); this module only stores them,
//! enforces the limits and deletes them on time.
//!
//! Times are whole seconds since the epoch, always from `crate::clock::now()` (the caller passes
//! `now`), so a test can cross the 30-minute limit and the 7-day deletion.

use super::skills::{Skill, SkillError, SkillText, MAX_PENDING_PER_KWANT, MAX_SKILL_NAME_CHARS};
use super::{Database, DbError};
use chrono::{TimeZone, Utc};
use serde::Serialize;
use sqlx::FromRow;
use uuid::Uuid;

/// Most counted steps in one recording (notes don't count).
pub const LIMIT_STEPS: i64 = 200;
/// Most recording time in seconds, summed over Keep recording parts.
pub const LIMIT_SECS: i64 = 30 * 60;
/// "Near the limit" from this many steps left...
pub const NEAR_STEPS_LEFT: i64 = 8;
/// ...or from this much time used.
pub const NEAR_SECS: i64 = 28 * 60;
/// Most notes in one recording.
pub const MAX_NOTES: i64 = 50;
/// How long steps are kept after the recording ends or the skill is approved.
pub const KEEP_STEPS_SECS: i64 = 7 * 24 * 60 * 60;
/// A deleted step's Undo window.
pub const STEP_UNDO_SECS: i64 = 10;

/// A recording that is still being worked on: at most one per bot.
const OPEN_STATES: &str = "('recording', 'stopped', 'writing')";

#[derive(Debug, Clone, FromRow, PartialEq)]
pub struct Recording {
    pub id: String,
    pub kwant_id: String,
    pub space_id: String,
    pub stage: String,
    /// `recording`, `stopped`, `writing` or `written`.
    pub state: String,
    pub stop_reason: Option<String>,
    pub started_at: i64,
    /// When the current part began; set only while `recording`.
    pub segment_started_at: Option<i64>,
    /// Recording time of the finished parts.
    pub elapsed_secs: i64,
    pub ended_at: Option<i64>,
    pub step_count: i64,
    pub note_count: i64,
    pub near_sent: i64,
    /// JSON array of origins, in the order first seen.
    pub origins: String,
    pub write_error: Option<String>,
    pub skill_id: Option<String>,
    /// JSON array of `{name, description, example}` once written.
    pub inputs: Option<String>,
    pub shown_at: Option<i64>,
    pub shown_step_count: Option<i64>,
    pub approved_at: Option<i64>,
    pub steps_expire_at: Option<i64>,
    pub steps_deleted_at: Option<i64>,
}

impl Recording {
    /// Recording time used at `now`, all parts, never past the limit.
    pub fn elapsed_at(&self, now: i64) -> i64 {
        let running = match (self.state.as_str(), self.segment_started_at) {
            ("recording", Some(start)) => (now - start).max(0),
            _ => 0,
        };
        (self.elapsed_secs + running).min(LIMIT_SECS)
    }

    /// From 28:00 used or with 8 counted steps left.
    pub fn near_at(&self, now: i64) -> bool {
        LIMIT_STEPS - self.step_count <= NEAR_STEPS_LEFT || self.elapsed_at(now) >= NEAR_SECS
    }

    pub fn origin_list(&self) -> Vec<String> {
        serde_json::from_str(&self.origins).unwrap_or_default()
    }

    pub fn is_open(&self) -> bool {
        matches!(self.state.as_str(), "recording" | "stopped" | "writing")
    }

    /// The limit that is used up, if any.
    pub fn limit_reached(&self, now: i64) -> Option<&'static str> {
        if self.step_count >= LIMIT_STEPS {
            Some("limit_steps")
        } else if self.elapsed_at(now) >= LIMIT_SECS {
            Some("limit_time")
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, FromRow, PartialEq, Serialize)]
pub struct StepRow {
    pub id: i64,
    pub recording_id: String,
    pub ord: f64,
    pub kind: String,
    pub origin: Option<String>,
    pub path: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub masked: Option<String>,
    pub at: i64,
    pub deleted_at: Option<i64>,
    pub undo_until: Option<i64>,
}

/// A step to store, already masked and cleaned.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NewStep {
    pub kind: String,
    pub origin: Option<String>,
    pub path: Option<String>,
    pub role: Option<String>,
    pub name: Option<String>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub masked: Option<String>,
}

#[derive(Debug, PartialEq)]
pub enum ShowOnceError {
    NotFound,
    /// The bot already has an open recording.
    AlreadyOpen(Box<Recording>),
    /// Another bot's recording is live on the stage.
    StageBusy,
    /// The action doesn't fit the recording's state.
    WrongState(String),
    LimitReached,
    TooManyNotes,
    NoSteps,
    UndoOver,
    Db(String),
}

impl From<sqlx::Error> for ShowOnceError {
    fn from(e: sqlx::Error) -> Self {
        ShowOnceError::Db(e.to_string())
    }
}

impl From<DbError> for ShowOnceError {
    fn from(e: DbError) -> Self {
        ShowOnceError::Db(e.to_string())
    }
}

/// What adding an action to a live recording did.
#[derive(Debug, PartialEq)]
pub enum Added {
    /// Stored; `near` is true the one time the recording first gets near a limit.
    Step { step: StepRow, near: bool },
    /// A limit was used up: the recording is now stopped with this reason and the action wasn't stored.
    Stopped { recording: Box<Recording>, reason: String },
}

/// What one sweep did.
#[derive(Debug, Default, Clone, PartialEq, Serialize)]
pub struct SweepCounts {
    /// Recordings stopped (time limit, or left over from a restart).
    pub stopped: u64,
    /// Step rows deleted (past their deadline or their Undo).
    pub purged_steps: u64,
    /// Recordings deleted whole (never written and past their deadline, or their bot is gone).
    pub expired_recordings: u64,
    /// The recordings whose state changed, for the live update.
    #[serde(skip)]
    pub changed: Vec<String>,
}

fn new_recording_id() -> String {
    format!("rec_{}", Uuid::new_v4().simple())
}

/// `base`, or `base-2`, `base-3`... : the first that `taken` doesn't hold, within the name limit.
pub fn free_name(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    for n in 2.. {
        let suffix = format!("-{n}");
        let keep = MAX_SKILL_NAME_CHARS.saturating_sub(suffix.len());
        let head: String = base.chars().take(keep).collect();
        let name = format!("{}{suffix}", head.trim_end_matches('-'));
        if !taken.iter().any(|t| *t == name) {
            return name;
        }
    }
    unreachable!()
}

impl Database {
    /// Add-only. Runs last: earlier steps can rebuild `kwants` or `skills`, which drops triggers.
    pub(super) async fn migrate_show_once(&self) -> Result<(), DbError> {
        for stmt in [
            "CREATE TABLE IF NOT EXISTS skill_recordings (
                id TEXT PRIMARY KEY,
                kwant_id TEXT NOT NULL,
                space_id TEXT NOT NULL,
                stage TEXT NOT NULL,
                state TEXT NOT NULL CHECK (state IN ('recording', 'stopped', 'writing', 'written')),
                stop_reason TEXT,
                started_at INTEGER NOT NULL,
                segment_started_at INTEGER,
                elapsed_secs INTEGER NOT NULL DEFAULT 0,
                ended_at INTEGER,
                step_count INTEGER NOT NULL DEFAULT 0,
                note_count INTEGER NOT NULL DEFAULT 0,
                near_sent INTEGER NOT NULL DEFAULT 0,
                origins TEXT NOT NULL DEFAULT '[]',
                write_error TEXT,
                skill_id TEXT,
                inputs TEXT,
                shown_at INTEGER,
                shown_step_count INTEGER,
                approved_at INTEGER,
                steps_expire_at INTEGER,
                steps_deleted_at INTEGER
            )",
            "CREATE INDEX IF NOT EXISTS skill_recordings_by_kwant ON skill_recordings (kwant_id, state)",
            "CREATE INDEX IF NOT EXISTS skill_recordings_by_skill ON skill_recordings (skill_id)",
            "CREATE TABLE IF NOT EXISTS skill_recording_steps (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recording_id TEXT NOT NULL,
                ord REAL NOT NULL,
                kind TEXT NOT NULL CHECK (kind IN ('page', 'click', 'type', 'select', 'key', 'upload', 'note')),
                origin TEXT,
                path TEXT,
                role TEXT,
                name TEXT,
                label TEXT,
                value TEXT,
                masked TEXT CHECK (masked IS NULL OR masked IN ('password', 'code', 'payment', 'secret', 'saved_login')),
                at INTEGER NOT NULL,
                deleted_at INTEGER,
                undo_until INTEGER
            )",
            "CREATE INDEX IF NOT EXISTS skill_recording_steps_order ON skill_recording_steps (recording_id, ord)",
            // Deleting a skill or a bot deletes its recordings and their steps at once (A28)
            "CREATE TRIGGER IF NOT EXISTS show_once_skill_deleted AFTER DELETE ON skills BEGIN
                DELETE FROM skill_recording_steps WHERE recording_id IN (SELECT id FROM skill_recordings WHERE skill_id = OLD.id);
                DELETE FROM skill_recordings WHERE skill_id = OLD.id;
            END",
            "CREATE TRIGGER IF NOT EXISTS show_once_kwant_deleted AFTER DELETE ON kwants BEGIN
                DELETE FROM skill_recording_steps WHERE recording_id IN (SELECT id FROM skill_recordings WHERE kwant_id = OLD.id);
                DELETE FROM skill_recordings WHERE kwant_id = OLD.id;
            END",
        ] {
            sqlx::query(stmt).execute(&self.pool).await?;
        }
        Ok(())
    }

    pub async fn get_recording(&self, id: &str) -> Result<Option<Recording>, DbError> {
        Ok(sqlx::query_as::<_, Recording>("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// The bot's open recording (`recording`, `stopped` or `writing`), if any.
    pub async fn open_recording_for(&self, kwant_id: &str) -> Result<Option<Recording>, DbError> {
        Ok(sqlx::query_as::<_, Recording>(&format!(
            "SELECT * FROM skill_recordings WHERE kwant_id = ? AND state IN {OPEN_STATES} ORDER BY started_at DESC LIMIT 1"
        ))
        .bind(kwant_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The recording live on a stage, if any.
    pub async fn live_recording_on(&self, stage: &str) -> Result<Option<Recording>, DbError> {
        Ok(sqlx::query_as::<_, Recording>(
            "SELECT * FROM skill_recordings WHERE stage = ? AND state = 'recording' ORDER BY started_at DESC LIMIT 1",
        )
        .bind(stage)
        .fetch_optional(&self.pool)
        .await?)
    }

    /// The recording a shown skill was written from.
    pub async fn recording_for_skill(&self, skill_id: &str) -> Result<Option<Recording>, DbError> {
        Ok(sqlx::query_as::<_, Recording>("SELECT * FROM skill_recordings WHERE skill_id = ? LIMIT 1")
            .bind(skill_id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Starts a recording. One open recording per bot; one live recording per stage.
    pub async fn start_recording(
        &self,
        kwant_id: &str,
        space_id: &str,
        stage: &str,
        now: i64,
    ) -> Result<Recording, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let open: Option<Recording> = sqlx::query_as(&format!(
            "SELECT * FROM skill_recordings WHERE kwant_id = ? AND state IN {OPEN_STATES} LIMIT 1"
        ))
        .bind(kwant_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(r) = open {
            tx.rollback().await?;
            return Err(ShowOnceError::AlreadyOpen(Box::new(r)));
        }
        let busy: Option<(String,)> =
            sqlx::query_as("SELECT id FROM skill_recordings WHERE stage = ? AND state = 'recording' LIMIT 1")
                .bind(stage)
                .fetch_optional(&mut *tx)
                .await?;
        if busy.is_some() {
            tx.rollback().await?;
            return Err(ShowOnceError::StageBusy);
        }
        let id = new_recording_id();
        sqlx::query(
            "INSERT INTO skill_recordings (id, kwant_id, space_id, stage, state, started_at, segment_started_at)
             VALUES (?, ?, ?, ?, 'recording', ?, ?)",
        )
        .bind(&id)
        .bind(kwant_id)
        .bind(space_id)
        .bind(stage)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_recording(&id).await?.ok_or(ShowOnceError::NotFound)
    }

    /// Ends the current part. A recording that isn't live is left as it is (`Ok(false)`).
    /// The steps are kept until `ended_at` + 7 days (A28).
    pub async fn stop_recording(&self, id: &str, reason: &str, now: i64) -> Result<bool, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let changed = stop_in(&mut tx, id, reason, now).await?;
        tx.commit().await?;
        Ok(changed)
    }

    /// Keep recording: a stopped, never-written recording goes live again on its stage.
    pub async fn resume_recording(&self, id: &str, now: i64) -> Result<Recording, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let rec: Recording = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ShowOnceError::NotFound)?;
        if rec.state != "stopped" {
            tx.rollback().await?;
            return Err(ShowOnceError::WrongState(rec.state));
        }
        if rec.limit_reached(now).is_some() {
            tx.rollback().await?;
            return Err(ShowOnceError::LimitReached);
        }
        let busy: Option<(String,)> = sqlx::query_as(
            "SELECT id FROM skill_recordings WHERE stage = ? AND state = 'recording' AND id != ? LIMIT 1",
        )
        .bind(&rec.stage)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        if busy.is_some() {
            tx.rollback().await?;
            return Err(ShowOnceError::StageBusy);
        }
        sqlx::query(
            "UPDATE skill_recordings SET state = 'recording', segment_started_at = ?, ended_at = NULL,
             stop_reason = NULL, write_error = NULL, steps_expire_at = NULL WHERE id = ?",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_recording(id).await?.ok_or(ShowOnceError::NotFound)
    }

    /// Adds one recorded action (not a note) to a live recording, enforcing both limits: an action
    /// past 200 steps or 30 minutes stops the recording instead of being stored.
    pub async fn add_step(&self, id: &str, step: &NewStep, now: i64) -> Result<Added, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let rec: Recording = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ShowOnceError::NotFound)?;
        if rec.state != "recording" {
            tx.rollback().await?;
            return Err(ShowOnceError::WrongState(rec.state));
        }
        if let Some(reason) = rec.limit_reached(now) {
            stop_in(&mut tx, id, reason, now).await?;
            tx.commit().await?;
            let recording = self.get_recording(id).await?.ok_or(ShowOnceError::NotFound)?;
            return Ok(Added::Stopped { recording: Box::new(recording), reason: reason.into() });
        }
        let step_id = insert_step(&mut tx, id, step, None, now).await?;
        let mut origins = rec.origin_list();
        if let Some(o) = step.origin.as_ref().filter(|o| !origins.contains(o)) {
            origins.push(o.clone());
        }
        let after = Recording { step_count: rec.step_count + 1, ..rec.clone() };
        let near = rec.near_sent == 0 && after.near_at(now);
        sqlx::query("UPDATE skill_recordings SET step_count = step_count + 1, origins = ?, near_sent = MAX(near_sent, ?) WHERE id = ?")
            .bind(serde_json::to_string(&origins).unwrap_or_else(|_| "[]".into()))
            .bind(near as i64)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let row = get_step_in(&mut tx, step_id).await?;
        tx.commit().await?;
        Ok(Added::Step { step: row, near })
    }

    /// Marks the time-near warning sent (the 28:00 one, noticed by a timer rather than a step).
    /// True when this call is the one that marked it.
    pub async fn mark_near_sent(&self, id: &str) -> Result<bool, DbError> {
        let res = sqlx::query("UPDATE skill_recordings SET near_sent = 1 WHERE id = ? AND near_sent = 0")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    /// A note at the end, or right after step `after`. Allowed while recording or in review.
    pub async fn add_note(&self, id: &str, text: &str, after: Option<i64>, now: i64) -> Result<StepRow, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let rec: Recording = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ShowOnceError::NotFound)?;
        if !matches!(rec.state.as_str(), "recording" | "stopped") {
            tx.rollback().await?;
            return Err(ShowOnceError::WrongState(rec.state));
        }
        if rec.note_count >= MAX_NOTES {
            tx.rollback().await?;
            return Err(ShowOnceError::TooManyNotes);
        }
        let ord = match after {
            None => None,
            Some(after) => {
                let at: Option<(f64,)> = sqlx::query_as(
                    "SELECT ord FROM skill_recording_steps WHERE id = ? AND recording_id = ? AND (deleted_at IS NULL OR undo_until > ?)",
                )
                .bind(after)
                .bind(id)
                .bind(now)
                .fetch_optional(&mut *tx)
                .await?;
                let Some((at,)) = at else {
                    tx.rollback().await?;
                    return Err(ShowOnceError::NotFound);
                };
                let next: Option<(f64,)> = sqlx::query_as(
                    "SELECT ord FROM skill_recording_steps WHERE recording_id = ? AND ord > ? ORDER BY ord LIMIT 1",
                )
                .bind(id)
                .bind(at)
                .fetch_optional(&mut *tx)
                .await?;
                Some(next.map(|(n,)| (at + n) / 2.0).unwrap_or(at + 1.0))
            }
        };
        let note = NewStep { kind: "note".into(), value: Some(text.to_string()), ..Default::default() };
        let step_id = insert_step(&mut tx, id, &note, ord, now).await?;
        sqlx::query("UPDATE skill_recordings SET note_count = note_count + 1 WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let row = get_step_in(&mut tx, step_id).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// The listed steps in order: undeleted ones and deleted ones still inside their Undo.
    pub async fn list_steps(&self, id: &str, now: i64) -> Result<Vec<StepRow>, DbError> {
        Ok(sqlx::query_as::<_, StepRow>(
            "SELECT * FROM skill_recording_steps WHERE recording_id = ? AND (deleted_at IS NULL OR undo_until > ?) ORDER BY ord, id",
        )
        .bind(id)
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Review: deletes a step with a 10 s Undo. Only in `stopped`.
    pub async fn delete_step(&self, id: &str, step_id: i64, now: i64) -> Result<StepRow, ShowOnceError> {
        self.review_step(id, step_id, now, true).await
    }

    /// Review: brings back a step deleted less than 10 s ago.
    pub async fn restore_step(&self, id: &str, step_id: i64, now: i64) -> Result<StepRow, ShowOnceError> {
        self.review_step(id, step_id, now, false).await
    }

    async fn review_step(&self, id: &str, step_id: i64, now: i64, delete: bool) -> Result<StepRow, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let rec: Recording = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ShowOnceError::NotFound)?;
        if rec.state != "stopped" {
            tx.rollback().await?;
            return Err(ShowOnceError::WrongState(rec.state));
        }
        let step: Option<StepRow> =
            sqlx::query_as("SELECT * FROM skill_recording_steps WHERE id = ? AND recording_id = ?")
                .bind(step_id)
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(step) = step.filter(|s| s.deleted_at.is_none() || s.undo_until.is_some_and(|u| u > now) || !delete)
        else {
            tx.rollback().await?;
            return Err(ShowOnceError::NotFound);
        };
        if delete {
            if step.deleted_at.is_none() {
                sqlx::query("UPDATE skill_recording_steps SET deleted_at = ?, undo_until = ? WHERE id = ?")
                    .bind(now)
                    .bind(now + STEP_UNDO_SECS)
                    .bind(step_id)
                    .execute(&mut *tx)
                    .await?;
            }
        } else {
            match step.undo_until {
                None => {}
                Some(until) if until > now => {
                    sqlx::query("UPDATE skill_recording_steps SET deleted_at = NULL, undo_until = NULL WHERE id = ?")
                        .bind(step_id)
                        .execute(&mut *tx)
                        .await?;
                }
                Some(_) => {
                    tx.rollback().await?;
                    return Err(ShowOnceError::UndoOver);
                }
            }
        }
        let row = get_step_in(&mut tx, step_id).await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Discard: the recording and every step, now. Not once a skill was written from it.
    pub async fn discard_recording(&self, id: &str) -> Result<Recording, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let rec: Recording = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ShowOnceError::NotFound)?;
        if !matches!(rec.state.as_str(), "recording" | "stopped") {
            tx.rollback().await?;
            return Err(ShowOnceError::WrongState(rec.state));
        }
        delete_steps_in(&mut tx, id).await?;
        sqlx::query("DELETE FROM skill_recordings WHERE id = ?").bind(id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(rec)
    }

    /// Write pressed: `stopped` → `writing`, when an undeleted action step is left.
    pub async fn begin_writing(&self, id: &str) -> Result<Recording, ShowOnceError> {
        let mut tx = self.begin_write().await?;
        let rec: Recording = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(ShowOnceError::NotFound)?;
        if rec.state != "stopped" {
            tx.rollback().await?;
            return Err(ShowOnceError::WrongState(rec.state));
        }
        let (left,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM skill_recording_steps WHERE recording_id = ? AND kind != 'note' AND deleted_at IS NULL",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        if left == 0 || rec.steps_deleted_at.is_some() {
            tx.rollback().await?;
            return Err(ShowOnceError::NoSteps);
        }
        sqlx::query("UPDATE skill_recordings SET state = 'writing', write_error = NULL WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_recording(id).await?.ok_or(ShowOnceError::NotFound)
    }

    /// The draft turn failed: back to review with a plain reason. The steps' deadline stays
    /// `ended_at` + 7 days (Q-S11).
    pub async fn write_failed(&self, id: &str, why: &str) -> Result<(), DbError> {
        sqlx::query("UPDATE skill_recordings SET state = 'stopped', write_error = ? WHERE id = ? AND state = 'writing'")
            .bind(why)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// The draft is saved: `writing` → `written`, linked to its pending skill.
    pub async fn written(
        &self,
        id: &str,
        skill_id: &str,
        inputs_json: &str,
        step_count: i64,
        now: i64,
    ) -> Result<(), DbError> {
        sqlx::query(
            "UPDATE skill_recordings SET state = 'written', skill_id = ?, inputs = ?, shown_at = ?, shown_step_count = ?,
             write_error = NULL WHERE id = ?",
        )
        .bind(skill_id)
        .bind(inputs_json)
        .bind(now)
        .bind(step_count)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Approve of a shown draft: its steps are kept 7 days from now instead.
    pub async fn shown_skill_approved(&self, skill_id: &str, now: i64) -> Result<Option<Recording>, DbError> {
        sqlx::query(
            "UPDATE skill_recordings SET approved_at = ?, steps_expire_at = ? WHERE skill_id = ? AND steps_deleted_at IS NULL",
        )
        .bind(now)
        .bind(now + KEEP_STEPS_SECS)
        .bind(skill_id)
        .execute(&self.pool)
        .await?;
        self.recording_for_skill(skill_id).await
    }

    /// Reject, or Delete steps now: the steps go at once; the skill keeps its `shown` facts.
    pub async fn delete_shown_steps(&self, skill_id: &str, now: i64) -> Result<Option<Recording>, DbError> {
        let Some(rec) = self.recording_for_skill(skill_id).await? else {
            return Ok(None);
        };
        let mut tx = self.begin_write().await?;
        delete_steps_in(&mut tx, &rec.id).await?;
        sqlx::query("UPDATE skill_recordings SET steps_deleted_at = IFNULL(steps_deleted_at, ?) WHERE id = ?")
            .bind(now)
            .bind(&rec.id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        self.get_recording(&rec.id).await
    }

    /// Saves a shown draft as a pending skill of the bot. Unlike `propose_skill` it never rewrites
    /// another proposal or makes a revision: a name the bot already uses gets `-2`, `-3`...
    pub async fn insert_shown_skill(&self, kwant_id: &str, text: SkillText) -> Result<Skill, SkillError> {
        let space = self.space_of_kwant(kwant_id).await?.ok_or(SkillError::NotFound)?;
        let mut tx = self.begin_write().await?;
        let (count,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM skills WHERE status = 'pending' AND kwant_id = ?")
                .bind(kwant_id)
                .fetch_one(&mut *tx)
                .await?;
        if count >= MAX_PENDING_PER_KWANT {
            tx.rollback().await?;
            return Err(SkillError::TooManyPending);
        }
        let taken: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM skills WHERE space_id = ? AND (kwant_id = ? OR kwant_id IS NULL)
             AND status IN ('pending', 'enabled', 'disabled')",
        )
        .bind(space.as_str())
        .bind(kwant_id)
        .fetch_all(&mut *tx)
        .await?;
        let taken: Vec<String> = taken.into_iter().map(|(n,)| n).collect();
        let name = free_name(&text.name, &taken);
        let id = Uuid::new_v4().to_string();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO skills (id, kwant_id, name, description, body, status, source, proposed_by, replaces_id, created_at, updated_at, space_id)
             VALUES (?, ?, ?, ?, ?, 'pending', 'agent', ?, NULL, ?, ?, ?)",
        )
        .bind(&id)
        .bind(kwant_id)
        .bind(&name)
        .bind(&text.description)
        .bind(&text.body)
        .bind(kwant_id)
        .bind(now)
        .bind(now)
        .bind(space.as_str())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        self.get_skill(&id).await?.ok_or(SkillError::NotFound)
    }

    /// Stops every live recording (kwant restarted: nobody holds Take over any more) and puts a
    /// draft that was being written back into review.
    pub async fn stop_leftover_recordings(&self, now: i64) -> Result<Vec<String>, DbError> {
        let live: Vec<(String,)> = sqlx::query_as("SELECT id FROM skill_recordings WHERE state = 'recording'")
            .fetch_all(&self.pool)
            .await?;
        let mut changed = Vec::new();
        for (id,) in live {
            let mut tx = self.begin_write().await?;
            if stop_in(&mut tx, &id, "restart", now).await.map_err(|e| DbError::Migration(format!("{e:?}")))? {
                changed.push(id);
            }
            tx.commit().await?;
        }
        let writing: Vec<(String,)> = sqlx::query_as("SELECT id FROM skill_recordings WHERE state = 'writing'")
            .fetch_all(&self.pool)
            .await?;
        for (id,) in writing {
            self.write_failed(&id, "kwant restarted while the skill was being written. Press Write the skill again.").await?;
            changed.push(id);
        }
        Ok(changed)
    }

    /// Ends recordings past their 30 minutes, purges steps past their Undo or their deadline, and
    /// deletes recordings nobody can use any more. Idempotent; catches up after any time off.
    pub async fn sweep_recordings(&self, now: i64) -> Result<SweepCounts, DbError> {
        let mut out = SweepCounts::default();
        let live: Vec<Recording> = sqlx::query_as("SELECT * FROM skill_recordings WHERE state = 'recording'")
            .fetch_all(&self.pool)
            .await?;
        for rec in live.iter().filter(|r| r.elapsed_at(now) >= LIMIT_SECS) {
            let mut tx = self.begin_write().await?;
            if stop_in(&mut tx, &rec.id, "limit_time", now).await.map_err(|e| DbError::Migration(format!("{e:?}")))? {
                out.stopped += 1;
                out.changed.push(rec.id.clone());
            }
            tx.commit().await?;
        }
        // Steps whose Undo is over
        let res = blank_and_delete(
            &self.pool,
            "deleted_at IS NOT NULL AND undo_until <= ?1",
            now,
        )
        .await?;
        out.purged_steps += res;
        // Recordings past their deadline
        let due: Vec<Recording> = sqlx::query_as(
            "SELECT * FROM skill_recordings WHERE steps_expire_at IS NOT NULL AND steps_expire_at <= ? AND steps_deleted_at IS NULL
             AND state IN ('stopped', 'written')",
        )
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        for rec in due {
            let mut tx = self.begin_write().await?;
            out.purged_steps += delete_steps_in(&mut tx, &rec.id).await?;
            if rec.state == "stopped" {
                // Never written: nothing is left to review or to keep
                sqlx::query("DELETE FROM skill_recordings WHERE id = ?").bind(&rec.id).execute(&mut *tx).await?;
                out.expired_recordings += 1;
            } else {
                sqlx::query("UPDATE skill_recordings SET steps_deleted_at = ? WHERE id = ?")
                    .bind(now)
                    .bind(&rec.id)
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
            out.changed.push(rec.id);
        }
        // Belt and braces for the delete triggers: a recording whose bot or skill is gone
        let orphans: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM skill_recordings r WHERE NOT EXISTS (SELECT 1 FROM kwants k WHERE k.id = r.kwant_id)
             OR (r.skill_id IS NOT NULL AND NOT EXISTS (SELECT 1 FROM skills s WHERE s.id = r.skill_id))",
        )
        .fetch_all(&self.pool)
        .await?;
        for (id,) in orphans {
            let mut tx = self.begin_write().await?;
            out.purged_steps += delete_steps_in(&mut tx, &id).await?;
            sqlx::query("DELETE FROM skill_recordings WHERE id = ?").bind(&id).execute(&mut *tx).await?;
            tx.commit().await?;
            out.expired_recordings += 1;
        }
        let (stray,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM skill_recording_steps s WHERE NOT EXISTS (SELECT 1 FROM skill_recordings r WHERE r.id = s.recording_id)",
        )
        .fetch_one(&self.pool)
        .await?;
        if stray > 0 {
            out.purged_steps += blank_and_delete(
                &self.pool,
                "NOT EXISTS (SELECT 1 FROM skill_recordings r WHERE r.id = skill_recording_steps.recording_id) AND ?1 = ?1",
                now,
            )
            .await?;
        }
        Ok(out)
    }

    /// How many step rows a recording has in the database, deleted-in-Undo ones included (tests).
    pub async fn stored_step_rows(&self, id: &str) -> Result<i64, DbError> {
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM skill_recording_steps WHERE recording_id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await?;
        Ok(n)
    }
}

type Tx = sqlx::Transaction<'static, sqlx::Sqlite>;

/// Ends a live recording's current part inside `tx`; false when it wasn't live.
async fn stop_in(tx: &mut Tx, id: &str, reason: &str, now: i64) -> Result<bool, ShowOnceError> {
    let rec: Option<Recording> = sqlx::query_as("SELECT * FROM skill_recordings WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?;
    let Some(rec) = rec else { return Err(ShowOnceError::NotFound) };
    if rec.state != "recording" {
        return Ok(false);
    }
    let start = rec.segment_started_at.unwrap_or(now);
    let raw = rec.elapsed_secs + (now - start).max(0);
    // Past the 30 minutes, the recording ended at 30:00, whenever the sweep noticed
    let (elapsed, ended) = if raw >= LIMIT_SECS {
        (LIMIT_SECS, (start + (LIMIT_SECS - rec.elapsed_secs).max(0)).min(now))
    } else {
        (raw, now)
    };
    sqlx::query(
        "UPDATE skill_recordings SET state = 'stopped', stop_reason = ?, elapsed_secs = ?, ended_at = ?,
         segment_started_at = NULL, steps_expire_at = ? WHERE id = ?",
    )
    .bind(reason)
    .bind(elapsed)
    .bind(ended)
    .bind(ended + KEEP_STEPS_SECS)
    .bind(id)
    .execute(&mut **tx)
    .await?;
    Ok(true)
}

async fn insert_step(tx: &mut Tx, id: &str, step: &NewStep, ord: Option<f64>, now: i64) -> Result<i64, sqlx::Error> {
    let ord = match ord {
        Some(o) => o,
        None => {
            let (max,): (Option<f64>,) =
                sqlx::query_as("SELECT MAX(ord) FROM skill_recording_steps WHERE recording_id = ?")
                    .bind(id)
                    .fetch_one(&mut **tx)
                    .await?;
            max.map(|m| m.floor() + 1.0).unwrap_or(1.0)
        }
    };
    // A masked step never keeps a value, whatever the caller passed
    let value = if step.masked.is_some() { None } else { step.value.clone() };
    let res = sqlx::query(
        "INSERT INTO skill_recording_steps (recording_id, ord, kind, origin, path, role, name, label, value, masked, at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(ord)
    .bind(&step.kind)
    .bind(&step.origin)
    .bind(&step.path)
    .bind(&step.role)
    .bind(&step.name)
    .bind(&step.label)
    .bind(value)
    .bind(&step.masked)
    .bind(now)
    .execute(&mut **tx)
    .await?;
    Ok(res.last_insert_rowid())
}

async fn get_step_in(tx: &mut Tx, step_id: i64) -> Result<StepRow, sqlx::Error> {
    sqlx::query_as("SELECT * FROM skill_recording_steps WHERE id = ?")
        .bind(step_id)
        .fetch_one(&mut **tx)
        .await
}

/// Deletes a recording's step rows, blanking their text first so it doesn't linger in free pages.
async fn delete_steps_in(tx: &mut Tx, id: &str) -> Result<u64, sqlx::Error> {
    sqlx::query(
        "UPDATE skill_recording_steps SET value = NULL, name = NULL, label = NULL, path = NULL, origin = NULL, role = NULL
         WHERE recording_id = ?",
    )
    .bind(id)
    .execute(&mut **tx)
    .await?;
    let res = sqlx::query("DELETE FROM skill_recording_steps WHERE recording_id = ?").bind(id).execute(&mut **tx).await?;
    Ok(res.rows_affected())
}

/// [`delete_steps_in`] for the rows matching `filter` (which may use `?1` = `now`).
async fn blank_and_delete(pool: &sqlx::SqlitePool, filter: &str, now: i64) -> Result<u64, sqlx::Error> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    sqlx::query(&format!(
        "UPDATE skill_recording_steps SET value = NULL, name = NULL, label = NULL, path = NULL, origin = NULL, role = NULL
         WHERE {filter}"
    ))
    .bind(now)
    .execute(&mut *tx)
    .await?;
    let res = sqlx::query(&format!("DELETE FROM skill_recording_steps WHERE {filter}")).bind(now).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(res.rows_affected())
}

/// RFC 3339 for a stored time.
pub fn rfc3339(secs: i64) -> String {
    Utc.timestamp_opt(secs, 0).single().map(|t| t.to_rfc3339()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_gets_the_first_free_number() {
        let taken = vec!["export-orders".to_string(), "export-orders-2".to_string()];
        assert_eq!(free_name("export-orders", &taken), "export-orders-3");
        assert_eq!(free_name("fresh", &taken), "fresh");
        let long = "a".repeat(MAX_SKILL_NAME_CHARS);
        let named = free_name(&long, std::slice::from_ref(&long));
        assert_eq!(named.chars().count(), MAX_SKILL_NAME_CHARS);
        assert!(named.ends_with("-2"));
    }

    fn rec(state: &str, elapsed: i64, seg: Option<i64>, steps: i64) -> Recording {
        Recording {
            id: "rec_1".into(),
            kwant_id: "k".into(),
            space_id: "s".into(),
            stage: "k".into(),
            state: state.into(),
            stop_reason: None,
            started_at: 0,
            segment_started_at: seg,
            elapsed_secs: elapsed,
            ended_at: None,
            step_count: steps,
            note_count: 0,
            near_sent: 0,
            origins: "[]".into(),
            write_error: None,
            skill_id: None,
            inputs: None,
            shown_at: None,
            shown_step_count: None,
            approved_at: None,
            steps_expire_at: None,
            steps_deleted_at: None,
        }
    }

    #[test]
    fn limits_and_near_follow_the_spec_numbers() {
        assert_eq!(rec("recording", 0, Some(100), 199).limit_reached(200), None);
        assert_eq!(rec("recording", 0, Some(100), 200).limit_reached(200), Some("limit_steps"));
        assert_eq!(rec("recording", 600, Some(0), 3).limit_reached(1199), None);
        assert_eq!(rec("recording", 600, Some(0), 3).limit_reached(1200), Some("limit_time"));
        // Review time between parts doesn't count: a stopped recording's clock doesn't run
        assert_eq!(rec("stopped", 600, None, 3).elapsed_at(1_000_000), 600);
        assert!(!rec("recording", 0, Some(0), 191).near_at(10));
        assert!(rec("recording", 0, Some(0), 192).near_at(10), "8 steps left");
        assert!(!rec("recording", 0, Some(0), 1).near_at(NEAR_SECS - 1));
        assert!(rec("recording", 0, Some(0), 1).near_at(NEAR_SECS), "28:00 used");
    }
}
